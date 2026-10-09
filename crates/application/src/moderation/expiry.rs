//! 治理动作到期自动解除。这是系统自己做的：没有登录的人，不走要权限、要最近认证的撤回入口；撤副作用
//! 和人工撤回是同一套，每个活跃分片都撤到。没撤成的保持已生效，下一轮再试。

use agent_room_domain::{
    ids::ModerationActionId,
    moderation::{
        ModerationAction, ModerationActionKind, ModerationAuditOutcome, ModerationTargetKind,
    },
    time::UtcMillis,
};

use crate::{
    persistence::RepositoryErrorKind,
    ports::{
        MatrixFailure, MatrixFailureKind, MatrixRoomId, ModerationExpiryCursor,
        ModerationRoomContext, PortFuture,
    },
};

use super::{
    ModerationFailureKind, ModerationResult,
    service::{ModerationService, failure, matrix_failure_code, repository_failure},
};

/// 一次取这么多个到期的动作。
const PAGE_SIZE: u16 = 50;
/// 一轮最多翻这么多页，再多的留给下一轮。
const MAX_PAGES: usize = 20;

/// 到期自动解除，由控制面的定时任务调用。
pub trait ModerationExpiryUseCases: Send + Sync {
    /// 解除一轮到期的动作：撤掉留在房间里的副作用，记成已撤销（撤销时间不早于到期时间），写一条
    /// `moderation.action.expired` 审计。撤不成的保持已生效，下一轮再试。
    fn expire_due_actions(&self) -> PortFuture<'_, ModerationResult<ModerationExpiryOutcome>>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModerationExpiryOutcome {
    /// 这一轮解除了几个。
    pub expired: usize,
    /// 这一轮没撤成、留到下一轮再试的。
    pub retrying: Vec<ModerationExpiryRetry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModerationExpiryRetry {
    pub action_id: ModerationActionId,
    /// 和动作失败时记的错误码是一套，比如 `matrix.unavailable`。
    pub failure_code: &'static str,
}

/// 一个到期的动作这次处理成什么样。
enum Step {
    Expired,
    /// 已经不在生效：管理员刚好撤回了，或者另一个控制面先一步解除了。
    AlreadyEnded,
    /// 副作用没撤成。`homeserver_down` 是整个聊天服务器这会儿用不了，别的也不用再试了。
    Retry {
        failure_code: &'static str,
        homeserver_down: bool,
    },
}

/// 到期时要撤什么。
enum Scope {
    /// 在这些分片上撤。
    Rooms(ModerationRoomContext, Vec<MatrixRoomId>),
    /// 没有留在房间里的副作用要撤，直接记成解除。
    Nothing,
    /// 有分片读不了，不知道消息在哪个分片。
    Unreadable,
}

impl ModerationService {
    async fn expire_due_actions_internal(&self) -> ModerationResult<ModerationExpiryOutcome> {
        const OPERATION: &str = "moderation.expire_due_actions";
        let now = self.clock.now();
        let mut outcome = ModerationExpiryOutcome::default();
        let mut after = None;
        for _ in 0..MAX_PAGES {
            let page = self
                .expiry
                .list_due_actions(now, after, PAGE_SIZE)
                .await
                .map_err(|error| repository_failure(OPERATION, &error))?;
            for action in &page {
                match self.expire_action(action, now).await? {
                    Step::Expired => outcome.expired += 1,
                    Step::AlreadyEnded => {}
                    Step::Retry {
                        failure_code,
                        homeserver_down,
                    } => {
                        outcome.retrying.push(ModerationExpiryRetry {
                            action_id: action.id(),
                            failure_code,
                        });
                        if homeserver_down {
                            return Ok(outcome);
                        }
                    }
                }
            }
            after = page.last().and_then(expiry_cursor);
            if page.len() < usize::from(PAGE_SIZE) || after.is_none() {
                break;
            }
        }
        Ok(outcome)
    }

    async fn expire_action(
        &self,
        action: &ModerationAction,
        now: UtcMillis,
    ) -> ModerationResult<Step> {
        const OPERATION: &str = "moderation.expire_action";
        if !action.is_due_at(now) {
            return Ok(Step::AlreadyEnded);
        }
        match self.expiry_scope(action, now, OPERATION).await? {
            Scope::Rooms(context, rooms) => {
                if let Err(matrix_failure) = self.reverse_effects(action, &context, &rooms).await {
                    return Ok(Step::Retry {
                        failure_code: matrix_failure_code(matrix_failure),
                        homeserver_down: homeserver_unavailable(matrix_failure),
                    });
                }
            }
            Scope::Unreadable => {
                return Ok(Step::Retry {
                    failure_code: "matrix.unavailable",
                    homeserver_down: false,
                });
            }
            Scope::Nothing => {}
        }
        self.record_expired(action.clone(), now, OPERATION).await
    }

