//! 子房间第 2c-2 步（`specs/redesign/sub-rooms.md` 的“对账”）：父房间变了以后，聊天服务器上跟着做。
//!
//! 库里那一半在存父房间的同一个事务里已经做完（存储的 `sub_room_follow`），聊天服务器上还差的排在
//! 任务表里。这里把排着的事一件件领出来做：改父房间的那个请求结束前先做一遍它下面的（没做成不算
//! 请求失败），后台每 30 秒一轮补上剩下的，顺带把照到的版本落后的子房间重算一遍。
//!
//! 每件事只说“这个人在这间子房间里变了”或者“这间子房间归档了”，不说变成了什么：做的时候读库里
//! 现在的样子，照它把聊天服务器摆成一样，做几遍结果都一样。

use agent_room_domain::{
    ids::PrincipalId,
    private_rooms::{PrivateRoom, PrivateRoomLifecycleStatus},
    time::UtcMillis,
};

use crate::{
    moderation::{homeserver_unavailable, matrix_failure_code},
    persistence::RepositoryError,
    ports::{
        MatrixFailure, MatrixRoomId, MatrixUserId, PortFuture, PrivateMatrixMembership,
        PrivateRoomSnapshot, SubRoomSyncScope, SubRoomSyncTask,
    },
};

use super::{
    super::{PrivateRoomFailureStage, PrivateRoomResult, failure::repository},
    MemberAgentChange, MemberAgentTrouble, PrivateRoomService, matrix_room_id,
};

/// 后台一轮最多重算这么多间照到的版本落后的子房间，再多的留给下一轮。
const STALE_PER_ROUND: u16 = 50;
/// 后台一轮最多领这么多件事。
const TASKS_PER_ROUND: u16 = 50;
/// 改父房间的那个请求结束前最多做这么多件：再多的留给后台，别让请求等太久。
const TASKS_PER_REQUEST: u16 = 20;
/// 领到以后多久别处不能再领。一批做完只要几秒；做到一半进程断了，过了这个时间别处接着做。
const LEASE_MILLIS: i64 = 2 * 60 * 1000;
/// 没做成时第一次隔多久再试，之后每次翻倍。
const RETRY_INITIAL_MILLIS: i64 = 30 * 1000;
/// 最长隔多久再试。不设放弃：放弃了被移出的人就一直留在子房间里。
const RETRY_MAXIMUM_MILLIS: i64 = 15 * 60 * 1000;

/// 子房间在聊天服务器上跟着父房间变，由控制面的定时任务调用。
pub trait SubRoomSyncUseCases: Send + Sync {
    /// 做一轮：先把照到的版本落后的子房间照父房间重算（回滚到旧版那段时间留下的），再把排着的事
    /// 领一批出来做。没做成的留在队里，往后退避再试。
    fn sync_sub_rooms(&self) -> PortFuture<'_, PrivateRoomResult<SubRoomSyncOutcome>>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubRoomSyncOutcome {
    /// 这一轮重算了几间照到的版本落后的子房间。
    pub followed: u32,
    /// 聊天服务器上做完了几件。
    pub done: usize,
    /// 没做成、往后退避再试的。
    pub retrying: Vec<SubRoomSyncRetry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubRoomSyncRetry {
    pub task_id: i64,
    /// 聊天服务器那边的和治理动作失败时记的是一套，比如 `matrix.unavailable`。
    pub failure_code: &'static str,
    /// 连这一次，一共试过几次。
    pub attempt: u32,
}

/// 一件事这次没做成，卡在哪。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trouble {
    Matrix(MatrixFailure),
    /// 这个人的聊天账户、他名下的 Agent 没查成。
    Directory,
    /// 子房间没读成。
    Storage,
    /// 存着的房间号不是合法的 Matrix 房间号。
    Corrupt,
}

impl Trouble {
    fn code(self) -> &'static str {
        match self {
            Self::Matrix(failure) => matrix_failure_code(failure),
            Self::Directory => "directory.unavailable",
            Self::Storage => "storage.unavailable",
            Self::Corrupt => "storage.corrupt",
        }
    }

    /// 整个聊天服务器这会儿用不了：换一件事也一样做不成。
    const fn homeserver_down(self) -> bool {
        match self {
            Self::Matrix(failure) => homeserver_unavailable(failure),
            Self::Directory | Self::Storage | Self::Corrupt => false,
        }
    }
}

impl From<MemberAgentTrouble> for Trouble {
    fn from(trouble: MemberAgentTrouble) -> Self {
        match trouble {
            MemberAgentTrouble::Directory(_) => Self::Directory,
            MemberAgentTrouble::Matrix(failure) => Self::Matrix(failure),
        }
    }
}

/// 一个人在一间子房间里，聊天服务器上该摆成什么样。照库里现在的记录定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemberCourse {
    /// 还在里面，房间在用：发言级别照他的权限设，没在聊天服务器的这间房里就邀请，他的 Agent 跟着
    /// 能说或者不能说。
    Inside { speaks: bool },
    /// 还在里面，房间归档了：只补邀请。发言级别不能动：设一次就把整间房的门槛写回“在用”的那
    /// 一套，等于把归档的房间解开。
    InsideArchived,
    /// 不在里面了：踢出去，他的 Agent 一起。归档了的子房间里也一样，不然他还读得到里面的消息。
    Outside,
}

