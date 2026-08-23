use agent_room_domain::{
    ids::{
        AgentId, AgentInstanceId, AutomationGrantId, ContentId, DeviceId, HandoffId,
        LoginAttemptId, OutboxEventId, PrincipalId, RoomCatalogId, RoomInstanceId, WebSessionId,
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
    fn agent_id(&self) -> AgentId;
    fn agent_instance_id(&self) -> AgentInstanceId;
    fn room_catalog_id(&self) -> RoomCatalogId;
    fn room_instance_id(&self) -> RoomInstanceId;
    fn content_id(&self) -> ContentId;
    fn handoff_id(&self) -> HandoffId;
    fn automation_grant_id(&self) -> AutomationGrantId;
    fn outbox_event_id(&self) -> OutboxEventId;
}
