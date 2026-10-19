use agent_room_domain::{
    ids::{AgentId, DeviceId, RoomCatalogId, RoomInstanceId, RoomReservationId},
    rooms::{MatrixRoomReference, RoomCatalogStatus},
    time::UtcMillis,
};

use crate::{
    persistence::RepositoryResult,
    ports::{MatrixUserId, PortFuture},
};

use super::AgentRoomKind;

/// 领到的一条：撤销电脑以后要处理的一个“Agent + Matrix 房间”（specs/computer-revocation/design.md）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevokedAgentDepartureClaim {
    pub agent_id: AgentId,
    pub room_instance_id: RoomInstanceId,
    /// 记下它的那次撤销是哪台电脑。
    pub device_id: DeviceId,
    /// 领过几次，算上这次。排下次、做完删掉时拿它认是不是自己领的那次，退避也按它算。
    pub attempt: u32,
    /// 上次没做成的原因；没有时这次没做成要写一条审计。
    pub previous_failure_code: Option<String>,
}

/// 做之前看到的这个 Agent 在这个房间里的样子。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevokedAgentDepartureFacts {
    pub matrix_user_id: MatrixUserId,
    pub catalog_id: RoomCatalogId,
    pub matrix_room_id: MatrixRoomReference,
    pub kind: AgentRoomKind,
    pub status: RoomCatalogStatus,
    /// 这个 Agent 在这个房间里还有没撤销的连接占着位置：这时不离开，只放掉撤销的连接占的位置。
    pub live_connection_remains: bool,
    /// 撤销了的连接在这个房间里还占着的位置（已确认的）。
    pub revoked_reservations: Vec<RoomReservationId>,
}

/// 做完或者第一次没做成时写的审计。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevokedAgentDepartureAudit {
    pub occurred_at: UtcMillis,
    pub catalog_id: RoomCatalogId,
    pub matrix_room_id: MatrixRoomReference,
    pub outcome: RevokedAgentDepartureAuditOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevokedAgentDepartureAuditOutcome {
    /// `left_room` 为假时同一个房间里还有别的连接，只放了位置。
    Departed {
        left_room: bool,
        released_reservations: usize,
    },
    Failed {
        failure_code: &'static str,
    },
}

/// 撤销电脑以后要离开的房间，一条一条领、做完删掉。
pub trait RevokedAgentDepartureStore: Send + Sync {
    /// 领一条到期的，同一条语句里设好租约、次数加一。别的实例正在领的跳过。给了 `device_id` 只领这台
    /// 电脑记下的。
    fn claim_departure(
        &self,
        device_id: Option<DeviceId>,
        now: UtcMillis,
        lease_until: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<RevokedAgentDepartureClaim>>>;

    /// 这个 Agent 在这个房间里现在的样子。是网络 Agent、或者 Agent 和房间有一个找不到时为空，不用做。
    fn departure_facts<'a>(
        &'a self,
        claim: &'a RevokedAgentDepartureClaim,
    ) -> PortFuture<'a, RepositoryResult<Option<RevokedAgentDepartureFacts>>>;

    /// 做完了：还是自己领的那次时删掉这一条，有审计就在同一个事务里写。别的实例已经接手时返回 `false`。
    fn complete_departure<'a>(
        &'a self,
        claim: &'a RevokedAgentDepartureClaim,
        audit: Option<&'a RevokedAgentDepartureAudit>,
    ) -> PortFuture<'a, RepositoryResult<bool>>;

    /// 没做成：还是自己领的那次时排到 `retry_at` 再试、记下原因，有审计就在同一个事务里写。
    fn retry_departure<'a>(
        &'a self,
        claim: &'a RevokedAgentDepartureClaim,
        retry_at: UtcMillis,
        failure_code: &'static str,
        audit: Option<&'a RevokedAgentDepartureAudit>,
    ) -> PortFuture<'a, RepositoryResult<bool>>;
}
