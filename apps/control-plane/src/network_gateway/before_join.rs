//! 凭口令进的私人房间里，加入之前的消息它拿不到房间密钥，解不开（`specs/agent-reading/design.md`
//! 的 `gaps`，原因 `undecryptable_before_join`）。维护者 2026-10-05 定：只告诉它这一段解不开，不给它
//! 加入之前的房间密钥（那样服务器也读得到加入之前的消息）。
//!
//! 加入时间只认这一批里亲眼看到的加入事件，和本机记加入时间是同一套（`room_state_changes`，改昵称、
//! 换头像不算）。刚进的房间第一次同步带回最近一段，里面服务器早于加入收到、又解不开的事件就是这一段；
//! 之后的同步只有新到的，看不到加入事件，也就不会再报。
//!
//! 报的时候带上这次加入的时间：存储重建后从头同步会再看到同一次加入，收件箱认得出是同一次，不再说；
//! 被移出以后又凭口令进来是更晚的一次，离开那段同样解不开，再说一次。

use std::collections::HashMap;

use agent_room_application::ports::{
    MatrixRoomId, MatrixRoomSyncKind, MatrixSyncBatch, MatrixTimelineEvent,
    NetworkAgentBeforeJoinGap,
};
use agent_room_bridge_core::messages::{OwnMembership, is_undecryptable, room_state_changes};
use agent_room_domain::time::UtcMillis;

/// 这一批里看到它加入、加入之前又有解不开的加密消息的房间，和这次加入的时间。
pub(super) fn undecryptable_before_join(
    batch: &MatrixSyncBatch,
    own_user_id: &str,
) -> Vec<NetworkAgentBeforeJoinGap> {
    let joined: HashMap<MatrixRoomId, UtcMillis> = room_state_changes(batch, own_user_id)
        .into_iter()
        .filter_map(|change| match change.membership? {
            OwnMembership::Joined { at_ms } => Some((change.room_id, UtcMillis::new(at_ms).ok()?)),
            OwnMembership::Left => None,
        })
        .collect();
    batch
        .rooms()
        .iter()
        .filter(|room| room.kind() == MatrixRoomSyncKind::Joined)
        .filter_map(|room| {
            let joined_at = *joined.get(room.room_id())?;
            room.timeline()
                .iter()
                .any(|event| is_undecryptable(event) && sent_before(event, joined_at))
                .then(|| NetworkAgentBeforeJoinGap {
                    room_id: room.room_id().clone(),
                    joined_at,
                })
        })
        .collect()
}

/// 服务器早于这个时间收到的。
fn sent_before(event: &MatrixTimelineEvent, at: UtcMillis) -> bool {
    event
        .origin_server_timestamp()
        .and_then(|sent| i64::try_from(sent).ok())
        .is_some_and(|sent| sent < at.value())
}

#[cfg(test)]
mod tests {
    use agent_room_application::ports::{
        MatrixEventId, MatrixEventType, MatrixRoomId, MatrixRoomSync, MatrixRoomSyncKind,
        MatrixSyncBatch, MatrixSyncToken, MatrixTimelineEvent, MatrixUserId,
    };
    use serde_json::{Value, json};

    use super::undecryptable_before_join;

    const ME: &str = "@_agent_scout:matrix.test";
    const ROOM: &str = "!private:matrix.test";
    const JOINED_AT: u64 = 1_758_600_000_500;

    fn timeline_event(
        event_id: &str,
        event_type: &str,
        state_key: Option<&str>,
        content: Value,
        at: u64,
    ) -> MatrixTimelineEvent {
        MatrixTimelineEvent::new(
            Some(MatrixEventId::new(event_id).unwrap()),
            Some(MatrixUserId::new("@owner:matrix.test").unwrap()),
            MatrixEventType::new(event_type).unwrap(),
            state_key.map(str::to_owned),
            None,
            Some(at),
            content,
        )
        .unwrap()
    }

    /// 解不开、原样留下的加密事件。
    fn locked(event_id: &str, at: u64) -> MatrixTimelineEvent {
        timeline_event(
            event_id,
            "m.room.encrypted",
            None,
            json!({"algorithm": "m.megolm.v1.aes-sha2", "ciphertext": "AwgA", "session_id": "s"}),
            at,
        )
    }

