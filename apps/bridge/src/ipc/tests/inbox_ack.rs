//! 收件箱的确认位置（`AckInbox`、`fromAck`），用真的本地消息库。

use agent_room_bridge_ipc::{IpcAckInboxRequest, IpcListPreviewsRequest, IpcMethod, IpcResponse};
use uuid::Uuid;

use super::super::BridgeIpcRequestHandler as _;
use super::FoundationBridgeIpcRequestHandler;
use super::viewing::{ME, ROOM, chat, event, event_ids, handler, human, store_with};

fn inbox(after: Option<String>, from_ack: bool) -> IpcMethod {
    IpcMethod::ReadInbox(IpcListPreviewsRequest {
        after_event_id: after,
        room_id: None,
        before_event_id: None,
        limit: 20,
        keep_waiting: false,
        wait_ms: None,
        from_ack,
    })
}

async fn read(handler: &FoundationBridgeIpcRequestHandler, method: IpcMethod) -> Vec<String> {
    match handler.dispatch(method).await.expect("可以读收件箱") {
        IpcResponse::MessagePreviews { previews, .. } => event_ids(&previews),
        other => panic!("收件箱必须返回消息预览：{other:?}"),
    }
}

async fn ack(handler: &FoundationBridgeIpcRequestHandler, id: String) -> (String, bool, u64) {
    match handler
        .dispatch(IpcMethod::AckInbox(IpcAckInboxRequest { id }))
        .await
        .expect("可以确认")
    {
        IpcResponse::InboxAcknowledged {
            room_id,
            event_id,
            acknowledged,
            pending,
        } => {
            assert_eq!(room_id, ROOM);
            (event_id, acknowledged, pending)
        }
        other => panic!("确认必须返回确认结果：{other:?}"),
    }
}

#[tokio::test]
async fn 确认以后从确认位置开始读_不带_from_ack_的照旧从头() {
    let ada = human("Ada", "@ada:matrix.test");
    let me = human("我", ME);
    let messages = [
        chat(ROOM, 0, ada.clone(), "第一句", &[]),
        chat(ROOM, 1, me, "我回了一句", &[]),
        chat(ROOM, 2, ada.clone(), "第三句", &[]),
        chat(ROOM, 3, ada, "第四句", &[]),
    ];
    let (_directory, store) = store_with(&messages).await;
    let handler = handler(store);

    // 没确认过：从最早一条开始。
    let all = [event(0), event(1), event(2), event(3)];
    assert_eq!(read(&handler, inbox(None, true)).await, all);

    // 确认到第二条（自己发的）：还剩两条别人发的。
    assert_eq!(ack(&handler, event(1)).await, (event(1), true, 2));
    assert_eq!(
        read(&handler, inbox(None, true)).await,
        [event(2), event(3)]
    );
    // 不要确认位置的照旧从头；给了位置就从它之后，不看确认位置。
    assert_eq!(read(&handler, inbox(None, false)).await, all);
    assert_eq!(
        read(&handler, inbox(Some(event(2)), true)).await,
        [event(3)]
    );

    // 往回确认不动位置；用消息 ID 确认也行。
    assert_eq!(ack(&handler, event(0)).await, (event(0), false, 2));
    let last = messages[3].message_id.to_string();
    assert_eq!(ack(&handler, last).await, (event(3), true, 0));
    assert!(read(&handler, inbox(None, true)).await.is_empty());
}

#[tokio::test]
async fn 不在的房间里的和找不到的不能确认() {
    let ada = human("Ada", "@ada:matrix.test");
    let elsewhere = chat("!other:matrix.test", 0, ada, "别的房间", &[]);
    let (_directory, store) = store_with(std::slice::from_ref(&elsewhere)).await;
    let handler = handler(store);

    for id in [
        elsewhere.event_id.as_str().to_owned(),
        Uuid::now_v7().to_string(),
    ] {
        let failure = handler
            .dispatch(IpcMethod::AckInbox(IpcAckInboxRequest { id }))
            .await
            .expect_err("不能确认");
        assert_eq!(failure.code, "bridge.message_not_found");
    }
}