    /// 到期时要在哪些分片上撤。分片照当下的找，和人工撤回一样：隐藏回到消息所在的分片，管人的动作在
    /// 每个活跃分片上撤。
    async fn expiry_scope(
        &self,
        action: &ModerationAction,
        now: UtcMillis,
        operation: &'static str,
    ) -> ModerationResult<Scope> {
        // 踢出当下就做完了，没有留在房间里的副作用；撤销踢出是把人重新请回私人房间，到期不该替管理员
        // 发这个邀请。
        if action.kind() == ModerationActionKind::Kick {
            return Ok(Scope::Nothing);
        }
        // 同一个对象上还有别的同类动作在生效（比如又禁言了一次、没设期限）：副作用留给它，这条只记成解除。
        if self
            .expiry
            .has_other_effective_action(action, now)
            .await
            .map_err(|error| repository_failure(operation, &error))?
        {
            return Ok(Scope::Nothing);
        }
        let room = self
            .expiry
            .expiry_room(action)
            .await
            .map_err(|error| repository_failure(operation, &error))?;
        // 房间已经关了（比如房主删了账户），或者管的人连账户都查不到：没有副作用可撤。
        let Some(context) = room.filter(|context| {
            !context.matrix_room_ids.is_empty()
                && (action.target().kind() != ModerationTargetKind::Principal
                    || context.target_matrix_user_id.is_some())
        }) else {
            return Ok(Scope::Nothing);
        };
        match self
            .effect_rooms(&context, action.kind(), action.target(), operation)
            .await
        {
            Ok(rooms) => Ok(Scope::Rooms(context, rooms)),
            // 消息在哪个分片里都没有了（比如过了保留期被删掉）：没有隐藏可撤。
            Err(failure) if failure.kind() == ModerationFailureKind::NotFound => Ok(Scope::Nothing),
            Err(_) => Ok(Scope::Unreadable),
        }
    }

    async fn record_expired(
        &self,
        mut action: ModerationAction,
        due_at: UtcMillis,
        operation: &'static str,
    ) -> ModerationResult<Step> {
        // 时钟往回拨了也不早于这一轮认定它到期的时间。
        let at = self.clock.now().max(due_at);
        action
            .expire(at)
            .map_err(|_| failure(operation, ModerationFailureKind::Internal))?;
        let audit = self.action_audit(
            &action,
            "moderation.action.expired",
            ModerationAuditOutcome::Allowed,
            self.identifiers.moderation_audit_event_id(),
            at,
            operation,
        )?;
        match self.repository.finalize_action(&action, &audit).await {
            Ok(_) => Ok(Step::Expired),
            // 管理员刚好撤回了，或者另一个控制面先一步解除了：已经不在生效，这回不用再记。还在生效的
            // 下一轮还会被找出来。
            Err(error) if error.kind() == RepositoryErrorKind::Conflict => Ok(Step::AlreadyEnded),
            Err(error) => Err(repository_failure(operation, &error)),
        }
    }
}

impl ModerationExpiryUseCases for ModerationService {
    fn expire_due_actions(&self) -> PortFuture<'_, ModerationResult<ModerationExpiryOutcome>> {
        Box::pin(self.expire_due_actions_internal())
    }
}

fn expiry_cursor(action: &ModerationAction) -> Option<ModerationExpiryCursor> {
    action
        .expires_at()
        .map(|expires_at| ModerationExpiryCursor {
            expires_at,
            action_id: action.id(),
        })
}

/// 整个聊天服务器这会儿用不了：连不上、超时、限速，或者不认应用服务的令牌。换别的动作也一样撤不成。
const fn homeserver_unavailable(failure: MatrixFailure) -> bool {
    matches!(
        failure.kind(),
        MatrixFailureKind::DependencyUnavailable
            | MatrixFailureKind::Timeout
            | MatrixFailureKind::RateLimited
            | MatrixFailureKind::Unauthenticated
            | MatrixFailureKind::AuthenticationRejected
    )
}
