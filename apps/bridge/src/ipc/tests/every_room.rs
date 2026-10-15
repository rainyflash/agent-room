//! 不说房间时看 Agent 在的所有房间（`specs/agent-identity/design.md` 第 4 步），用真的本地消息库。

use std::{sync::Arc, time::Duration};

use agent_room_application::ports::{MatrixRoomId, MatrixRoomSyncKind, MatrixSyncToken};
use agent_room_bridge_core::messages::{
    MessageProjectionBatch, MessageProjectionMutation, MessageTimelineProjectionStore as _,
    ProjectedMessagePreview,
};
use agent_room_bridge_ipc::{
    IpcAckInboxRequest, IpcListPreviewsRequest, IpcMethod, IpcResponse, IpcRoomCursor,
};
use agent_room_bridge_storage_adapter::SqliteMessageTimelineRepository;

use super::super::BridgeIpcRequestHandler as _;
use super::room_membership::entry_with;
use super::viewing::{ROOM, chat, event, human, store_with};
use super::{
    BridgeAgentRuntimeSnapshot, FoundationBridgeIpcRequestHandler, 固定Agent运行时, 固定时钟,
    固定状态, 测试_agent_身份, 空正文服务,
};
use crate::joined_rooms::JoinedRooms;

const PRIVATE: &str = "!private:matrix.test";

fn at(mut message: ProjectedMessagePreview, received_ms: u64) -> ProjectedMessagePreview {
    message.origin_server_timestamp = Some(received_ms);
    message
}

/// 会话开在大厅，同步里还看到它在私人房间。
fn handler_in_both(
    store: Arc<SqliteMessageTimelineRepository>,
) -> FoundationBridgeIpcRequestHandler {
    let joined = Arc::new(JoinedRooms::new());
    let private = MatrixRoomId::new(PRIVATE).expect("房间有效");
    let lobby = MatrixRoomId::new(ROOM).expect("房间有效");
    joined.record([
        (&private, MatrixRoomSyncKind::Joined),
        (&lobby, MatrixRoomSyncKind::Joined),
    ]);
    FoundationBridgeIpcRequestHandler::with_agent_runtime(
        super::super::AgentRuntimeConsumer::HostSession,
        Arc::new(固定状态),
        Arc::new(固定Agent运行时(
            BridgeAgentRuntimeSnapshot::new(
                测试_agent_身份(),
                "DEVICE-1",
                lobby,
                ["previews.read"],
            )
            .with_room_entry(Arc::new(entry_with(joined))),
        )),
        store.clone(),
        空正文服务(store),
        Arc::new(固定时钟),
    )
}

fn every_room(cursors: &[(&str, String)], from_ack: bool, limit: u16) -> IpcListPreviewsRequest {
    IpcListPreviewsRequest {
        after_event_id: None,
        room_id: None,
        before_event_id: None,
        limit,
        keep_waiting: false,
        wait_ms: None,
        from_ack,
        every_room: true,
        room_cursors: cursors
            .iter()
            .map(|(room, event)| IpcRoomCursor {
                room_id: (*room).to_owned(),
                after_event_id: event.clone(),
            })
            .collect(),
    }
}

/// 读一页：每条的房间和事件，还有没有下一页。
async fn read(
    handler: &FoundationBridgeIpcRequestHandler,
    method: IpcMethod,
) -> (Vec<(String, String)>, bool) {
    match handler.dispatch(method).await.expect("可以读") {
        IpcResponse::MessagePreviews {
            previews,
            next_cursor,
            ..
        } => (
            previews
                .into_iter()
                .map(|preview| (preview.room_id, preview.event_id))
                .collect(),
            next_cursor.is_some(),
        ),
        other => panic!("必须返回消息预览：{other:?}"),
    }
}

fn seen(entries: &[(&str, u8)]) -> Vec<(String, String)> {
    entries
        .iter()
        .map(|(room, index)| ((*room).to_owned(), event(*index)))
        .collect()
}

