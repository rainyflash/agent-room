//! 撤销电脑时它的 Agent 离开房间（`specs/computer-revocation/design.md`）。
//!
//! 撤销的事务里，这台电脑上每个连接占着位置的“Agent + Matrix 房间”各记一条。撤销的那次请求里接着做
//! 这台电脑的；没做成的由控制面的定时任务每 30 秒领一轮，往后退避再试，和治理动作到期解除一个做法。
//!
//! 每条做之前再看一次：这个 Agent 在这个房间里还有没撤销的连接占着位置，就只放掉撤销的连接占的位置；
//! 没有就让它离开，和“离开一个房间”做的一样（私人房间先收回发言级别、在 Matrix 上离开、凭口令或敲门
//! 进来的删掉成员记录），最后放掉位置。先离开 Matrix 再放位置：反过来中途失败，房间里看得到它，
//! 却查不到是谁的。

use std::sync::Arc;

use agent_room_domain::{
    ids::{AgentId, DeviceId, RoomInstanceId},
    rooms::RoomReservationState,
    time::UtcMillis,
};

use crate::{
    agent_room_placements::{MatrixRoomToLeave, RoomLeaveFailure, leave_matrix_room},
    moderation::{homeserver_unavailable, matrix_failure_code},
    persistence::RepositoryError,
    ports::{
        AgentRoomKind, AgentRoomMembershipFactory, AgentRoomPlacementStore, Clock, PortFuture,
        PrivateRoomMatrixGateway, RevokedAgentDepartureAudit, RevokedAgentDepartureAuditOutcome,
        RevokedAgentDepartureClaim, RevokedAgentDepartureFacts, RevokedAgentDepartureStore,
        RoomAllocationStore,
    },
};

/// 一轮最多领这么多条，再多的留给下一轮。撤销一台电脑一般只有几条。
const CLAIMS_PER_ROUND: usize = 50;
/// 领到以后多久别的实例不能再领。正常做完一条只要一两秒；做到一半进程断了，过了这个时间别的实例接着做。
const LEASE_MILLIS: i64 = 2 * 60 * 1000;
/// 没做成时第一次隔多久再试，之后每次翻倍。
const RETRY_INITIAL_MILLIS: i64 = 30 * 1000;
/// 最长隔多久再试。不设放弃：放弃了它就一直挂在房间里。
const RETRY_MAXIMUM_MILLIS: i64 = 15 * 60 * 1000;
/// 记着的 Matrix 房间号不对。
const INVALID_ROOM_CODE: &str = "room.invalid_matrix_room";

/// 撤销电脑以后让它的 Agent 离开房间。
pub trait AgentRoomDepartureUseCases: Send + Sync {
    /// 撤销的那次请求里接着做：只做这台电脑记下的、已经到期的。
    fn depart_for_device(
        &self,
        device_id: DeviceId,
    ) -> PortFuture<'_, Result<DepartureRound, RepositoryError>>;

    /// 定时任务的一轮：做到期的。
    fn depart_due(&self) -> PortFuture<'_, Result<DepartureRound, RepositoryError>>;
}

/// 一轮做成什么样。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DepartureRound {
    /// 离开了几个房间。
    pub left: usize,
    /// 同一个房间里还有别的连接、只放了位置的有几条。
    pub released_only: usize,
    /// 没做成、往后退避再试的。
    pub retrying: Vec<DepartureRetry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepartureRetry {
    pub agent_id: AgentId,
    pub room_instance_id: RoomInstanceId,
    /// 和治理动作失败时记的错误码是一套，比如 `matrix.unavailable`。
    pub failure_code: &'static str,
}

pub struct AgentRoomDepartureDependencies {
    pub departures: Arc<dyn RevokedAgentDepartureStore>,
    pub placements: Arc<dyn AgentRoomPlacementStore>,
    pub allocations: Arc<dyn RoomAllocationStore>,
    pub private_matrix: Arc<dyn PrivateRoomMatrixGateway>,
    pub memberships: Arc<dyn AgentRoomMembershipFactory>,
    pub clock: Arc<dyn Clock>,
}

pub struct AgentRoomDepartureService {
    departures: Arc<dyn RevokedAgentDepartureStore>,
    placements: Arc<dyn AgentRoomPlacementStore>,
    allocations: Arc<dyn RoomAllocationStore>,
    private_matrix: Arc<dyn PrivateRoomMatrixGateway>,
    memberships: Arc<dyn AgentRoomMembershipFactory>,
    clock: Arc<dyn Clock>,
}

/// 领到的一条这次做成什么样。
enum Step {
    /// 没有要做的：撤销的连接在这个房间已经不占位置了（主人让它离开了、删掉了它），或者是网络 Agent。
    Nothing,
    Departed {
        facts: RevokedAgentDepartureFacts,
        left_room: bool,
    },
    /// 没做成。`homeserver_down` 是整个聊天服务器这会儿用不了，别的也不用再试了。
    Failed {
        facts: RevokedAgentDepartureFacts,
        failure_code: &'static str,
        homeserver_down: bool,
    },
}

impl AgentRoomDepartureService {
    pub fn new(dependencies: AgentRoomDepartureDependencies) -> Self {
        Self {
            departures: dependencies.departures,
            placements: dependencies.placements,
            allocations: dependencies.allocations,
            private_matrix: dependencies.private_matrix,
            memberships: dependencies.memberships,
            clock: dependencies.clock,
        }
    }