impl MemberCourse {
    fn of(room: &PrivateRoom, principal: PrincipalId) -> Self {
        let inside = room
            .member(principal)
            .filter(|member| member.status().carries_permissions());
        match (inside, room.status()) {
            (None, _) => Self::Outside,
            (Some(_), PrivateRoomLifecycleStatus::Archived) => Self::InsideArchived,
            (Some(member), PrivateRoomLifecycleStatus::Active) => Self::Inside {
                speaks: member.permissions().speak(),
            },
        }
    }
}

impl SubRoomSyncUseCases for PrivateRoomService {
    fn sync_sub_rooms(&self) -> PortFuture<'_, PrivateRoomResult<SubRoomSyncOutcome>> {
        Box::pin(self.sync_sub_rooms_internal())
    }
}

impl PrivateRoomService {
    async fn sync_sub_rooms_internal(&self) -> PrivateRoomResult<SubRoomSyncOutcome> {
        const OPERATION: &str = "sub_room.sync";
        let mut outcome = SubRoomSyncOutcome::default();
        // 重算没做完也接着做排着的事：两样互不依赖，哪个出错都留到最后再报。
        let followed = self
            .sub_rooms
            .follow_stale_sub_rooms(STALE_PER_ROUND, self.clock.now())
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error));
        let ran = self
            .run_sub_room_sync(
                SubRoomSyncScope::Everywhere,
                TASKS_PER_ROUND,
                &mut outcome,
                OPERATION,
            )
            .await;
        outcome.followed = followed?;
        ran?;
        Ok(outcome)
    }

    /// 父房间存好以后，先把它下面的子房间在聊天服务器上跟着做一遍，被移出的人当场就出去。没做成不算
    /// 这个请求失败：事还排着，后台接着做。
    pub(super) async fn sync_sub_rooms_under(&self, parent: &PrivateRoomSnapshot) {
        // 子房间下面没有子房间。
        if parent.parent_catalog_id().is_some() {
            return;
        }
        let mut outcome = SubRoomSyncOutcome::default();
        let _ = self
            .run_sub_room_sync(
                SubRoomSyncScope::Under(parent.catalog().id()),
                TASKS_PER_REQUEST,
                &mut outcome,
                "sub_room.sync_under",
            )
            .await;
    }

    /// 领一批排着的事，一件件做：做成的删掉，没做成的记下原因往后排。
    async fn run_sub_room_sync(
        &self,
        scope: SubRoomSyncScope,
        limit: u16,
        outcome: &mut SubRoomSyncOutcome,
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        let persistence = |error: RepositoryError| {
            repository(operation, PrivateRoomFailureStage::Persistence, &error)
        };
        let now = self.clock.now();
        let tasks = self
            .sub_rooms
            .claim_sub_room_sync(scope, limit, now, later(now, LEASE_MILLIS))
            .await
            .map_err(persistence)?;
        // 整个聊天服务器用不了时记下原因：这一批剩下的不用再试，照同一个原因往后排。
        let mut homeserver_down = None;
        for task in tasks {
            let trouble = match homeserver_down {
                Some(trouble) => Some(trouble),
                None => self.run_sync_task(task).await.err(),
            };
            let Some(trouble) = trouble else {
                self.sub_rooms
                    .complete_sub_room_sync(task)
                    .await
                    .map_err(persistence)?;
                outcome.done += 1;
                continue;
            };
            if trouble.homeserver_down() {
                homeserver_down = Some(trouble);
            }
            let retry_at = later(self.clock.now(), retry_delay_millis(task.attempt));
            self.sub_rooms
                .retry_sub_room_sync(task, retry_at, trouble.code())
                .await
                .map_err(persistence)?;
            outcome.retrying.push(SubRoomSyncRetry {
                task_id: task.id,
                failure_code: trouble.code(),
                attempt: task.attempt,
            });
        }
        Ok(())
    }

    async fn run_sync_task(&self, task: SubRoomSyncTask) -> Result<(), Trouble> {
        // 子房间不在了：没什么可做的。
        let Some(sub_room) = self.sub_room_now(&task).await? else {
            return Ok(());
        };
        let matrix_room =
            matrix_room_id(&sub_room, "sub_room.sync_task").map_err(|_| Trouble::Corrupt)?;
        match task.principal {
            Some(principal) => {
                self.sync_sub_room_member(&task, &sub_room, &matrix_room, principal)
                    .await
            }
            // 这间子房间自己的事只有一样：跟着父房间归档了，在聊天服务器上锁住。之后又恢复了的不动。
            None if sub_room.room().status() == PrivateRoomLifecycleStatus::Archived => self
                .matrix
                .archive(&matrix_room)
                .await
                .map_err(Trouble::Matrix),
            None => Ok(()),
        }
    }

    async fn sub_room_now(
        &self,
        task: &SubRoomSyncTask,
    ) -> Result<Option<PrivateRoomSnapshot>, Trouble> {
        self.store
            .find_by_catalog(task.catalog_id)
            .await
            .map_err(|_| Trouble::Storage)
    }

    /// 照库里现在的记录，把这个人在聊天服务器的这间子房间里摆成一样。
    async fn sync_sub_room_member(
        &self,
        task: &SubRoomSyncTask,
        sub_room: &PrivateRoomSnapshot,
        matrix_room: &MatrixRoomId,
        principal: PrincipalId,
    ) -> Result<(), Trouble> {
        // 替网络 Agent 记账的合成主体没有聊天账户：只动它名下的 Agent。
        let user = self
            .principals
            .matrix_user_id(principal)
            .await
            .map_err(|_| Trouble::Directory)?;
        let course = MemberCourse::of(sub_room.room(), principal);
        self.follow_member_course(course, matrix_room, principal, user.as_ref())
            .await?;
        // 动完再看一眼：做的当口他自己退出了这一间，或者这一间归档了，照新的样子再摆一遍。这两样
        // 不经过任务表，没有人会替它们再排一次。
        let Some(after) = self.sub_room_now(task).await? else {
            return Ok(());
        };
        let settled = MemberCourse::of(after.room(), principal);
        if settled != course {
            self.follow_member_course(settled, matrix_room, principal, user.as_ref())
                .await?;
        }
        // 刚才设发言级别把整间房的门槛写回了“在用”的那一套：这当口归档了的，再锁一次。
        if matches!(course, MemberCourse::Inside { .. })
            && after.room().status() == PrivateRoomLifecycleStatus::Archived
        {
            self.matrix
                .archive(matrix_room)
                .await
                .map_err(Trouble::Matrix)?;
        }
        Ok(())
    }

    async fn follow_member_course(
        &self,
        course: MemberCourse,
        matrix_room: &MatrixRoomId,
        principal: PrincipalId,
        user: Option<&MatrixUserId>,
    ) -> Result<(), Trouble> {
        match course {
            MemberCourse::Inside { speaks } => {
                if let Some(user) = user {
                    // 发言级别先设好再邀请，和他自己加入时一个顺序。
                    self.matrix
                        .set_speaking(matrix_room, user, speaks)
                        .await
                        .map_err(Trouble::Matrix)?;
                    self.invite_if_absent(matrix_room, user).await?;
                }
                self.member_agents_follow(matrix_room, principal, MemberAgentChange::Speak(speaks))
                    .await?;
            }
            MemberCourse::InsideArchived => {
                if let Some(user) = user {
                    self.invite_if_absent(matrix_room, user).await?;
                }
            }
            MemberCourse::Outside => {
                if let Some(user) = user {
                    self.matrix
                        .kick(matrix_room, user)
                        .await
                        .map_err(Trouble::Matrix)?;
                }
                self.member_agents_follow(matrix_room, principal, MemberAgentChange::Evict)
                    .await?;
            }
        }
        Ok(())
    }

    /// 没在聊天服务器的这间房里就邀请：换了房主以后新房主没进过的子房间就靠这一步，网页接着照
    /// “服务器记着已加入、聊天服务器上是受邀”替他加入。在这一间里封着的不邀请：聊天服务器不收，
    /// 这件事会一直排着做不完。
    async fn invite_if_absent(
        &self,
        matrix_room: &MatrixRoomId,
        user: &MatrixUserId,
    ) -> Result<(), Trouble> {
        let membership = self
            .matrix
            .membership(matrix_room, user)
            .await
            .map_err(Trouble::Matrix)?;
        if matches!(
            membership,
            Some(
                PrivateMatrixMembership::Invited
                    | PrivateMatrixMembership::Joined
                    | PrivateMatrixMembership::Banned
            )
        ) {
            return Ok(());
        }
        self.matrix
            .invite(matrix_room, user)
            .await
            .map_err(Trouble::Matrix)
    }
}

