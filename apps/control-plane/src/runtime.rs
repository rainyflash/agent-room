use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_room_application::{
    ports::{Clock, IdentifierFactory, ModerationIdentifierFactory, NetworkAgentPause, PortFuture},
    rooms::{LobbyProvisioningIdentifierFactory, RoomReservationIdentifierFactory},
};
use agent_room_domain::{
    ids::{
        AdapterBindingId, AgentCardSnapshotId, AgentId, AgentInstanceId, AuditEventId,
        AutomationGrantId, ContentId, DeviceAccessTokenId, DeviceId, DeviceRefreshTokenId,
        DeviceTokenFamilyId, HandoffId, LoginAttemptId, ModerationActionId, ModerationCaseId,
        OutboxEventId, PrincipalId, RoomCatalogId, RoomInstanceId, RoomProvisioningJobId,
        RoomProvisioningLeaseId, RoomReservationId, WebSessionId,
    },
    time::UtcMillis,
};
use uuid::Uuid;

/// 大厅正在准备房间时，网络 Agent 的创建请求最多等这么久再试一次，免得请求挂太久。
const MAX_NETWORK_AGENT_PAUSE: Duration = Duration::from_secs(3);

pub(crate) struct SystemRuntime;

impl Clock for SystemRuntime {
    fn now(&self) -> UtcMillis {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时钟不得早于 Unix epoch");
        let milliseconds = i64::try_from(elapsed.as_millis()).expect("系统时间不得超出 i64");
        UtcMillis::new(milliseconds).expect("当前系统时间必须有效")
    }
}

impl IdentifierFactory for SystemRuntime {
    fn principal_id(&self) -> PrincipalId {
        PrincipalId::from_uuid(Uuid::now_v7())
    }

    fn login_attempt_id(&self) -> LoginAttemptId {
        LoginAttemptId::from_uuid(Uuid::now_v7())
    }

    fn web_session_id(&self) -> WebSessionId {
        WebSessionId::from_uuid(Uuid::now_v7())
    }

    fn device_id(&self) -> DeviceId {
        DeviceId::from_uuid(Uuid::now_v7())
    }

    fn device_token_family_id(&self) -> DeviceTokenFamilyId {
        DeviceTokenFamilyId::from_uuid(Uuid::now_v7())
    }

    fn device_access_token_id(&self) -> DeviceAccessTokenId {
        DeviceAccessTokenId::from_uuid(Uuid::now_v7())
    }

    fn device_refresh_token_id(&self) -> DeviceRefreshTokenId {
        DeviceRefreshTokenId::from_uuid(Uuid::now_v7())
    }

    fn agent_id(&self) -> AgentId {
        AgentId::from_uuid(Uuid::now_v7())
    }

    fn agent_card_snapshot_id(&self) -> AgentCardSnapshotId {
        AgentCardSnapshotId::from_uuid(Uuid::now_v7())
    }

    fn adapter_binding_id(&self) -> AdapterBindingId {
        AdapterBindingId::from_uuid(Uuid::now_v7())
    }

    fn agent_instance_id(&self) -> AgentInstanceId {
        AgentInstanceId::from_uuid(Uuid::now_v7())
    }

    fn room_catalog_id(&self) -> RoomCatalogId {
        RoomCatalogId::from_uuid(Uuid::now_v7())
    }

    fn room_instance_id(&self) -> RoomInstanceId {
        RoomInstanceId::from_uuid(Uuid::now_v7())
    }

    fn room_reservation_id(&self) -> RoomReservationId {
        RoomReservationId::from_uuid(Uuid::now_v7())
    }

    fn content_id(&self) -> ContentId {
        ContentId::from_uuid(Uuid::now_v7())
    }

    fn handoff_id(&self) -> HandoffId {
        HandoffId::from_uuid(Uuid::now_v7())
    }

    fn automation_grant_id(&self) -> AutomationGrantId {
        AutomationGrantId::from_uuid(Uuid::now_v7())
    }

    fn outbox_event_id(&self) -> OutboxEventId {
        OutboxEventId::from_uuid(Uuid::now_v7())
    }
}

impl RoomReservationIdentifierFactory for SystemRuntime {
    fn room_reservation_id(&self) -> RoomReservationId {
        RoomReservationId::from_uuid(Uuid::now_v7())
    }
}

impl LobbyProvisioningIdentifierFactory for SystemRuntime {
    fn room_provisioning_job_id(&self) -> RoomProvisioningJobId {
        RoomProvisioningJobId::from_uuid(Uuid::now_v7())
    }

    fn room_provisioning_lease_id(&self) -> RoomProvisioningLeaseId {
        RoomProvisioningLeaseId::from_uuid(Uuid::now_v7())
    }

    fn room_instance_id(&self) -> RoomInstanceId {
        RoomInstanceId::from_uuid(Uuid::now_v7())
    }
}

impl ModerationIdentifierFactory for SystemRuntime {
    fn moderation_case_id(&self) -> ModerationCaseId {
        ModerationCaseId::from_uuid(Uuid::now_v7())
    }

    fn moderation_action_id(&self) -> ModerationActionId {
        ModerationActionId::from_uuid(Uuid::now_v7())
    }

    fn moderation_audit_event_id(&self) -> AuditEventId {
        AuditEventId::from_uuid(Uuid::now_v7())
    }
}

impl NetworkAgentPause for SystemRuntime {
    fn until(&self, at: UtcMillis) -> PortFuture<'_, ()> {
        let remaining = at.value().saturating_sub(self.now().value()).max(0);
        let wait = Duration::from_millis(u64::try_from(remaining).unwrap_or(0))
            .min(MAX_NETWORK_AGENT_PAUSE);
        Box::pin(tokio::time::sleep(wait))
    }
}
