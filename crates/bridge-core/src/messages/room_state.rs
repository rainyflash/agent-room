//! 房间名和“我什么时候加入的”：从一段同步里挑出来，交给本地消息库记下。读消息时据此给每条
//! 消息带上 `roomName`，标出 `beforeJoin`（`specs/agent-reading/design.md` 第 2 步）。
//!
//! 加入时间只认亲眼看到的加入：成员状态从别的变成 `join` 的那条事件。改昵称、换头像也是一条
//! `join` 事件，靠它的上一个成员状态区分；分不清时宁可不记，读消息时就不标。
//!
//! 房间的常驻公告（`room.notice.v1`，`specs/public-rooms/design.md`“常驻公告”）也从这里挑出来，
//! 记下以后进房间、改了以后等消息时交给 Agent。

use agent_room_application::ports::{
    MatrixRoomId, MatrixRoomStatePosition, MatrixRoomSync, MatrixRoomSyncKind, MatrixSyncBatch,
    MatrixTimelineEvent,
};
use agent_room_domain::room_notices::RoomNoticeText;
use chrono::DateTime;
use serde_json::Value;

/// 房间公告的状态事件类型，由控制面以应用服务写。
pub const ROOM_NOTICE_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.room.notice.v1";

/// 一个房间这段同步里的变化。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomStateChange {
    pub room_id: MatrixRoomId,
    /// 房间名变了；没变就没有。
    pub name: Option<RoomName>,
    /// 自己的成员状态变了。
    pub membership: Option<OwnMembership>,
    /// 这段同步里看到的公告（最新的那一版）；没看到就没有。
    pub notice: Option<RoomNotice>,
}

/// 房间的常驻公告，某一版。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomNotice {
    /// 公告正文；撤掉了是 `None`。
    pub text: Option<String>,
    /// 改的人在 Matrix 上的 ID。
    pub updated_by: String,
    /// 改的时间（Unix 毫秒），按它分新旧。
    pub updated_at_ms: i64,
}

/// 房间名变成了什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomName {
    Named(String),
    /// 名字被清掉了。
    Unnamed,
}

/// 自己在房间里的成员状态变化。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnMembership {
    /// 真正加入（之前不是 `join`）：服务器收到加入事件的时间（Unix 毫秒）。
    Joined { at_ms: i64 },
    /// 离开或被移出：之后再加入时重新记。
    Left,
}

/// 从一段同步里挑出每个房间的名字和自己成员状态的变化；没有变化的房间不出现。
pub fn room_state_changes(batch: &MatrixSyncBatch, own_user_id: &str) -> Vec<RoomStateChange> {
    batch
        .rooms()
        .iter()
        .filter_map(|room| room_change(room, own_user_id))
        .collect()
}

fn room_change(room: &MatrixRoomSync, own_user_id: &str) -> Option<RoomStateChange> {
    let mut change = RoomStateChange {
        room_id: room.room_id().clone(),
        name: None,
        membership: None,
        notice: None,
    };
    match room.kind() {
        MatrixRoomSyncKind::Joined => {}
        MatrixRoomSyncKind::Left => change.membership = Some(OwnMembership::Left),
        MatrixRoomSyncKind::Invited | MatrixRoomSyncKind::Knocked => return None,
    }
    // 状态在时间线之前时先看状态再看时间线，之后的反过来：后看到的是更新的。
    let events: Vec<&MatrixTimelineEvent> = match room.state_position() {
        MatrixRoomStatePosition::BeforeTimeline => {
            room.state().iter().chain(room.timeline()).collect()
        }
        MatrixRoomStatePosition::AfterTimeline => {
            room.timeline().iter().chain(room.state()).collect()
        }
    };
    for event in events {
        match event.event_type().as_str() {
            "m.room.name" if event.state_key() == Some("") => {
                change.name = Some(room_name(event.content()));
            }
            "m.room.member" if event.state_key() == Some(own_user_id) => {
                if let Some(membership) = own_membership(event) {
                    change.membership = Some(membership);
                }
            }
            ROOM_NOTICE_EVENT_TYPE if event.state_key() == Some("") => {
                if let Some(notice) = room_notice(event.content()) {
                    change.notice = Some(notice);
                }
            }
            _ => {}
        }
    }
    (change.name.is_some() || change.membership.is_some() || change.notice.is_some())
        .then_some(change)
}

