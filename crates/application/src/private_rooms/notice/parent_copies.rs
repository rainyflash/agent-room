//! 子房间里父房间须知的副本（`specs/redesign/brief-and-pins.md` 的“子房间里父房间的须知也算”，4N-3b）：
//! 进子房间的 Agent 不一定在父房间的聊天房间里，读不到父房间的须知，所以控制面以应用服务在每间子房间
//! 里写一份（`room.parent_notice.v1`）。
//!
//! 父房间改了须知、新建了子房间时排进 `private_room_parent_notice_sync`（归档了的子房间也排：它恢复以后
//! 里面的 Agent 读到的该是父房间此刻的那一版）。改须知的那个请求结束前先写一批它下面的（没写成不算请求
//! 失败），后台每 30 秒一轮补上剩下的。每件事只说“这间子房间的副本要照父房间写”，不带写什么：做的时候
//! 读父房间此刻在聊天服务器上的须知，和副本不一样才写，做几遍结果都一样。

use agent_room_domain::{ids::RoomCatalogId, room_notices::RoomNoticeText, time::UtcMillis};

use crate::{
    moderation::{homeserver_unavailable, matrix_failure_code},
    ports::{
        MatrixFailure, MatrixRoomId, ParentNoticeCopy, ParentNoticeSyncTask, PortFuture,
        PrivateRoomSnapshot, RoomNoticeWrite, SubRoomSyncScope,
    },
};

use super::{NoticeResult, PrivateRoomNoticeService, repository};

/// 后台一轮最多写这么多间。
const TASKS_PER_ROUND: u16 = 50;
/// 改须知的那个请求结束前最多写这么多间：再多的留给后台，别让请求等太久。
const TASKS_PER_REQUEST: u16 = 20;
/// 领到以后多久别处不能再领。一批写完只要几秒；写到一半进程断了，过了这个时间别处接着写。
const LEASE_MILLIS: i64 = 2 * 60 * 1000;
/// 没写成时第一次隔多久再试，之后每次翻倍。
const RETRY_INITIAL_MILLIS: i64 = 30 * 1000;
/// 最长隔多久再试。不设放弃：放弃了子房间里的 Agent 就一直读着旧的那一版。
const RETRY_MAXIMUM_MILLIS: i64 = 15 * 60 * 1000;

/// 子房间里的副本跟着父房间的须知写，由控制面的定时任务调用。
pub trait ParentNoticeSyncUseCases: Send + Sync {
    /// 做一轮：把排着的领一批出来写。没写成的留在队里，往后退避再试。
    fn sync_parent_notices(&self) -> PortFuture<'_, NoticeResult<ParentNoticeSyncOutcome>>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParentNoticeSyncOutcome {
    /// 做完了几间（写了的，和本来就一样、不用写的）。
    pub done: usize,
    /// 没写成、往后退避再试的。
    pub retrying: Vec<ParentNoticeSyncRetry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParentNoticeSyncRetry {
    /// 聊天服务器那边的和治理动作失败时记的是一套，比如 `matrix.unavailable`。
    pub failure_code: &'static str,
    /// 连这一次，一共试过几次。
    pub attempt: u32,
}

/// 一间这次没写成，卡在哪。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trouble {
    Matrix(MatrixFailure),
    /// 子房间、父房间没读成。
    Storage,
    /// 存着的房间号不是合法的 Matrix 房间号。
    Corrupt,
}

impl Trouble {
    fn code(self) -> &'static str {
        match self {
            Self::Matrix(failure) => matrix_failure_code(failure),
            Self::Storage => "storage.unavailable",
            Self::Corrupt => "storage.corrupt",
        }
    }

    /// 整个聊天服务器这会儿用不了：换一间也一样写不成。
    const fn homeserver_down(self) -> bool {
        match self {
            Self::Matrix(failure) => homeserver_unavailable(failure),
            Self::Storage | Self::Corrupt => false,
        }
    }
}

impl ParentNoticeSyncUseCases for PrivateRoomNoticeService {
    fn sync_parent_notices(&self) -> PortFuture<'_, NoticeResult<ParentNoticeSyncOutcome>> {
        Box::pin(async move {
            let mut outcome = ParentNoticeSyncOutcome::default();
            self.run_copies(
                SubRoomSyncScope::Everywhere,
                TASKS_PER_ROUND,
                &mut outcome,
                "private_room.sync_parent_notices",
            )
            .await?;
            Ok(outcome)
        })
    }
}

