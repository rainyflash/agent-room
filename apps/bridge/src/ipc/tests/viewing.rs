//! 按需查看（按 ID 取、看前后、往前翻），用真的本地消息库。

use std::sync::Arc;

use agent_room_application::ports::{MatrixEventId, MatrixRoomId, MatrixSyncToken, MatrixUserId};
use agent_room_bridge_core::messages::{
    MessageProjectionBatch, MessageProjectionMutation, MessageTimelineProjectionStore as _,
    ProjectedMessageActor, ProjectedMessagePreview,
};
use agent_room_bridge_ipc::{
    IpcGetMessagesRequest, IpcListPreviewsRequest, IpcMessagePreviewSummary,
    IpcMessagesAroundRequest, IpcMethod, IpcResponse, IpcRoomHistoryRequest,
};
use agent_room_bridge_storage_adapter::{
    MessageProjectionStorageKey, SqliteMessageTimelineRepository,
};
use agent_room_domain::{
    content::Sha256Digest,
    ids::{ContentId, MessageId, PrincipalId},
    messages::ConversationMessage,
};
use uuid::Uuid;

use super::super::BridgeIpcRequestHandler as _;
use super::{
    BridgeAgentRuntimeSnapshot, FoundationBridgeIpcRequestHandler, 固定Agent运行时, 固定时钟,
    固定状态, 测试_agent_身份, 测试正文投影, 空正文服务,
};

pub(super) const ROOM: &str = "!lobby:matrix.test";
/// 测试 Agent 自己的 Matrix 用户 ID（`测试_agent_身份`）。
pub(super) const ME: &str = "@_agent_01945c1e7b5a7c7f8a282de53f56a9a3:matrix.test";

pub(super) fn human(name: &str, id: &str) -> ProjectedMessageActor {
    ProjectedMessageActor::Human {
        principal_id: PrincipalId::from_uuid(Uuid::now_v7()),
        display_name: name.to_owned(),
        matrix_user_id: MatrixUserId::new(id).expect("用户标识有效"),
        avatar_url: None,
    }
}

pub(super) fn event(index: u8) -> String {
    format!("$view-{index}:matrix.test")
}

pub(super) fn chat(
    room: &str,
    index: u8,
    actor: ProjectedMessageActor,
    text: &str,
    mentions: &[&str],
) -> ProjectedMessagePreview {
    let mut message = 测试正文投影(
        MatrixRoomId::new(room).expect("房间标识有效"),
        ContentId::from_uuid(Uuid::now_v7()),
        Sha256Digest::from_bytes([index; 32]),
    );
    message.event_id = MatrixEventId::new(event(index)).expect("事件标识有效");
    message.message_id = MessageId::from_uuid(Uuid::now_v7());
    message.actor = actor;
    message.preview = message.preview.clone().with_conversation(
        ConversationMessage::new(
            text.to_owned(),
            mentions.iter().map(|id| (*id).to_owned()).collect(),
        )
        .expect("聊天有效"),
    );
    message
}

pub(super) async fn store_with(
    messages: &[ProjectedMessagePreview],
) -> (tempfile::TempDir, Arc<SqliteMessageTimelineRepository>) {
    let directory = tempfile::tempdir().expect("临时目录可创建");
    let store = SqliteMessageTimelineRepository::open(
        &directory.path().join("messages.sqlite3"),
        &MessageProjectionStorageKey::from_bytes([3; 32]),
    )
    .await
    .expect("消息库可打开");
    store
        .apply(&MessageProjectionBatch::new(
            MatrixSyncToken::new("viewing").expect("同步游标有效"),
            messages
                .iter()
                .cloned()
                .map(MessageProjectionMutation::Preview)
                .collect(),
            Vec::new(),
            Vec::new(),
        ))
        .await
        .expect("消息可投影");
    (directory, Arc::new(store))
}

