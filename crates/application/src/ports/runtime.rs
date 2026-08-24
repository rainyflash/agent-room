use agent_room_domain::{
    ids::{
        AdapterBindingId, AgentCardSnapshotId, AgentId, AgentInstanceId, AutomationGrantId,
        ContentId, DeviceAccessTokenId, DeviceId, DeviceRefreshTokenId, DeviceTokenFamilyId,
        HandoffId, LoginAttemptId, OutboxEventId, PrincipalId, RoomCatalogId, RoomInstanceId,
        WebSessionId,
    },
    time::UtcMillis,
};

pub trait Clock: Send + Sync {
    fn now(&self) -> UtcMillis;
}

pub trait IdentifierFactory: Send + Sync {
    fn principal_id(&self) -> PrincipalId;
    fn login_attempt_id(&self) -> LoginAttemptId;
    fn web_session_id(&self) -> WebSessionId;
    fn device_id(&self) -> DeviceId;
    fn device_token_family_id(&self) -> DeviceTokenFamilyId;
    fn device_access_token_id(&self) -> DeviceAccessTokenId;
    fn device_refresh_token_id(&self) -> DeviceRefreshTokenId;
    fn agent_id(&self) -> AgentId;
    fn agent_card_snapshot_id(&self) -> AgentCardSnapshotId;
    fn adapter_binding_id(&self) -> AdapterBindingId;
    fn agent_instance_id(&self) -> AgentInstanceId;
    fn room_catalog_id(&self) -> RoomCatalogId;
    fn room_instance_id(&self) -> RoomInstanceId;
    fn content_id(&self) -> ContentId;
    fn handoff_id(&self) -> HandoffId;
    fn automation_grant_id(&self) -> AutomationGrantId;
    fn outbox_event_id(&self) -> OutboxEventId;
}
