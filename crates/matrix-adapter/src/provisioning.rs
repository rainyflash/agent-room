use std::{fmt, sync::Arc};

use agent_room_application::ports::{
    MatrixCreateRoom, MatrixEventId, MatrixEventType, MatrixFailure, MatrixFailureKind,
    MatrixGateway, MatrixOperation, MatrixResult, MatrixRoomAliasLocalpart, MatrixRoomId,
    MatrixStateEvent, MatrixStateKey, PortFuture, RoomProvisioningGateway,
};
use matrix_sdk::ruma::{RoomId, events::space::child::SpaceChildEventContent};

#[derive(Clone)]
pub struct MatrixRoomProvisioningAdapter {
    gateway: Arc<dyn MatrixGateway>,
}

impl MatrixRoomProvisioningAdapter {
    pub const fn new(gateway: Arc<dyn MatrixGateway>) -> Self {
        Self { gateway }
    }
}

impl fmt::Debug for MatrixRoomProvisioningAdapter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixRoomProvisioningAdapter")
            .finish_non_exhaustive()
    }
}

impl RoomProvisioningGateway for MatrixRoomProvisioningAdapter {
    fn create_room<'a>(
        &'a self,
        request: &'a MatrixCreateRoom,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        self.gateway.create_room(request)
    }

    fn resolve_room_alias<'a>(
        &'a self,
        alias_localpart: &'a MatrixRoomAliasLocalpart,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        self.gateway.resolve_room_alias(alias_localpart)
    }

    fn attach_child<'a>(
        &'a self,
        space_id: &'a MatrixRoomId,
        child_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<MatrixEventId>> {
        Box::pin(async move {
            let event = space_child_event(child_id)?;
            self.gateway.send_state_event(space_id, &event).await
        })
    }
}

fn space_child_event(child_id: &MatrixRoomId) -> MatrixResult<MatrixStateEvent> {
    let parsed = RoomId::parse(child_id.as_str())
        .map_err(|_| invalid_response(MatrixOperation::SendStateEvent))?;
    let via = parsed
        .server_name()
        .map(ToOwned::to_owned)
        .ok_or_else(|| invalid_response(MatrixOperation::SendStateEvent))?;
    let mut content = SpaceChildEventContent::new(vec![via]);
    content.suggested = true;
    let content = serde_json::to_value(content)
        .map_err(|_| invalid_response(MatrixOperation::SendStateEvent))?;
    let event_type = MatrixEventType::new("m.space.child")
        .map_err(|_| invalid_response(MatrixOperation::SendStateEvent))?;
    let state_key = MatrixStateKey::new(child_id.as_str())
        .map_err(|_| invalid_response(MatrixOperation::SendStateEvent))?;
    MatrixStateEvent::new(event_type, state_key, content)
        .map_err(|_| invalid_response(MatrixOperation::SendStateEvent))
}

const fn invalid_response(operation: MatrixOperation) -> MatrixFailure {
    MatrixFailure::new(operation, MatrixFailureKind::InvalidResponse)
}

#[cfg(test)]
mod tests {
    use agent_room_application::ports::MatrixRoomId;

    use super::space_child_event;

    #[test]
    fn space_child_使用子房间作为状态键并声明可达服务器() {
        let child =
            MatrixRoomId::new("!child:matrix.agent-room.localhost").expect("子房间标识有效");
        let event = space_child_event(&child).expect("Space 子状态有效");

        assert_eq!(event.event_type().as_str(), "m.space.child");
        assert_eq!(event.state_key().as_str(), child.as_str());
        assert_eq!(
            event.content()["via"],
            serde_json::json!(["matrix.agent-room.localhost"])
        );
        assert_eq!(event.content()["suggested"], true);
    }
}
