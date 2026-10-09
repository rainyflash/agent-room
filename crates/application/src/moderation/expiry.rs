//! 治理动作到期自动解除。这是系统自己做的：没有登录的人，不走要权限、要最近认证的撤回入口；撤副作用
//! 和人工撤回是同一套。动作一个一个地领，领到的带 2 分钟租约，多个控制面实例不会同时做同一个；没撤成
//! 的保持已生效，往后退避再试（specs/moderation/timed-mute-expiry.md）。

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
        MatrixFailure, MatrixFailureKind, MatrixRoomId, ModerationExpiryClaim,
        ModerationExpiryReschedule, ModerationRoomContext, PortFuture,
    },
};

use super::{
    ModerationFailureKind, ModerationResult,
    mutes::{MuteCourse, settled_after},
    service::{ModerationService, failure, matrix_failure_code, repository_failure},
};

/// 一轮最多领这么多个，再多的留给下一轮。
const CLAIMS_PER_ROUND: usize = 20;
/// 领到以后多久别的实例不能再领。正常做完一个只要几秒；做到一半进程断了，过了这个时间别的实例接着做。
const LEASE_MILLIS: i64 = 2 * 60 * 1000;
/// 撤不成时第一次隔多久再试，之后每次翻倍。
const RETRY_INITIAL_MILLIS: i64 = 30 * 1000;
/// 最长隔多久再试。不设放弃：放弃了这个人就一直禁着，比一直重试更糟。
const RETRY_MAXIMUM_MILLIS: i64 = 15 * 60 * 1000;
/// 定不下来的过多久再看。比定时任务的间隔（30 秒）短一点，下一轮就能领到。
const SETTLE_DELAY_MILLIS: i64 = 20 * 1000;

/// 到期自动解除，由控制面的定时任务调用。
pub trait ModerationExpiryUseCases: Send + Sync {
    /// 解除一轮到期的动作：一个一个地领，撤掉留在房间里的副作用，记成已撤销（撤销时间不早于到期时间），
    /// 写一条 `moderation.action.expired` 审计。撤不成的保持已生效，往后退避再试。
    fn expire_due_actions(&self) -> PortFuture<'_, ModerationResult<ModerationExpiryOutcome>>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModerationExpiryOutcome {
    /// 这一轮解除了几个。
    pub expired: usize,
    /// 这一轮没撤成、往后退避再试的。
    pub retrying: Vec<ModerationExpiryRetry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModerationExpiryRetry {
    pub action_id: ModerationActionId,
    /// 和动作失败时记的错误码是一套，比如 `matrix.unavailable`。
    pub failure_code: &'static str,
}

/// 领到的一个动作这次处理成什么样。
enum Step {
    Expired,
    /// 已经不在生效：管理员刚好撤回了，或者另一个控制面先一步解除了。
    AlreadyEnded,
    /// 这次定不下来：有一条禁言正在落、动完再看一眼变了，或者别处正在处理同一个人。
    Unsettled,
    /// 副作用没撤成。`homeserver_down` 是整个聊天服务器这会儿用不了，别的也不用再试了。
    Failed {
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
        let mut outcome = ModerationExpiryOutcome::default();
        for _ in 0..CLAIMS_PER_ROUND {
            let now = self.clock.now();
            let Some(claim) = self
                .expiry
                .claim_due_action(now, later(now, LEASE_MILLIS))
                .await
                .map_err(|error| repository_failure(OPERATION, &error))?
            else {
                break;
            };
            let step = self.expire_action(&claim.action, now).await?;
            let homeserver_down = self.settle_claim(&claim, step, now, &mut outcome).await?;
            if homeserver_down {
                break;
            }
        }
        Ok(outcome)
    }

    /// 记下领到的这一个做成什么样：没记成到期的排好下次，第一次撤不成时写一条审计。整个聊天服务器这会儿
    /// 用不了时交回 `true`，这一轮先停。
    async fn settle_claim(
        &self,
        claim: &ModerationExpiryClaim,
        step: Step,
        now: UtcMillis,
        outcome: &mut ModerationExpiryOutcome,
    ) -> ModerationResult<bool> {
        const OPERATION: &str = "moderation.settle_expiry";
        match step {
            Step::Expired => {
                outcome.expired += 1;
                Ok(false)
            }
            Step::AlreadyEnded => Ok(false),
            Step::Unsettled => {
                let at = later(now, SETTLE_DELAY_MILLIS);
                self.reschedule(claim, ModerationExpiryReschedule::Defer { at }, OPERATION)
                    .await?;
                Ok(false)
            }
            Step::Failed {
                failure_code,
                homeserver_down,
            } => {
                let at = later(now, retry_delay_millis(claim.attempt));
                let retry = ModerationExpiryReschedule::Retry { at, failure_code };
                // 别的实例已经接手了，失败归它记。
                if self.reschedule(claim, retry, OPERATION).await?
                    && claim.previous_failure_code.is_none()
                {
                    let audit = self.action_audit(
                        &claim.action,
                        "moderation.action.expire_failed",
                        ModerationAuditOutcome::Failed,
                        self.identifiers.moderation_audit_event_id(),
                        now,
                        OPERATION,
                    )?;
                    self.repository
                        .append_audit(&audit)
                        .await
                        .map_err(|error| repository_failure(OPERATION, &error))?;
                }
                outcome.retrying.push(ModerationExpiryRetry {
                    action_id: claim.action.id(),
                    failure_code,
                });
                Ok(homeserver_down)
            }
        }
    }