fn later(at: UtcMillis, millis: i64) -> UtcMillis {
    UtcMillis::new(at.value().saturating_add(millis)).unwrap_or(at)
}

/// 第 `attempt` 次没做成以后隔多久再试：30 秒起每次翻倍，最长 15 分钟。
fn retry_delay_millis(attempt: u32) -> i64 {
    let doublings = attempt.saturating_sub(1).min(20);
    RETRY_INITIAL_MILLIS
        .saturating_mul(1_i64 << doublings)
        .min(RETRY_MAXIMUM_MILLIS)
}

#[cfg(test)]
mod tests {
    use super::{RETRY_MAXIMUM_MILLIS, retry_delay_millis};

    #[test]
    fn 没做成以后三十秒起每次翻倍_最长十五分钟() {
        assert_eq!(retry_delay_millis(1), 30_000);
        assert_eq!(retry_delay_millis(2), 60_000);
        assert_eq!(retry_delay_millis(5), 480_000);
        assert_eq!(retry_delay_millis(6), RETRY_MAXIMUM_MILLIS);
        assert_eq!(retry_delay_millis(u32::MAX), RETRY_MAXIMUM_MILLIS);
        // 领到的那一次从 1 数起；万一是 0 也不出错。
        assert_eq!(retry_delay_millis(0), 30_000);
    }
}
