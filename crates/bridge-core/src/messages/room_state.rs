//! 房间名和“我什么时候加入的”：从一段同步里挑出来，交给本地消息库记下。读消息时据此给每条
//! 消息带上 `roomName`，标出 `beforeJoin`（`specs/agent-reading/design.md` 第 2 步）。
//!
//! 加入时间只认亲眼看到的加入：成员状态从别的变成 `join` 的那条事件。改昵称、换头像也是一条
//! `join` 事件，靠它的上一个成员状态区分；分不清时宁可不记，读消息时就不标。
//!
//! 房间的常驻公告（`room.notice.v1`，`specs/public-rooms/design.md`“常驻公告”）也从这里挑出来，
//! 记下以后进房间、改了以后等消息时交给 Agent。子房间里还有一份父房间须知的副本
//! （`room.parent_notice.v1`，`specs/redesign/brief-and-pins.md`“子房间里父房间的须知也算”）：
//! 进子房间的 Agent 不一定在父房间里，读不到父房间的状态，所以控制面在子房间里写一份，一样交给它。
//! 房间的置顶（`m.room.pinned_events` 加我们的字段，`specs/redesign/brief-and-pins.md`“Agent 读置顶”）
//! 也一样：记下以后进房间、改了以后等消息时交整份。

use agent_room_application::ports::{
    MatrixEventId, MatrixRoomId, MatrixRoomStatePosition, MatrixRoomSync, MatrixRoomSyncKind,
    MatrixSyncBatch, MatrixTimelineEvent,
};
use agent_room_domain::{room_notices::RoomNoticeText, room_pins::MAX_ROOM_PINS};
use chrono::DateTime;
use serde_json::Value;

/// 房间公告的状态事件类型，由控制面以应用服务写。
pub const ROOM_NOTICE_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.room.notice.v1";
/// 子房间里父房间须知的副本，由控制面以应用服务写；内容和公告一样，多一个 `parentRoomId`。
pub const ROOM_PARENT_NOTICE_EVENT_TYPE: &str =
    "io.github.rainyflash.agentroom.room.parent_notice.v1";
/// Matrix 标准的置顶状态，由控制面以应用服务写。
pub const ROOM_PINS_EVENT_TYPE: &str = "m.room.pinned_events";
/// 置顶状态里我们自己的那一段：每一句谁、什么时候置顶的，最近一次谁、什么时候改的。
const ROOM_PINS_FIELD: &str = "io.github.rainyflash.agentroom.pins";

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
    /// 这段同步里看到的父房间须知的副本（最新的那一版）；不是子房间、没看到就没有。
    pub parent_notice: Option<ParentRoomNotice>,
    /// 这段同步里看到的置顶（最新的那一版）；没看到就没有。
    pub pins: Option<RoomPinsState>,
}

/// 一间房此刻的置顶，某一版。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomPinsState {
    /// 新的在最前面；全取消了是空的。
    pub entries: Vec<RoomPinEntry>,
    /// 最近一次改置顶的人在 Matrix 上的 ID。
    pub updated_by: String,
    /// 最近一次改的时间（Unix 毫秒），按它分新旧。
    pub updated_at_ms: i64,
}

/// 置顶的一句：服务器上只记事件 ID、谁、什么时候。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomPinEntry {
    pub event_id: MatrixEventId,
    pub pinned_by: String,
    pub pinned_at_ms: i64,
}

/// 子房间里父房间须知的副本，某一版。交给 Agent 时按父房间的房间号交，和这一间自己的分开。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentRoomNotice {
    pub parent_room_id: MatrixRoomId,
    /// 父房间那一版须知；撤掉了正文是 `None`。
    pub notice: RoomNotice,
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
        parent_notice: None,
        pins: None,
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
            ROOM_PARENT_NOTICE_EVENT_TYPE if event.state_key() == Some("") => {
                if let Some(copy) = room_parent_notice(event.content()) {
                    change.parent_notice = Some(copy);
                }
            }
            ROOM_PINS_EVENT_TYPE if event.state_key() == Some("") => {
                if let Some(pins) = room_pins(event.content()) {
                    change.pins = Some(pins);
                }
            }
            _ => {}
        }
    }
    (change.name.is_some()
        || change.membership.is_some()
        || change.notice.is_some()
        || change.parent_notice.is_some()
        || change.pins.is_some())
    .then_some(change)
}

/// 读一条公告状态事件的内容；读不出来的不算一版。调用方只认状态事件（`state_key` 为空）：私人房间
/// 和公开房间的 `state_default` 是管理员那一级，只有应用服务写得进，所以不另外核对发送者。网关
/// 进房间时直接读房间状态，也用它。
#[must_use]
pub fn room_notice(content: &Value) -> Option<RoomNotice> {
    notice_of(content, ROOM_NOTICE_EVENT_TYPE)
}