    /// 自己的成员事件；`previous` 是上一个成员状态。
    fn membership(event_id: &str, previous: Option<&str>, at: u64) -> MatrixTimelineEvent {
        timeline_event(
            event_id,
            "m.room.member",
            Some(ME),
            json!({"membership": "join", "displayname": "Scout"}),
            at,
        )
        .with_previous_membership(previous.map(str::to_owned))
    }

    fn sync(kind: MatrixRoomSyncKind, timeline: Vec<MatrixTimelineEvent>) -> MatrixSyncBatch {
        MatrixSyncBatch::new(
            MatrixSyncToken::new("s1").unwrap(),
            vec![MatrixRoomSync::new(
                MatrixRoomId::new(ROOM).unwrap(),
                kind,
                true,
                None,
                timeline,
                Vec::new(),
            )],
        )
    }

    /// 自己离开或被移出。
    fn left(event_id: &str, at: u64) -> MatrixTimelineEvent {
        timeline_event(
            event_id,
            "m.room.member",
            Some(ME),
            json!({"membership": "leave"}),
            at,
        )
        .with_previous_membership(Some("join".to_owned()))
    }

    fn rooms(batch: &MatrixSyncBatch) -> Vec<String> {
        undecryptable_before_join(batch, ME)
            .iter()
            .map(|gap| gap.room_id.as_str().to_owned())
            .collect()
    }

    fn joined_at(batch: &MatrixSyncBatch) -> Vec<i64> {
        undecryptable_before_join(batch, ME)
            .iter()
            .map(|gap| gap.joined_at.value())
            .collect()
    }

    #[test]
    fn 看到自己加入_之前有解不开的就报这个房间和加入时间() {
        let batch = sync(
            MatrixRoomSyncKind::Joined,
            vec![
                locked("$early:matrix.test", JOINED_AT - 400),
                membership("$join:matrix.test", Some("invite"), JOINED_AT),
            ],
        );
        assert_eq!(rooms(&batch), [ROOM]);
        assert_eq!(joined_at(&batch), [i64::try_from(JOINED_AT).unwrap()]);
    }

    #[test]
    fn 离开以后又进来_按后一次加入算() {
        let rejoined = JOINED_AT + 3_000;
        let batch = sync(
            MatrixRoomSyncKind::Joined,
            vec![
                membership("$join:matrix.test", Some("invite"), JOINED_AT),
                left("$kicked:matrix.test", JOINED_AT + 1_000),
                locked("$while-away:matrix.test", JOINED_AT + 2_000),
                membership("$rejoin:matrix.test", Some("leave"), rejoined),
            ],
        );
        assert_eq!(rooms(&batch), [ROOM], "不在的时候别人说的也解不开");
        assert_eq!(joined_at(&batch), [i64::try_from(rejoined).unwrap()]);
    }

    #[test]
    fn 加入之后才解不开的_或者加入之前什么都没说的_都不报() {
        let after = sync(
            MatrixRoomSyncKind::Joined,
            vec![
                membership("$join:matrix.test", None, JOINED_AT),
                locked("$late:matrix.test", JOINED_AT + 10),
            ],
        );
        assert!(rooms(&after).is_empty(), "加入以后解不开的是别的问题");
        let quiet = sync(
            MatrixRoomSyncKind::Joined,
            vec![membership("$join:matrix.test", None, JOINED_AT)],
        );
        assert!(rooms(&quiet).is_empty());
    }

    #[test]
    fn 这一批没看到自己加入_或者只是改了昵称_都不报() {
        let earlier = sync(
            MatrixRoomSyncKind::Joined,
            vec![locked("$old:matrix.test", JOINED_AT - 400)],
        );
        assert!(rooms(&earlier).is_empty(), "已经在跟的房间不再报");
        let renamed = sync(
            MatrixRoomSyncKind::Joined,
            vec![
                locked("$old:matrix.test", JOINED_AT - 400),
                membership("$rename:matrix.test", Some("join"), JOINED_AT),
            ],
        );
        assert!(rooms(&renamed).is_empty(), "改昵称不是加入");
    }

    #[test]
    fn 已经离开的房间不报() {
        let left = sync(
            MatrixRoomSyncKind::Left,
            vec![
                locked("$early:matrix.test", JOINED_AT - 400),
                membership("$join:matrix.test", Some("invite"), JOINED_AT),
            ],
        );
        assert!(rooms(&left).is_empty());
    }
}
