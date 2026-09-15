use agent_room_agent_client::{
    BridgeToolClient, BridgeToolFuture, MessageReadMode, MessageWait, wait_for_messages,
};
use agent_room_bridge_ipc::{IpcListPreviewsRequest, IpcMethod, IpcResponse};
use std::{sync::Mutex, time::Duration};

#[derive(Default)]
struct Bridge {
    calls: Mutex<Vec<String>>,
}
impl BridgeToolClient for Bridge {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        self.calls.lock().unwrap().push(method.name().to_owned());
        Box::pin(async {
            Ok(IpcResponse::MessagePreviews {
                previews: Vec::new(),
                next_cursor: None,
            })
        })
    }
}
fn request() -> IpcListPreviewsRequest {
    IpcListPreviewsRequest {
        room_id: None,
        before_event_id: None,
        after_event_id: Some("$last:room.test".to_owned()),
        limit: 20,
    }
}

#[tokio::test(start_paused = true)]
async fn blocking_wait_advertises_waiting_but_a_one_off_read_does_not() {
    let bridge = Bridge::default();
    let session = "01990d9e-8400-7000-8000-000000000001".to_owned();
    wait_for_messages(
        &bridge,
        session.clone(),
        request(),
        MessageReadMode::Inbox,
        MessageWait::For(Duration::ZERO),
    )
    .await
    .unwrap();
    assert_eq!(bridge.calls.lock().unwrap().as_slice(), ["read_inbox"]);
    bridge.calls.lock().unwrap().clear();
    wait_for_messages(
        &bridge,
        session,
        request(),
        MessageReadMode::Inbox,
        MessageWait::For(Duration::from_secs(3)),
    )
    .await
    .unwrap();
    assert_eq!(
        bridge.calls.lock().unwrap().as_slice(),
        ["wait_inbox", "wait_inbox", "wait_inbox", "read_inbox"]
    );
}