pub(super) fn handler(
    store: Arc<SqliteMessageTimelineRepository>,
) -> FoundationBridgeIpcRequestHandler {
    FoundationBridgeIpcRequestHandler::with_agent_runtime(
        super::super::AgentRuntimeConsumer::HostSession,
        Arc::new(固定状态),
        Arc::new(固定Agent运行时(BridgeAgentRuntimeSnapshot::new(
            测试_agent_身份(),
            "DEVICE-1",
            MatrixRoomId::new(ROOM).expect("房间标识有效"),
            ["previews.read"],
        ))),
        store.clone(),
        空正文服务(store),
        Arc::new(固定时钟),
    )
}

pub(super) fn event_ids(messages: &[IpcMessagePreviewSummary]) -> Vec<String> {
    messages
        .iter()
        .map(|message| message.event_id.clone())
        .collect()
}

async fn history(
    handler: &FoundationBridgeIpcRequestHandler,
    request: IpcRoomHistoryRequest,
) -> (Vec<String>, Option<String>) {
    match handler
        .dispatch(IpcMethod::RoomHistory(request))
        .await
        .expect("可以往前翻")
    {
        IpcResponse::RoomMessages {
            messages,
            next_cursor,
        } => (event_ids(&messages), next_cursor),
        other => panic!("往前翻必须返回房间里的一段消息：{other:?}"),
    }
}

#[tokio::test]
async fn 按_id_取_给全文按要的顺序_别的房间和找不到的放进_missing() {
    let ada = human("Ada", "@ada:matrix.test");
    let first = chat(ROOM, 0, ada.clone(), "第一句", &[]);
    let lengthy = chat(ROOM, 1, ada.clone(), &"长".repeat(1_200), &[]);
    let elsewhere = chat("!other:matrix.test", 2, ada, "别的房间", &[]);
    let (_directory, store) =
        store_with(&[first.clone(), lengthy.clone(), elsewhere.clone()]).await;
    let unknown = Uuid::now_v7().to_string();

    let response = handler(store)
        .dispatch(IpcMethod::GetMessages(IpcGetMessagesRequest {
            ids: vec![
                lengthy.message_id.to_string(),
                first.event_id.as_str().to_owned(),
                elsewhere.message_id.to_string(),
                unknown.clone(),
            ],
        }))
        .await
        .expect("可以按 ID 取");
    let IpcResponse::Messages {
        messages,
        missing,
        more,
    } = response
    else {
        panic!("按 ID 取必须返回取到的消息：{response:?}");
    };
    assert_eq!(event_ids(&messages), [event(1), event(0)], "按要的顺序");
    let text = &messages[0].conversation.as_ref().expect("是聊天").text;
    assert_eq!(text.chars().count(), 1_200, "按 ID 取给全文");
    assert_eq!(
        missing,
        [elsewhere.message_id.to_string(), unknown],
        "不在的房间里的当作找不到"
    );
    assert!(more.is_empty());
}

#[tokio::test]
async fn 收件箱里长消息只给开头_按_id_取回全文() {
    let ada = human("Ada", "@ada:matrix.test");
    let (_directory, store) = store_with(&[chat(ROOM, 0, ada, &"长".repeat(1_200), &[])]).await;
    let handler = handler(store);

    let response = handler
        .dispatch(IpcMethod::ReadInbox(IpcListPreviewsRequest {
            after_event_id: None,
            room_id: None,
            before_event_id: None,
            limit: 20,
            keep_waiting: false,
            wait_ms: None,
            from_ack: false,
        }))
        .await
        .expect("可以读收件箱");
    let IpcResponse::MessagePreviews { previews, .. } = response else {
        panic!("收件箱必须返回消息预览：{response:?}");
    };
    let chat = previews[0].conversation.as_ref().expect("是聊天");
    assert!(chat.truncated, "收件箱里长消息只给开头");
    assert_eq!(chat.text.chars().count(), 1_000);
    assert_eq!(chat.full_length, Some(1_200));

    let IpcResponse::Messages { messages, .. } = handler
        .dispatch(IpcMethod::GetMessages(IpcGetMessagesRequest {
            ids: vec![previews[0].message_id.clone()],
        }))
        .await
        .expect("可以按 ID 取")
    else {
        panic!("按 ID 取必须返回取到的消息");
    };
    let whole = messages[0].conversation.as_ref().expect("是聊天");
    assert!(!whole.truncated);
    assert_eq!(whole.text, "长".repeat(1_200));
}