#[tokio::test]
async fn 不说房间看所有房间_按收到的时间排_各房间从自己的位置接着读() {
    let ada = human("Ada", "@ada:matrix.test");
    let (_directory, store) = store_with(&[
        at(chat(ROOM, 1, ada.clone(), "大厅一", &[]), 1_000),
        at(chat(ROOM, 3, ada.clone(), "大厅三", &[]), 3_000),
        at(chat(PRIVATE, 2, ada.clone(), "私人二", &[]), 2_000),
        at(chat(PRIVATE, 4, ada, "私人四", &[]), 4_000),
    ])
    .await;
    let handler = handler_in_both(store);

    let everything = read(&handler, IpcMethod::ReadInbox(every_room(&[], false, 20))).await;
    assert_eq!(
        everything,
        (
            seen(&[(ROOM, 1), (PRIVATE, 2), (ROOM, 3), (PRIVATE, 4)]),
            false
        )
    );
    let first_two = read(&handler, IpcMethod::ReadInbox(every_room(&[], false, 2))).await;
    assert_eq!(first_two, (seen(&[(ROOM, 1), (PRIVATE, 2)]), true));

    // 给了位置的房间从它之后；不在的房间给了也不看。
    let after = read(
        &handler,
        IpcMethod::ReadInbox(every_room(
            &[(PRIVATE, event(2)), ("!gone:matrix.test", event(9))],
            false,
            20,
        )),
    )
    .await;
    assert_eq!(after.0, seen(&[(ROOM, 1), (ROOM, 3), (PRIVATE, 4)]));

    // 确认照旧按房间记：大厅确认到三，私人房间没确认过就从头。
    handler
        .dispatch(IpcMethod::AckInbox(IpcAckInboxRequest { id: event(3) }))
        .await
        .expect("可以确认");
    let from_ack = read(&handler, IpcMethod::WaitInbox(every_room(&[], true, 20))).await;
    assert_eq!(from_ack.0, seen(&[(PRIVATE, 2), (PRIVATE, 4)]));

    // 从新到旧看一眼（找上一条点到的人用）。
    let newest = read(&handler, IpcMethod::ListPreviews(every_room(&[], false, 3))).await;
    assert_eq!(
        newest,
        (seen(&[(PRIVATE, 4), (ROOM, 3), (PRIVATE, 2)]), true)
    );

    // 旧版客户端不设 `everyRoom`：照旧只看会话的那个房间。
    let mut old = every_room(&[], false, 20);
    old.every_room = false;
    let session_only = read(&handler, IpcMethod::ReadInbox(old)).await;
    assert_eq!(session_only.0, seen(&[(ROOM, 1), (ROOM, 3)]));
}

#[tokio::test]
async fn 挂着等时别的房间来了消息也马上交() {
    let ada = human("Ada", "@ada:matrix.test");
    let (_directory, store) =
        store_with(&[at(chat(ROOM, 1, ada.clone(), "大厅一", &[]), 1_000)]).await;
    let handler = handler_in_both(store.clone());
    let mut request = every_room(&[(ROOM, event(1))], false, 20);
    request.wait_ms = Some(5_000);

    let started = tokio::time::Instant::now();
    let (page, ()) = tokio::join!(read(&handler, IpcMethod::WaitInbox(request)), async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        store
            .apply(&MessageProjectionBatch::new(
                MatrixSyncToken::new("every-room").expect("同步游标有效"),
                vec![MessageProjectionMutation::Preview(at(
                    chat(PRIVATE, 2, ada, "私人二", &[]),
                    2_000,
                ))],
                Vec::new(),
                Vec::new(),
            ))
            .await
            .expect("消息可投影");
    });
    assert!(started.elapsed() < Duration::from_secs(3), "不等满 5 秒");
    assert_eq!(page.0, seen(&[(PRIVATE, 2)]));
}