/// 读子房间里父房间须知的副本；读不出来（包括父房间号不对）的不算一版。和公告一样只认应用服务写的
/// 状态事件，网关进房间时也直接读它。
#[must_use]
pub fn room_parent_notice(content: &Value) -> Option<ParentRoomNotice> {
    let notice = notice_of(content, ROOM_PARENT_NOTICE_EVENT_TYPE)?;
    let parent_room_id =
        MatrixRoomId::new(content.get("parentRoomId")?.as_str()?.to_owned()).ok()?;
    Some(ParentRoomNotice {
        parent_room_id,
        notice,
    })
}

/// 读置顶状态。只认我们写的（带 [`ROOM_PINS_FIELD`]，按它的 `updatedAt` 分新旧）：别的客户端写的
/// 不知道是谁、什么时候改的，不算一版。只留标准的 `pinned` 里也有的那几句，读不懂的一条跳过，最多
/// [`MAX_ROOM_PINS`] 条。网关进房间时也直接读它。
#[must_use]
pub fn room_pins(content: &Value) -> Option<RoomPinsState> {
    let pinned: Vec<&str> = content
        .get("pinned")?
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let field = content.get(ROOM_PINS_FIELD)?;
    let updated_by = matrix_user(field.get("updatedBy")?)?;
    let updated_at_ms = timestamp_ms(field.get("updatedAt")?)?;
    let mut entries: Vec<RoomPinEntry> = Vec::new();
    for entry in field.get("entries")?.as_array()? {
        let Some(entry) = pin_entry(entry) else {
            continue;
        };
        let known = entries.iter().any(|kept| kept.event_id == entry.event_id);
        if !known && pinned.contains(&entry.event_id.as_str()) && entries.len() < MAX_ROOM_PINS {
            entries.push(entry);
        }
    }
    Some(RoomPinsState {
        entries,
        updated_by,
        updated_at_ms,
    })
}

fn pin_entry(entry: &Value) -> Option<RoomPinEntry> {
    Some(RoomPinEntry {
        event_id: MatrixEventId::new(entry.get("eventId")?.as_str()?.to_owned()).ok()?,
        pinned_by: matrix_user(entry.get("pinnedBy")?)?,
        pinned_at_ms: timestamp_ms(entry.get("pinnedAt")?)?,
    })
}

fn matrix_user(value: &Value) -> Option<String> {
    let user = value.as_str()?;
    (user.starts_with('@') && user.len() <= 255).then(|| user.to_owned())
}

fn timestamp_ms(value: &Value) -> Option<i64> {
    DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|time| time.timestamp_millis())
}