    async fn round(&self, device_id: Option<DeviceId>) -> Result<DepartureRound, RepositoryError> {
        let mut round = DepartureRound::default();
        for _ in 0..CLAIMS_PER_ROUND {
            let now = self.clock.now();
            let Some(claim) = self
                .departures
                .claim_departure(device_id, now, later(now, LEASE_MILLIS))
                .await?
            else {
                break;
            };
            let step = self.depart(&claim, now).await?;
            if self.settle(&claim, step, now, &mut round).await? {
                break;
            }
        }
        Ok(round)
    }

    async fn depart(
        &self,
        claim: &RevokedAgentDepartureClaim,
        now: UtcMillis,
    ) -> Result<Step, RepositoryError> {
        let Some(facts) = self.departures.departure_facts(claim).await? else {
            return Ok(Step::Nothing);
        };
        if facts.revoked_reservations.is_empty() {
            return Ok(Step::Nothing);
        }
        let left_room = !facts.live_connection_remains;
        if left_room {
            let room = MatrixRoomToLeave {
                kind: facts.kind,
                status: facts.status,
                matrix_room_id: &facts.matrix_room_id,
            };
            let left = leave_matrix_room(
                self.private_matrix.as_ref(),
                self.memberships.as_ref(),
                room,
                &facts.matrix_user_id,
            )
            .await;
            if let Err(failure) = left {
                let (failure_code, homeserver_down) = match failure {
                    RoomLeaveFailure::InvalidRoom => (INVALID_ROOM_CODE, false),
                    RoomLeaveFailure::Matrix(failure) => (
                        matrix_failure_code(failure),
                        homeserver_unavailable(failure),
                    ),
                };
                return Ok(Step::Failed {
                    facts,
                    failure_code,
                    homeserver_down,
                });
            }
            if facts.kind == AgentRoomKind::PrivateRoom {
                self.placements
                    .forget_agent_member(facts.catalog_id, claim.agent_id)
                    .await?;
            }
        }
        for reservation in &facts.revoked_reservations {
            self.allocations
                .transition(
                    *reservation,
                    RoomReservationState::Committed,
                    RoomReservationState::Released,
                    now,
                )
                .await?;
        }
        Ok(Step::Departed { facts, left_room })
    }

    /// 记下领到的这一条做成什么样：做完的删掉，没做成的排好下次，第一次没做成时写一条审计。整个聊天
    /// 服务器这会儿用不了时交回 `true`，这一轮先停。
    async fn settle(
        &self,
        claim: &RevokedAgentDepartureClaim,
        step: Step,
        now: UtcMillis,
        round: &mut DepartureRound,
    ) -> Result<bool, RepositoryError> {
        match step {
            Step::Nothing => {
                self.departures.complete_departure(claim, None).await?;
                Ok(false)
            }
            Step::Departed { facts, left_room } => {
                let outcome = RevokedAgentDepartureAuditOutcome::Departed {
                    left_room,
                    released_reservations: facts.revoked_reservations.len(),
                };
                let audit = departure_audit(&facts, now, outcome);
                // 别的实例已经接手了，做完归它记。
                if self
                    .departures
                    .complete_departure(claim, Some(&audit))
                    .await?
                {
                    if left_room {
                        round.left += 1;
                    } else {
                        round.released_only += 1;
                    }
                }
                Ok(false)
            }
            Step::Failed {
                facts,
                failure_code,
                homeserver_down,
            } => {
                let retry_at = later(now, retry_delay_millis(claim.attempt));
                let audit = claim.previous_failure_code.is_none().then(|| {
                    departure_audit(
                        &facts,
                        now,
                        RevokedAgentDepartureAuditOutcome::Failed { failure_code },
                    )
                });
                if self
                    .departures
                    .retry_departure(claim, retry_at, failure_code, audit.as_ref())
                    .await?
                {
                    round.retrying.push(DepartureRetry {
                        agent_id: claim.agent_id,
                        room_instance_id: claim.room_instance_id,
                        failure_code,
                    });
                }
                Ok(homeserver_down)
            }
        }
    }
}

impl AgentRoomDepartureUseCases for AgentRoomDepartureService {
    fn depart_for_device(
        &self,
        device_id: DeviceId,
    ) -> PortFuture<'_, Result<DepartureRound, RepositoryError>> {
        Box::pin(self.round(Some(device_id)))
    }

    fn depart_due(&self) -> PortFuture<'_, Result<DepartureRound, RepositoryError>> {
        Box::pin(self.round(None))
    }
}

fn departure_audit(
    facts: &RevokedAgentDepartureFacts,
    occurred_at: UtcMillis,
    outcome: RevokedAgentDepartureAuditOutcome,
) -> RevokedAgentDepartureAudit {
    RevokedAgentDepartureAudit {
        occurred_at,
        catalog_id: facts.catalog_id,
        matrix_room_id: facts.matrix_room_id.clone(),
        outcome,
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
    use super::*;

    #[test]
    fn 退避从_30_秒起翻倍_最多_15_分钟() {
        assert_eq!(retry_delay_millis(1), 30_000);
        assert_eq!(retry_delay_millis(2), 60_000);
        assert_eq!(retry_delay_millis(5), 480_000);
        assert_eq!(retry_delay_millis(6), 900_000);
        assert_eq!(retry_delay_millis(40), 900_000);
    }
}