    async fn reschedule(
        &self,
        claim: &ModerationExpiryClaim,
        reschedule: ModerationExpiryReschedule,
        operation: &'static str,
    ) -> ModerationResult<bool> {
        self.expiry
            .reschedule_expiry(claim, reschedule)
            .await
            .map_err(|error| repository_failure(operation, &error))
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
        if action.kind() == ModerationActionKind::Mute {
            return self.expire_mute(action, now, OPERATION).await;
        }
        match self.expiry_scope(action, now, OPERATION).await? {
            Scope::Rooms(context, rooms) => {
                if let Err(matrix_failure) = self.reverse_effects(action, &context, &rooms).await {
                    return Ok(failed(matrix_failure));
                }
            }
            Scope::Unreadable => {
                return Ok(Step::Failed {
                    failure_code: "matrix.unavailable",
                    homeserver_down: false,
                });
            }
            Scope::Nothing => {}
        }
        self.record_expired(action.clone(), now, OPERATION).await
    }

    /// 禁言到期：先拿这个人的禁言锁。拿不到就是别处（另一个实例、撤回禁言的管理员）正在处理他，
    /// 下一轮再看。
    async fn expire_mute(
        &self,
        action: &ModerationAction,
        now: UtcMillis,
        operation: &'static str,
    ) -> ModerationResult<Step> {
        let Some(lock) = self
            .mutes
            .try_lock_mutes(action.room_catalog_id(), action.target())
            .await
            .map_err(|error| repository_failure(operation, &error))?
        else {
            return Ok(Step::Unsettled);
        };
        let step = self.expire_mute_locked(action, now, operation).await;
        lock.release().await;
        step
    }

    /// 看这个人此刻该不该禁着、照着动 Matrix，动完再看一眼，一样才记成到期。
    async fn expire_mute_locked(
        &self,
        action: &ModerationAction,
        now: UtcMillis,
        operation: &'static str,
    ) -> ModerationResult<Step> {
        // 目录都没了：没有副作用可撤。
        let Some(standing) = self.mute_standing(action, operation).await? else {
            return self.record_expired(action.clone(), now, operation).await;
        };
        let course = MuteCourse::of(&standing, action, now);
        match course {
            MuteCourse::Unsettled => return Ok(Step::Unsettled),
            MuteCourse::LeaveAlone => {
                return self.record_expired(action.clone(), now, operation).await;
            }
            MuteCourse::KeepMuted(_) | MuteCourse::Lift => {}
        }
        if let Err(matrix_failure) = self.follow_mute_course(&course, action, &standing).await {
            return Ok(failed(matrix_failure));
        }
        // 期间有人又禁了他、那条生效的被撤了，或者新开了分片：这次不记，下一轮从头再来。
        let after = self.mute_standing(action, operation).await?;
        if !settled_after(&standing, &course, after.as_ref(), action, now) {
            return Ok(Step::Unsettled);
        }
        self.record_expired(action.clone(), now, operation).await
    }

    /// 隐藏、封禁、踢出到期时要在哪些分片上撤。分片照当下的找，和人工撤回一样：隐藏回到消息所在的
    /// 分片，封禁在每个活跃分片上撤。
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
        // 同一个对象上还有别的同类动作在生效（比如又封禁了一次、没设期限）：副作用留给它，这条只记成解除。
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
            // 管理员刚好撤回了，或者另一个控制面先一步解除了：已经不在生效，这回不用再记。
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

fn failed(matrix_failure: MatrixFailure) -> Step {
    Step::Failed {
        failure_code: matrix_failure_code(matrix_failure),
        homeserver_down: homeserver_unavailable(matrix_failure),
    }
}

/// `at` 再过 `millis` 毫秒。
fn later(at: UtcMillis, millis: i64) -> UtcMillis {
    UtcMillis::new(at.value().saturating_add(millis)).unwrap_or(at)
}

/// 第 `attempt` 次撤不成以后隔多久再试：30 秒起每次翻倍，最长 15 分钟。
fn retry_delay_millis(attempt: u32) -> i64 {
    let doublings = attempt.saturating_sub(1).min(20);
    RETRY_INITIAL_MILLIS
        .saturating_mul(1_i64 << doublings)
        .min(RETRY_MAXIMUM_MILLIS)
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
