//! 房间的常驻公告：同步到的交一次，同一版再同步到不算改，撤掉也交一次；进房间时读到的那一版
//! 记成交过了，之后同步到同一版不再交。

use std::time::Duration;

use agent_room_application::ports::{
    MatrixEventId, MatrixEventType, MatrixRoomId, MatrixRoomSync, MatrixRoomSyncKind,
    MatrixSyncBatch, MatrixSyncToken, MatrixTimelineEvent, MatrixUserId, NetworkAgentRoomNotice,
};
use agent_room_bridge_core::messages::ROOM_NOTICE_EVENT_TYPE;
use agent_room_domain::time::UtcMillis;
use serde_json::{Value, json};

use super::{Harness, NetworkAgentMessaging, PRIVATE_ROOM, TOKEN, everything, harness_in};
use crate::network_gateway::NetworkAgentMessages;

const OWNER: &str = "@owner:matrix.test";

/// 在私人房间里的网络 Agent：同步都走加密客户端，按顺序给这几批。
fn in_private(batches: Vec<MatrixSyncBatch>) -> Harness {
    let harness = harness_in(&[PRIVATE_ROOM]);
    *harness.agents.encrypted_since.lock().unwrap() = Some(UtcMillis::new(1).unwrap());
    harness
        .encrypted
        .batches
        .lock()
        .unwrap()
        .extend(batches.into_iter().map(Ok));
    harness
}

fn batch(next: &str, events: Vec<MatrixTimelineEvent>) -> MatrixSyncBatch {
    MatrixSyncBatch::new(
        MatrixSyncToken::new(next).unwrap(),
        vec![MatrixRoomSync::new(
            MatrixRoomId::new(PRIVATE_ROOM).unwrap(),
            MatrixRoomSyncKind::Joined,
            false,
            None,
            events,
            Vec::new(),
        )],
    )
}

/// 公告状态事件的内容；正文是空的就是撤掉。`second` 是改的时间（这一分钟里的第几秒）。
fn content(text: &str, second: u32) -> Value {
    json!({
        "schemaVersion": 1,
        "eventType": ROOM_NOTICE_EVENT_TYPE,
        "text": text,
        "updatedBy": OWNER,
        "updatedAt": format!("2026-11-03T08:00:{second:02}Z"),
    })
}

/// 应用服务写的那条公告状态事件。
fn notice_event(event_id: &str, text: &str, second: u32) -> MatrixTimelineEvent {
    MatrixTimelineEvent::new(
        Some(MatrixEventId::new(event_id).unwrap()),
        Some(MatrixUserId::new("@agent-room:matrix.test").unwrap()),
        MatrixEventType::new(ROOM_NOTICE_EVENT_TYPE).unwrap(),
        Some(String::new()),
        None,
        Some(1_793_692_800_000),
        content(text, second),
    )
    .unwrap()
}

fn notice(text: Option<&str>, second: u32) -> NetworkAgentRoomNotice {
    NetworkAgentRoomNotice {
        room_id: MatrixRoomId::new(PRIVATE_ROOM).unwrap(),
        text: text.map(str::to_owned),
        updated_by: OWNER.to_owned(),
        updated_at_ms: 1_793_692_800_000 + i64::from(second) * 1_000,
    }
}

async fn take(harness: &Harness) -> NetworkAgentMessages {
    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::ZERO, 50))
        .await
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn 同步到的公告交一次_同一版再同步到不算改_撤掉也交一次() {
    let harness = in_private(vec![
        batch(
            "e1",
            vec![notice_event("$n1:matrix.test", "只在被点名时说话。", 1)],
        ),
        // 从头同步又给了同一版。
        batch(
            "e2",
            vec![notice_event("$n1:matrix.test", "只在被点名时说话。", 1)],
        ),
        batch("e3", vec![notice_event("$n2:matrix.test", "", 2)]),
    ]);
    assert_eq!(
        take(&harness).await.notices,
        [notice(Some("只在被点名时说话。"), 1)]
    );
    assert!(take(&harness).await.notices.is_empty(), "同一版只交一次");
    assert_eq!(take(&harness).await.notices, [notice(None, 2)], "撤掉也说");
    assert!(take(&harness).await.notices.is_empty());
}

#[tokio::test(start_paused = true)]
async fn 进房间时交了现在的_之后同步到同一版不再交_改了再交() {
    let harness = in_private(vec![
        batch("e1", vec![notice_event("$n1:matrix.test", "回复要短。", 1)]),
        batch(
            "e2",
            vec![notice_event("$n2:matrix.test", "回复要短，用中文。", 2)],
        ),
    ]);
    harness
        .matrix
        .notices
        .lock()
        .unwrap()
        .insert(PRIVATE_ROOM.to_owned(), content("回复要短。", 1));
    assert_eq!(
        harness.gateway.current_notice(TOKEN, PRIVATE_ROOM).await,
        Some(notice(Some("回复要短。"), 1))
    );
    assert!(take(&harness).await.notices.is_empty(), "进房间时交过了");
    assert_eq!(
        take(&harness).await.notices,
        [notice(Some("回复要短，用中文。"), 2)]
    );

    // 撤掉了的、没有公告的房间，进房间时都没有。
    harness
        .matrix
        .notices
        .lock()
        .unwrap()
        .insert(PRIVATE_ROOM.to_owned(), content("", 3));
    assert_eq!(
        harness.gateway.current_notice(TOKEN, PRIVATE_ROOM).await,
        None
    );
    assert_eq!(
        harness
            .gateway
            .current_notice(TOKEN, "!elsewhere:matrix.test")
            .await,
        None
    );
    assert!(
        take(&harness).await.notices.is_empty(),
        "撤掉的那一版进房间时交过了"
    );
}