fn notice_of(content: &Value, event_type: &str) -> Option<RoomNotice> {
    if content.get("eventType").and_then(Value::as_str) != Some(event_type) {
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
            parent_notice: None,
            pins: None,
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
    fn 父房间须知的副本按父房间号记_撤掉的没有正文_缺父房间号的不算() {
        const PARENT: &str = "!project:matrix.test";
        let copy = |text: &str, updated_at: &str, parent: Option<&str>, at: u64| {
            let mut content = json!({
                "schemaVersion": "1.0",
                "eventType": super::ROOM_PARENT_NOTICE_EVENT_TYPE,
                "text": text,
                "updatedBy": "@owner:matrix.test",
                "updatedAt": updated_at,
            });
            if let Some(parent) = parent {
                content["parentRoomId"] = json!(parent);
            }
            state(super::ROOM_PARENT_NOTICE_EVENT_TYPE, "", content, at)
        };
        let copies = batch(
            MatrixRoomSyncKind::Joined,
            vec![
                copy(
                    "先说要做什么。",
                    "2026-11-16T08:00:00.000Z",
                    Some(PARENT),
                    1_000,
                ),
                copy(
                    "做完告诉主人。",
                    "2026-11-16T09:00:00.000Z",
                    Some(PARENT),
                    2_000,
                ),
            ],
        );
        let [change] = room_state_changes(&copies, ME).try_into().unwrap();
        assert_eq!(change.notice, None, "副本不是这一间自己的须知");
        assert_eq!(
            change.parent_notice,
            Some(super::ParentRoomNotice {
                parent_room_id: MatrixRoomId::new(PARENT).unwrap(),
                notice: super::RoomNotice {
                    text: Some("做完告诉主人。".to_owned()),
                    updated_by: "@owner:matrix.test".to_owned(),
                    updated_at_ms: 1_794_819_600_000,
                },
            })
        );
        let removed = batch(
            MatrixRoomSyncKind::Joined,
            vec![copy("", "2026-11-16T10:00:00Z", Some(PARENT), 3_000)],
        );
        let [change] = room_state_changes(&removed, ME).try_into().unwrap();
        assert_eq!(
            change.parent_notice.map(|copy| copy.notice.text),
            Some(None)
        );

        let ignored = batch(
            MatrixRoomSyncKind::Joined,
            vec![
                copy("没有父房间号", "2026-11-16T10:00:00Z", None, 4_000),
                copy(
                    "父房间号不对",
                    "2026-11-16T10:00:00Z",
                    Some("project"),
                    5_000,
                ),
                // 类型对不上的内容（把公告写进了副本的类型）不算。
                state(
                    super::ROOM_PARENT_NOTICE_EVENT_TYPE,
                    "",
                    json!({
                        "schemaVersion": "1.0",
                        "eventType": super::ROOM_NOTICE_EVENT_TYPE,
                        "text": "公告",
                        "updatedBy": "@owner:matrix.test",
                        "updatedAt": "2026-11-16T10:00:00Z",
                        "parentRoomId": PARENT,
                    }),
                    6_000,
                ),
                state(
                    super::ROOM_PARENT_NOTICE_EVENT_TYPE,
                    "other",
                    json!({
                        "schemaVersion": "1.0",
                        "eventType": super::ROOM_PARENT_NOTICE_EVENT_TYPE,
                        "text": "不是空的状态键",
                        "updatedBy": "@owner:matrix.test",
                        "updatedAt": "2026-11-16T10:00:00Z",
                        "parentRoomId": PARENT,
                    }),
                    7_000,
                ),
            ],
        );
        assert!(room_state_changes(&ignored, ME).is_empty());
    }

    #[test]
    fn 置顶只认我们写的_只留标准列表里也有的_全取消了是空的() {
        let pins = |pinned: Value, entries: Value, updated_at: &str, at: u64| {
            state(
                super::ROOM_PINS_EVENT_TYPE,
                "",
                json!({
                    "pinned": pinned,
                    "io.github.rainyflash.agentroom.pins": {
                        "schemaVersion": "1.0",
                        "entries": entries,
                        "updatedBy": "@owner:matrix.test",
                        "updatedAt": updated_at,
                        "change": {"kind": "pinned", "eventId": "$b:matrix.test"},
                    },
                }),
                at,
            )
        };
        let entry = |id: &str| {
            json!({
                "eventId": id,
                "pinnedBy": "@owner:matrix.test",
                "pinnedAt": "2026-11-16T08:00:00.000Z",
            })
        };
        let synced = batch(
            MatrixRoomSyncKind::Joined,
            vec![pins(
                json!(["$b:matrix.test", "$a:matrix.test"]),
                // 标准列表里没有的、读不懂的、重复的都不留。
                json!([
                    entry("$b:matrix.test"),
                    entry("$gone:matrix.test"),
                    {"eventId": "$a:matrix.test"},
                    entry("$a:matrix.test"),
                    entry("$b:matrix.test"),
                ]),
                "2026-11-16T08:00:01.000Z",
                1_000,
            )],
        );
        let [change] = room_state_changes(&synced, ME).try_into().unwrap();
        let pins_state = change.pins.expect("读出置顶");
        assert_eq!(
            pins_state
                .entries
                .iter()
                .map(|pin| pin.event_id.as_str())
                .collect::<Vec<_>>(),
            ["$b:matrix.test", "$a:matrix.test"]
        );
        assert_eq!(pins_state.updated_at_ms, 1_794_816_001_000);
        assert_eq!(pins_state.entries[0].pinned_at_ms, 1_794_816_000_000);

        let cleared = batch(
            MatrixRoomSyncKind::Joined,
            vec![pins(json!([]), json!([]), "2026-11-16T09:00:00Z", 2_000)],
        );
        let [change] = room_state_changes(&cleared, ME).try_into().unwrap();
        assert_eq!(change.pins.map(|pins| pins.entries.len()), Some(0));

        // 别的客户端写的（没有我们的字段）、撤回以后空的，都不算一版。
        let ignored = batch(
            MatrixRoomSyncKind::Joined,
            vec![
                state(
                    super::ROOM_PINS_EVENT_TYPE,
                    "",
                    json!({"pinned": ["$a:matrix.test"]}),
                    3_000,
                ),
                state(super::ROOM_PINS_EVENT_TYPE, "", json!({}), 4_000),
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