/// 读一条公告状态事件的内容；读不出来的不算一版。调用方只认状态事件（`state_key` 为空）：私人房间
/// 和公开房间的 `state_default` 是管理员那一级，只有应用服务写得进，所以不另外核对发送者。网关
/// 进房间时直接读房间状态，也用它。
#[must_use]
pub fn room_notice(content: &Value) -> Option<RoomNotice> {
    if content.get("eventType").and_then(Value::as_str) != Some(ROOM_NOTICE_EVENT_TYPE) {
        return None;
    }
    let text = RoomNoticeText::parse(content.get("text")?.as_str()?).ok()?;
    let updated_by = content.get("updatedBy")?.as_str()?;
    if !updated_by.starts_with('@') || updated_by.len() > 255 {
        return None;
    }
    let updated_at_ms = DateTime::parse_from_rfc3339(content.get("updatedAt")?.as_str()?)
        .ok()?
        .timestamp_millis();
    Some(RoomNotice {
        text: (!text.is_removal()).then(|| text.as_str().to_owned()),
        updated_by: updated_by.to_owned(),
        updated_at_ms,
    })
}

fn room_name(content: &Value) -> RoomName {
    content
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map_or(RoomName::Unnamed, |name| RoomName::Named(name.to_owned()))
}

fn own_membership(event: &MatrixTimelineEvent) -> Option<OwnMembership> {
    match event.content().get("membership").and_then(Value::as_str)? {
        // 上一个状态也是 join 的是改昵称、换头像，不算加入。
        "join" if event.previous_membership() != Some("join") => event
            .origin_server_timestamp()
            .and_then(|at| i64::try_from(at).ok())
            .map(|at_ms| OwnMembership::Joined { at_ms }),
        "leave" | "ban" => Some(OwnMembership::Left),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use agent_room_application::ports::{
        MatrixEventType, MatrixRoomId, MatrixRoomSync, MatrixRoomSyncKind, MatrixSyncBatch,
        MatrixSyncToken, MatrixTimelineEvent,
    };
    use serde_json::{Value, json};

    use super::{OwnMembership, RoomName, RoomStateChange, room_state_changes};

    const ME: &str = "@scout:matrix.test";
    const ROOM: &str = "!room:matrix.test";

    fn state(event_type: &str, key: &str, content: Value, at: u64) -> MatrixTimelineEvent {
        MatrixTimelineEvent::new(
            None,
            None,
            MatrixEventType::new(event_type).unwrap(),
            Some(key.to_owned()),
            None,
            Some(at),
            content,
        )
        .unwrap()
    }

    fn member(membership: &str, previous: Option<&str>, at: u64) -> MatrixTimelineEvent {
        state("m.room.member", ME, json!({"membership": membership}), at)
            .with_previous_membership(previous.map(str::to_owned))
    }

    fn batch(kind: MatrixRoomSyncKind, timeline: Vec<MatrixTimelineEvent>) -> MatrixSyncBatch {
        MatrixSyncBatch::new(
            MatrixSyncToken::new("s1").unwrap(),
            vec![MatrixRoomSync::new(
                MatrixRoomId::new(ROOM).unwrap(),
                kind,
                false,
                None,
                timeline,
                Vec::new(),
            )],
        )
    }

    fn change(name: Option<RoomName>, membership: Option<OwnMembership>) -> RoomStateChange {
        RoomStateChange {
            room_id: MatrixRoomId::new(ROOM).unwrap(),
            name,
            membership,
            notice: None,
        }
    }

    fn notice_state(text: &str, updated_at: &str, at: u64) -> MatrixTimelineEvent {
        state(
            super::ROOM_NOTICE_EVENT_TYPE,
            "",
            json!({
                "schemaVersion": "1.0",
                "eventType": super::ROOM_NOTICE_EVENT_TYPE,
                "text": text,
                "updatedBy": "@owner:matrix.test",
                "updatedAt": updated_at,
            }),
            at,
        )
    }

    #[test]
    fn 公告认最新的一版_撤掉的没有正文_读不出来和不是状态的不算() {
        let notices = batch(
            MatrixRoomSyncKind::Joined,
            vec![
                notice_state(
                    " 只在被点名时说话。\n回复要短。 ",
                    "2026-11-03T08:00:00.000Z",
                    1_000,
                ),
                notice_state("改过的", "2026-11-03T09:00:00.500+08:00", 2_000),
            ],
        );
        let [change] = room_state_changes(&notices, ME).try_into().unwrap();
        assert_eq!(
            change.notice,
            Some(super::RoomNotice {
                text: Some("改过的".to_owned()),
                updated_by: "@owner:matrix.test".to_owned(),
                updated_at_ms: 1_793_667_600_500,
            })
        );
        let removed = batch(
            MatrixRoomSyncKind::Joined,
            vec![notice_state("  ", "2026-11-03T10:00:00Z", 3_000)],
        );
        let [change] = room_state_changes(&removed, ME).try_into().unwrap();
        assert_eq!(change.notice.and_then(|notice| notice.text), None);

        let ignored = batch(
            MatrixRoomSyncKind::Joined,
            vec![
                // 回车不收，和网页、控制面一样。
                notice_state("一\r二", "2026-11-03T10:00:00Z", 4_000),
                notice_state("时间不对", "yesterday", 5_000),
                state(super::ROOM_NOTICE_EVENT_TYPE, "other", json!({}), 6_000),
                // 撤回以后内容是空的。
                state(super::ROOM_NOTICE_EVENT_TYPE, "", json!({}), 7_000),
            ],
        );
        assert!(room_state_changes(&ignored, ME).is_empty());
    }

    #[test]
    fn 只认真正的加入_改昵称不算() {
        let joined = batch(
            MatrixRoomSyncKind::Joined,
            vec![member("join", Some("invite"), 1_000)],
        );
        assert_eq!(
            room_state_changes(&joined, ME),
            [change(None, Some(OwnMembership::Joined { at_ms: 1_000 }))]
        );
        let first = batch(
            MatrixRoomSyncKind::Joined,
            vec![member("join", None, 2_000)],
        );
        assert_eq!(
            room_state_changes(&first, ME),
            [change(None, Some(OwnMembership::Joined { at_ms: 2_000 }))],
            "没有上一个状态的是第一次加入"
        );
        let renamed = batch(
            MatrixRoomSyncKind::Joined,
            vec![member("join", Some("join"), 3_000)],
        );
        assert!(room_state_changes(&renamed, ME).is_empty());
        // 别人的成员事件不看。
        let other = batch(
            MatrixRoomSyncKind::Joined,
            vec![state(
                "m.room.member",
                "@ada:matrix.test",
                json!({"membership": "join"}),
                4_000,
            )],
        );
        assert!(room_state_changes(&other, ME).is_empty());
    }

    #[test]
    fn 离开和被移出都清掉_房间名变了记下新名字() {
        for left in [
            member("leave", Some("join"), 5_000),
            member("ban", Some("join"), 5_000),
        ] {
            let batch = batch(MatrixRoomSyncKind::Joined, vec![left]);
            assert_eq!(
                room_state_changes(&batch, ME),
                [change(None, Some(OwnMembership::Left))]
            );
        }
        let gone = batch(MatrixRoomSyncKind::Left, Vec::new());
        assert_eq!(
            room_state_changes(&gone, ME),
            [change(None, Some(OwnMembership::Left))]
        );

        let named = batch(
            MatrixRoomSyncKind::Joined,
            vec![
                state("m.room.name", "", json!({"name": "  项目室 "}), 6_000),
                state("m.room.name", "", json!({"name": "新项目室"}), 7_000),
            ],
        );
        assert_eq!(
            room_state_changes(&named, ME),
            [change(Some(RoomName::Named("新项目室".to_owned())), None)],
            "后到的名字为准"
        );
        let cleared = batch(
            MatrixRoomSyncKind::Joined,
            vec![state("m.room.name", "", json!({}), 8_000)],
        );
        assert_eq!(
            room_state_changes(&cleared, ME),
            [change(Some(RoomName::Unnamed), None)]
        );
    }
}
