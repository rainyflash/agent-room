//! 凭口令进的私人房间里，加入之前的消息解不开：交出这个房间下一条消息时用 `gaps` 告诉 Agent
//! （`undecryptable_before_join`），每次加入只说一次。

use std::time::Duration;

use agent_room_application::ports::{
    MatrixEventId, MatrixEventType, MatrixRoomId, MatrixRoomSync, MatrixRoomSyncKind,
    MatrixSyncBatch, MatrixSyncToken, MatrixTimelineEvent, MatrixUserId,
};
use agent_room_bridge_ipc::IpcTimelineGap;
use agent_room_domain::time::UtcMillis;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    Harness, NetworkAgentMessaging, OWN_AGENT, PRIVATE_ROOM, TOKEN, chat_with, everything,
    harness_in, in_room, matrix_user, other,
};
use crate::network_gateway::NetworkAgentMessages;

/// 加入事件的服务器时间；之前的解不开，之后的照常。
const JOINED_AT: u64 = 1_758_599_500_000;

/// 进过加密房间、在私人房间里的网络 Agent：同步都走加密客户端，按顺序给这几批。
fn joined_private(batches: Vec<MatrixSyncBatch>) -> Harness {
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

/// 私人房间的一段同步。刚进来的那一段带着加入事件，只是房间里最近的一截（`limited`）；之后的都是
/// 新到的，没截断。
fn private_batch(next: &str, events: Vec<MatrixTimelineEvent>) -> MatrixSyncBatch {
    let limited = events
        .iter()
        .any(|event| event.event_id() == joined().event_id());
    MatrixSyncBatch::new(
        MatrixSyncToken::new(next).unwrap(),
        vec![MatrixRoomSync::new(
            MatrixRoomId::new(PRIVATE_ROOM).unwrap(),
            MatrixRoomSyncKind::Joined,
            limited,
            None,
            events,
            Vec::new(),
        )],
    )
}

/// 加入之前别人发的、它解不开的一条。
fn locked(event_id: &str, at: u64) -> MatrixTimelineEvent {
    MatrixTimelineEvent::new(
        Some(MatrixEventId::new(event_id).unwrap()),
        Some(MatrixUserId::new("@owner:matrix.test").unwrap()),
        MatrixEventType::new("m.room.encrypted").unwrap(),
        None,
        None,
        Some(at),
        json!({"algorithm": "m.megolm.v1.aes-sha2", "ciphertext": "AwgA", "session_id": "s"}),
    )
    .unwrap()
}

/// 它自己的成员事件：`membership` 是这次的，`previous` 是上一个。
fn own_member(
    event_id: &str,
    membership: &str,
    previous: Option<&str>,
    at: u64,
) -> MatrixTimelineEvent {
    let me = matrix_user(OWN_AGENT);
    MatrixTimelineEvent::new(
        Some(MatrixEventId::new(event_id).unwrap()),
        Some(MatrixUserId::new(me.clone()).unwrap()),
        MatrixEventType::new("m.room.member").unwrap(),
        Some(me),
        None,
        Some(at),
        json!({"membership": membership, "displayname": "Scout"}),
    )
    .unwrap()
    .with_previous_membership(previous.map(str::to_owned))
}

/// 它自己凭口令进来的那条成员事件。
fn joined() -> MatrixTimelineEvent {
    own_member("$join:matrix.test", "join", None, JOINED_AT)
}

/// 加入以后别人在私人房间里说的一句，正文就是事件 ID。
fn said(event_id: &str) -> MatrixTimelineEvent {
    in_room(
        &chat_with(event_id, other(), Uuid::now_v7(), event_id, &[], None),
        PRIVATE_ROOM,
    )
}

fn before_join(before: &str) -> IpcTimelineGap {
    IpcTimelineGap {
        room_id: PRIVATE_ROOM.to_owned(),
        after_event_id: None,
        before_event_id: before.to_owned(),
        reason: "undecryptable_before_join".to_owned(),
    }
}

fn event_ids(messages: &[Value]) -> Vec<&str> {
    messages
        .iter()
        .map(|message| message["eventId"].as_str().unwrap())
        .collect()
}

/// 有什么给什么；交出来的不确认。
async fn take(harness: &Harness) -> NetworkAgentMessages {
    harness
        .gateway
        .wait_for_messages(TOKEN, everything(Duration::ZERO, 50))
        .await
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn 进来时只有解不开的旧消息_等到下一条才说_确认之前每次交都说_之后不再说() {
    let harness = joined_private(vec![
        private_batch(
            "e1",
            vec![
                locked("$early1:matrix.test", JOINED_AT - 60_000),
                locked("$early2:matrix.test", JOINED_AT - 1_000),
                joined(),
            ],
        ),
        private_batch("e2", vec![said("$hello:matrix.test")]),
    ]);

    let first = take(&harness).await;
    assert!(first.messages.is_empty(), "解不开的不当消息交");
    assert!(first.gaps.is_empty(), "还没有能挂的消息");

    let page = take(&harness).await;
    assert_eq!(event_ids(&page.messages), ["$hello:matrix.test"]);
    assert_eq!(page.gaps, [before_join("$hello:matrix.test")]);
    let again = take(&harness).await;
    assert_eq!(again.gaps, page.gaps, "没确认就再说一遍");

    harness
        .gateway
        .acknowledge(TOKEN, "$hello:matrix.test", None)
        .await
        .unwrap();
    harness
        .encrypted
        .batches
        .lock()
        .unwrap()
        .push_back(Ok(private_batch("e3", vec![said("$next:matrix.test")])));
    let later = take(&harness).await;
    assert_eq!(event_ids(&later.messages), ["$next:matrix.test"]);
    assert!(later.gaps.is_empty(), "这次加入只说一次");
}

#[tokio::test(start_paused = true)]
async fn 被移出以后又凭口令进来_不在的那段再说一次() {
    let harness = joined_private(vec![
        private_batch(
            "e1",
            vec![
                locked("$early:matrix.test", JOINED_AT - 1_000),
                joined(),
                said("$hello:matrix.test"),
            ],
        ),
        private_batch(
            "e2",
            vec![
                own_member(
                    "$kicked:matrix.test",
                    "leave",
                    Some("join"),
                    JOINED_AT + 1_000,
                ),
                locked("$while-away:matrix.test", JOINED_AT + 2_000),
                own_member(
                    "$rejoin:matrix.test",
                    "join",
                    Some("leave"),
                    JOINED_AT + 3_000,
                ),
                said("$back:matrix.test"),
            ],
        ),
    ]);

    let first = take(&harness).await;
    assert_eq!(first.gaps, [before_join("$hello:matrix.test")]);
    harness
        .gateway
        .acknowledge(TOKEN, "$hello:matrix.test", None)
        .await
        .unwrap();

    let second = take(&harness).await;
    assert_eq!(event_ids(&second.messages), ["$back:matrix.test"]);
    assert_eq!(
        second.gaps,
        [before_join("$back:matrix.test")],
        "不在的时候别人说的也解不开"
    );
}

#[tokio::test(start_paused = true)]
async fn 进来以后马上有人说话_同一批就挂在那条上() {
    let harness = joined_private(vec![private_batch(
        "e1",
        vec![
            locked("$early:matrix.test", JOINED_AT - 1_000),
            joined(),
            said("$hello:matrix.test"),
        ],
    )]);

    let page = take(&harness).await;

    assert_eq!(event_ids(&page.messages), ["$hello:matrix.test"]);
    assert_eq!(page.gaps, [before_join("$hello:matrix.test")]);
}

#[tokio::test(start_paused = true)]
async fn 进来之前没人说过话_或者解不开的是进来以后的_都不说() {
    let harness = joined_private(vec![
        private_batch("e1", vec![joined(), said("$hello:matrix.test")]),
        private_batch(
            "e2",
            vec![
                locked("$late:matrix.test", JOINED_AT + 60_000),
                said("$next:matrix.test"),
            ],
        ),
    ]);

    let first = take(&harness).await;
    assert_eq!(event_ids(&first.messages), ["$hello:matrix.test"]);
    assert!(first.gaps.is_empty());
    let second = take(&harness).await;
    assert_eq!(
        event_ids(&second.messages),
        ["$hello:matrix.test", "$next:matrix.test"]
    );
    assert!(
        second.gaps.is_empty(),
        "加入以后解不开的是别的问题，不算这一段"
    );
}