#[tokio::test]
async fn 看前后_早的在前_长消息只给开头_那条找不到就说清楚() {
    let ada = human("Ada", "@ada:matrix.test");
    let messages: Vec<_> = (0..5)
        .map(|index| {
            let text = if index == 1 {
                "长".repeat(1_200)
            } else {
                format!("第 {index} 句")
            };
            chat(ROOM, index, ada.clone(), &text, &[])
        })
        .collect();
    let (_directory, store) = store_with(&messages).await;
    let handler = handler(store);

    let response = handler
        .dispatch(IpcMethod::MessagesAround(IpcMessagesAroundRequest {
            room_id: None,
            id: messages[2].message_id.to_string(),
            before: 1,
            after: 1,
        }))
        .await
        .expect("可以看前后");
    let IpcResponse::RoomMessages {
        messages: around,
        next_cursor,
    } = response
    else {
        panic!("看前后必须返回房间里的一段消息：{response:?}");
    };
    assert_eq!(event_ids(&around), [event(1), event(2), event(3)]);
    let long = around[0].conversation.as_ref().expect("是聊天");
    assert!(long.truncated, "看前后和收件箱一样，长消息只给开头");
    assert_eq!(long.full_length, Some(1_200));
    assert!(next_cursor.is_none());

    let failure = handler
        .dispatch(IpcMethod::MessagesAround(IpcMessagesAroundRequest {
            room_id: None,
            id: Uuid::now_v7().to_string(),
            before: 1,
            after: 1,
        }))
        .await
        .expect_err("找不到那一条");
    assert_eq!(failure.code, "bridge.message_not_found");
}

#[tokio::test]
async fn 往前翻_新的在前_给接着翻的位置_能只看某个人和提到我的() {
    let ada = human("Ada", "@ada:matrix.test");
    let nova = human("Nova", "@nova:matrix.test");
    let messages = vec![
        chat(ROOM, 0, ada.clone(), "Ada 一", &[]),
        chat(ROOM, 1, nova.clone(), "Nova 一", &[]),
        chat(ROOM, 2, ada.clone(), "Ada 点名", &[ME]),
        chat(ROOM, 3, nova, "Nova 二", &[]),
        chat(ROOM, 4, ada, "Ada 三", &[]),
    ];
    let (_directory, store) = store_with(&messages).await;
    let handler = handler(store);
    let base = IpcRoomHistoryRequest {
        room_id: None,
        before: None,
        after: None,
        limit: 2,
        from: None,
        mentions_me: false,
    };

    let (page, next) = history(&handler, base.clone()).await;
    assert_eq!(page, [event(4), event(3)], "最新的在前");
    assert_eq!(next, Some(event(3)));
    let (page, _) = history(
        &handler,
        IpcRoomHistoryRequest {
            before: next,
            ..base.clone()
        },
    )
    .await;
    assert_eq!(page, [event(2), event(1)], "接着往前翻");

    let only_ada = IpcRoomHistoryRequest {
        from: Some("ada".to_owned()),
        limit: 10,
        ..base.clone()
    };
    let (page, next) = history(&handler, only_ada).await;
    assert_eq!(
        page,
        [event(4), event(2), event(0)],
        "按名字只看 Ada，不分大小写"
    );
    assert!(next.is_none(), "翻到头了");
    let only_nova = IpcRoomHistoryRequest {
        from: Some("@nova:matrix.test".to_owned()),
        limit: 10,
        ..base.clone()
    };
    assert_eq!(history(&handler, only_nova).await.0, [event(3), event(1)]);
    let mentions_me = IpcRoomHistoryRequest {
        mentions_me: true,
        limit: 10,
        ..base.clone()
    };
    assert_eq!(history(&handler, mentions_me).await.0, [event(2)]);

    // 从某条往后翻，旧的在前；消息 ID 也能当位置。
    let forward = IpcRoomHistoryRequest {
        after: Some(messages[1].message_id.to_string()),
        ..base
    };
    assert_eq!(history(&handler, forward).await.0, [event(2), event(3)]);
}
