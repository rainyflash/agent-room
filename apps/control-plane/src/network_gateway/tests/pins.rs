//! 房间的置顶：同步到的交一次整份，同一版再同步到不算改，全取消了也交一次；进房间时读到的那一版记成
//! 交过了。置顶的那一句在它的消息记录里就带上开头，不在的写明是进来以前的、读不到。

use std::time::Duration;

use agent_room_application::ports::{
    MatrixEventId, MatrixEventType, MatrixRoomId, MatrixRoomSync, MatrixRoomSyncKind,
    MatrixSyncBatch, MatrixSyncToken, MatrixTimelineEvent, MatrixUserId,
};
use agent_room_bridge_core::messages::ROOM_PINS_EVENT_TYPE;
use agent_room_bridge_ipc::{IpcPinUnreadable, IpcRoomPins};
use agent_room_domain::time::UtcMillis;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    Harness, NetworkAgentMessaging, PRIVATE_ROOM, TOKEN, chat, everything, harness_in, in_room,
    other,
};
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

/// 置顶状态的内容：`pinned` 里的几句，我们的字段记谁、什么时候置顶的；`second` 是改的时间。
fn content(pinned: &[&str], second: u32) -> Value {
    let at = format!("2026-11-16T09:00:{second:02}Z");
    let entries: Vec<Value> = pinned
        .iter()
        .map(|event| json!({"eventId": event, "pinnedBy": OWNER, "pinnedAt": at}))
        .collect();
    json!({
        "pinned": pinned,
        "io.github.rainyflash.agentroom.pins": {
            "entries": entries,
            "updatedBy": OWNER,
            "updatedAt": at,
        },
    })
}

fn pins_event(event_id: &str, pinned: &[&str], second: u32) -> MatrixTimelineEvent {
    MatrixTimelineEvent::new(
        Some(MatrixEventId::new(event_id).unwrap()),
        Some(MatrixUserId::new(OWNER).unwrap()),
        MatrixEventType::new(ROOM_PINS_EVENT_TYPE).unwrap(),
        Some(String::new()),
        None,
        Some(1_794_816_000_000),
        content(pinned, second),
    )
    .unwrap()
}

async fn take(harness: &Harness) -> NetworkAgentMessages {
    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::ZERO, 50))
        .await
        .unwrap()
}

fn pinned(rooms: &[IpcRoomPins]) -> Vec<Vec<&str>> {
    rooms
        .iter()
        .map(|room| {
            room.pinned
                .iter()
                .map(|pin| pin.event_id.as_str())
                .collect()
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn 同步到的置顶交一次整份_那一句在记录里就带开头_不在的是进来以前的() {
    let said = in_room(
        &chat(
            "$said:matrix.test",
            other(),
            Uuid::now_v7(),
            "周五发版，别合改工作流的。",
            [1; 64],
        ),
        PRIVATE_ROOM,
    );
    let both = ["$said:matrix.test", "$old:matrix.test"];
    let harness = in_private(vec![
        batch("e1", vec![said, pins_event("$p1:matrix.test", &both, 1)]),
        // 从头同步又给了同一版。
        batch("e2", vec![pins_event("$p1:matrix.test", &both, 1)]),
        batch("e3", vec![pins_event("$p2:matrix.test", &[], 2)]),
    ]);
    let first = take(&harness).await.pins;
    assert_eq!(pinned(&first), [both.to_vec()]);
    let [room] = first.as_slice() else {
        panic!("一个房间：{first:?}");
    };
    assert_eq!(room.room_id, PRIVATE_ROOM);
    assert_eq!(room.updated_by, OWNER);
    let known = room.pinned[0].message.as_ref().expect("那一句在消息记录里");
    assert_eq!(known.excerpt, "周五发版，别合改工作流的。");
    assert_eq!(room.pinned[0].unreadable, None);
    assert_eq!(room.pinned[1].message, None, "进来以前的不给内容");
    assert_eq!(
        room.pinned[1].unreadable,
        Some(IpcPinUnreadable::BeforeJoin)
    );

    assert!(take(&harness).await.pins.is_empty(), "同一版只交一次");
    assert_eq!(
        pinned(&take(&harness).await.pins),
        [Vec::<&str>::new()],
        "全取消了也说"
    );
    assert!(take(&harness).await.pins.is_empty());
}

#[tokio::test(start_paused = true)]
async fn 进房间时交了现在的_之后同步到同一版不再交_改了再交() {
    let harness = in_private(vec![
        batch(
            "e1",
            vec![pins_event("$p1:matrix.test", &["$x:matrix.test"], 1)],
        ),
        batch(
            "e2",
            vec![pins_event(
                "$p2:matrix.test",
                &["$y:matrix.test", "$x:matrix.test"],
                2,
            )],
        ),
    ]);
    harness
        .matrix
        .pins
        .lock()
        .unwrap()
        .insert(PRIVATE_ROOM.to_owned(), content(&["$x:matrix.test"], 1));
    let entered = harness
        .gateway
        .current_pins(TOKEN, PRIVATE_ROOM)
        .await
        .expect("进房间时有置顶");
    assert_eq!(pinned(&[entered]), [["$x:matrix.test"]]);

    assert!(take(&harness).await.pins.is_empty(), "进房间时交过了");
    assert_eq!(
        pinned(&take(&harness).await.pins),
        [["$y:matrix.test", "$x:matrix.test"]]
    );

    harness
        .matrix
        .pins
        .lock()
        .unwrap()
        .insert(PRIVATE_ROOM.to_owned(), content(&[], 3));
    assert_eq!(
        harness.gateway.current_pins(TOKEN, PRIVATE_ROOM).await,
        None,
        "全取消了进房间时不给"
    );
    assert_eq!(
        harness
            .gateway
            .current_pins(TOKEN, "!elsewhere:matrix.test")
            .await,
        None
    );
}