impl PrivateRoomNoticeService {
    /// 父房间的须知写好了：它下面每间子房间排上，当场先写一批。没写成不算这个请求失败：排上的事后台
    /// 接着做；连排都没排上的，等下一次改须知。
    pub(super) async fn copy_to_sub_rooms(&self, parent: RoomCatalogId, now: UtcMillis) {
        let Ok(queued) = self.copy_queue.queue_parent_notice_sync(parent, now).await else {
            return;
        };
        if queued == 0 {
            return;
        }
        let mut outcome = ParentNoticeSyncOutcome::default();
        let _ = self
            .run_copies(
                SubRoomSyncScope::Under(parent),
                TASKS_PER_REQUEST,
                &mut outcome,
                "private_room.copy_notice_to_sub_rooms",
            )
            .await;
    }

    /// 领一批，一间间写：写好的删掉，没写成的记下原因往后排。
    async fn run_copies(
        &self,
        scope: SubRoomSyncScope,
        limit: u16,
        outcome: &mut ParentNoticeSyncOutcome,
        operation: &'static str,
    ) -> NoticeResult<()> {
        let now = self.clock.now();
        let tasks = self
            .copy_queue
            .claim_parent_notice_sync(scope, limit, now, later(now, LEASE_MILLIS))
            .await
            .map_err(|error| repository(operation, &error))?;
        // 整个聊天服务器用不了时记下原因：这一批剩下的不用再试，照同一个原因往后排。
        let mut homeserver_down = None;
        for task in tasks {
            let trouble = match homeserver_down {
                Some(trouble) => Some(trouble),
                None => self.copy_notice(task).await.err(),
            };
            let Some(trouble) = trouble else {
                self.copy_queue
                    .complete_parent_notice_sync(task)
                    .await
                    .map_err(|error| repository(operation, &error))?;
                outcome.done += 1;
                continue;
            };
            if trouble.homeserver_down() {
                homeserver_down = Some(trouble);
            }
            let retry_at = later(self.clock.now(), retry_delay_millis(task.attempt));
            self.copy_queue
                .retry_parent_notice_sync(task, retry_at, trouble.code())
                .await
                .map_err(|error| repository(operation, &error))?;
            outcome.retrying.push(ParentNoticeSyncRetry {
                failure_code: trouble.code(),
                attempt: task.attempt,
            });
        }
        Ok(())
    }

    /// 读父房间此刻的须知和子房间里的副本，不一样就照父房间写一份。
    async fn copy_notice(&self, task: ParentNoticeSyncTask) -> Result<(), Trouble> {
        // 子房间不在了、父房间不在了：没什么可写的。
        let Some(sub_room) = self.room_now(task.catalog_id).await? else {
            return Ok(());
        };
        let Some(parent_id) = sub_room.parent_catalog_id() else {
            return Ok(());
        };
        let Some(parent) = self.room_now(parent_id).await? else {
            return Ok(());
        };
        let sub_room = matrix_room(&sub_room)?;
        let parent = matrix_room(&parent)?;
        let source = self
            .copies
            .room_notice(&parent)
            .await
            .map_err(Trouble::Matrix)?;
        let current = self
            .copies
            .parent_notice(&sub_room)
            .await
            .map_err(Trouble::Matrix)?;
        let Some(wanted) = wanted_copy(source, current.as_ref(), &parent) else {
            return Ok(());
        };
        self.copies
            .set_parent_notice(&sub_room, &wanted)
            .await
            .map_err(Trouble::Matrix)
    }

    async fn room_now(
        &self,
        catalog_id: RoomCatalogId,
    ) -> Result<Option<PrivateRoomSnapshot>, Trouble> {
        self.rooms
            .find_by_catalog(catalog_id)
            .await
            .map_err(|_| Trouble::Storage)
    }
}

fn matrix_room(snapshot: &PrivateRoomSnapshot) -> Result<MatrixRoomId, Trouble> {
    MatrixRoomId::new(snapshot.instance().matrix_room_id().as_str().to_owned())
        .map_err(|_| Trouble::Corrupt)
}

/// 照父房间此刻的须知，子房间里的副本该写成什么；和 `current` 一样、不用写时为空。
///
/// 父房间有须知就原样抄过来。没有（撤掉了、从来没写过）时：副本上还写着的，写一份正文空的盖掉，改的人
/// 和时间用父房间撤掉的那一版的，读不到那一版就留着副本上的；副本本来就没有正文的不动。
fn wanted_copy(
    source: Option<RoomNoticeWrite>,
    current: Option<&ParentNoticeCopy>,
    parent_room: &MatrixRoomId,
) -> Option<ParentNoticeCopy> {
    let notice = match source {
        Some(notice) if !notice.text.is_removal() => notice,
        removed => {
            let shown = current.filter(|copy| !copy.notice.text.is_removal())?;
            let last = removed.unwrap_or_else(|| shown.notice.clone());
            RoomNoticeWrite {
                text: RoomNoticeText::parse("").ok()?,
                ..last
            }
        }
    };
    let wanted = ParentNoticeCopy {
        notice,
        parent_room_id: parent_room.clone(),
    };
    (current != Some(&wanted)).then_some(wanted)
}

fn later(at: UtcMillis, millis: i64) -> UtcMillis {
    UtcMillis::new(at.value().saturating_add(millis)).unwrap_or(at)
}

/// 第 `attempt` 次没写成以后隔多久再试：30 秒起每次翻倍，最长 15 分钟。
fn retry_delay_millis(attempt: u32) -> i64 {
    let doublings = attempt.saturating_sub(1).min(20);
    RETRY_INITIAL_MILLIS
        .saturating_mul(1_i64 << doublings)
        .min(RETRY_MAXIMUM_MILLIS)
}

#[cfg(test)]
mod tests {
    use agent_room_domain::{room_notices::RoomNoticeText, time::UtcMillis};

    use super::{RETRY_MAXIMUM_MILLIS, retry_delay_millis, wanted_copy};
    use crate::ports::{MatrixRoomId, MatrixUserId, ParentNoticeCopy, RoomNoticeWrite};

    fn notice(text: &str, by: &str, at: i64) -> RoomNoticeWrite {
        RoomNoticeWrite {
            text: RoomNoticeText::parse(text).unwrap(),
            updated_by: MatrixUserId::new(format!("@{by}:matrix.test")).unwrap(),
            updated_at: UtcMillis::new(at).unwrap(),
        }
    }

    fn parent() -> MatrixRoomId {
        MatrixRoomId::new("!parent:matrix.test").unwrap()
    }

    fn copy(notice: RoomNoticeWrite) -> ParentNoticeCopy {
        ParentNoticeCopy {
            notice,
            parent_room_id: parent(),
        }
    }

    #[test]
    fn 父房间有须知_副本不一样就照抄_一样就不写() {
        let source = notice("先说要做什么。", "owner", 1_000);
        assert_eq!(
            wanted_copy(Some(source.clone()), None, &parent()),
            Some(copy(source.clone()))
        );
        let stale = copy(notice("旧的", "owner", 500));
        assert_eq!(
            wanted_copy(Some(source.clone()), Some(&stale), &parent()),
            Some(copy(source.clone()))
        );
        assert_eq!(
            wanted_copy(Some(source.clone()), Some(&copy(source.clone())), &parent()),
            None
        );
        // 父房间换了一间（房间号对不上）也算不一样。
        let elsewhere = ParentNoticeCopy {
            parent_room_id: MatrixRoomId::new("!elsewhere:matrix.test").unwrap(),
            ..copy(source.clone())
        };
        assert_eq!(
            wanted_copy(Some(source.clone()), Some(&elsewhere), &parent()),
            Some(copy(source))
        );
    }

    #[test]
    fn 父房间撤掉了须知_副本上还写着的写一份空的_都没有就不动() {
        let shown = copy(notice("先说要做什么。", "owner", 1_000));
        let removal = notice("", "admin", 2_000);
        assert_eq!(
            wanted_copy(Some(removal.clone()), Some(&shown), &parent()),
            Some(copy(removal.clone()))
        );
        // 父房间那条读不到：正文清空，改的人和时间留着副本上的。
        assert_eq!(
            wanted_copy(None, Some(&shown), &parent()),
            Some(copy(notice("", "owner", 1_000)))
        );
        assert_eq!(wanted_copy(Some(removal.clone()), None, &parent()), None);
        assert_eq!(wanted_copy(None, None, &parent()), None);
        assert_eq!(
            wanted_copy(Some(removal.clone()), Some(&copy(removal)), &parent()),
            None
        );
        assert_eq!(
            wanted_copy(None, Some(&copy(notice("", "owner", 1_000))), &parent()),
            None
        );
    }

    #[test]
    fn 没写成以后三十秒起每次翻倍_最长十五分钟() {
        assert_eq!(retry_delay_millis(1), 30_000);
        assert_eq!(retry_delay_millis(2), 60_000);
        assert_eq!(retry_delay_millis(6), RETRY_MAXIMUM_MILLIS);
        assert_eq!(retry_delay_millis(u32::MAX), RETRY_MAXIMUM_MILLIS);
        assert_eq!(retry_delay_millis(0), 30_000);
    }
}
