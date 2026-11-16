//! 读个人工作区（`io.github.rainyflash.agentroom.personal-workspace.v1`）里算提醒要用的三样：每间房
//! 读到的位置、免打扰的房间、暂停提醒。文档的样子由网页定（`personal-workspace/domain/
//! workspace-document.ts`）：`{ "schema": 1, "entries": [{ kind, id, value, clock, writerId }] }`，
//! 每个 `[kind, id]` 只有一条。
//!
//! 这里宽松地读：认不出的条目跳过，整份认不出就当什么都没写过。宁可多提醒一次，也别因为网页加了
//! 一种条目就谁都不提醒。

use std::collections::{HashMap, HashSet};

use serde_json::Value;

/// 读到的位置：那一条事件的服务器时间和事件 ID，和网页的 `compareCursors` 一样先比时间再比 ID。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPosition {
    pub at: i64,
    pub event_id: String,
}

/// “暂停提醒”：`dnd` 那一条。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pause {
    /// 没暂停（`0`，或者没有这一条）。
    Off,
    /// 暂停到这一刻（毫秒）。过了就是暂停结束的时候。
    Until(i64),
    /// 一直暂停（`null`）。
    Indefinite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonalWorkspace {
    /// 按 Matrix 房间 ID。
    pub read: HashMap<String, ReadPosition>,
    /// 开了免打扰的 Matrix 房间 ID。
    pub muted: HashSet<String>,
    pub pause: Pause,
}

impl PersonalWorkspace {
    pub fn empty() -> Self {
        Self {
            read: HashMap::new(),
            muted: HashSet::new(),
            pause: Pause::Off,
        }
    }

    /// 从聊天服务器上的那一份读；`None`（没写过）和认不出的都当空的。
    pub fn parse(document: Option<&Value>) -> Self {
        let mut workspace = Self::empty();
        let Some(document) = document else {
            return workspace;
        };
        if document.get("schema").and_then(Value::as_i64) != Some(1) {
            return workspace;
        }
        let Some(entries) = document.get("entries").and_then(Value::as_array) else {
            return workspace;
        };
        for entry in entries {
            workspace.take(entry);
        }
        workspace
    }

    fn take(&mut self, entry: &Value) {
        let (Some(kind), Some(id)) = (
            entry.get("kind").and_then(Value::as_str),
            entry.get("id").and_then(Value::as_str),
        ) else {
            return;
        };
        let value = entry.get("value");
        match kind {
            "read" => {
                if let Some(position) = value.and_then(read_position) {
                    self.read.insert(id.to_owned(), position);
                }
            }
            "mute" => {
                if value.and_then(Value::as_bool) == Some(true) {
                    self.muted.insert(id.to_owned());
                }
            }
            "dnd" if id == "global" => {
                self.pause = match value {
                    Some(Value::Null) => Pause::Indefinite,
                    Some(until) => match until.as_i64() {
                        Some(until) if until > 0 => Pause::Until(until),
                        _ => Pause::Off,
                    },
                    None => Pause::Off,
                };
            }
            _ => {}
        }
    }
}

fn read_position(value: &Value) -> Option<ReadPosition> {
    let event_id = value.get("eventId").and_then(Value::as_str)?;
    let at = value.get("timestamp").and_then(Value::as_i64)?;
    (!event_id.is_empty() && at >= 0).then(|| ReadPosition {
        at,
        event_id: event_id.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Pause, PersonalWorkspace, ReadPosition};

    const ROOM: &str = "!room:matrix.agent-room.localhost";
    const QUIET: &str = "!quiet:matrix.agent-room.localhost";

    fn entry(kind: &str, id: &str, value: serde_json::Value) -> serde_json::Value {
        let mut entry = json!({ "kind": kind, "id": id, "clock": 1, "writerId": "web" });
        entry["value"] = value;
        entry
    }

    #[test]
    fn 读到的位置_免打扰和暂停提醒都读得出来() {
        let document = json!({
            "schema": 1,
            "entries": [
                entry("read", ROOM, json!({ "eventId": "$latest", "timestamp": 1_000 })),
                entry("mute", QUIET, json!(true)),
                entry("mute", ROOM, json!(false)),
                entry("dnd", "global", json!(5_000)),
                entry("favorite", "018f2a00-0000-7000-8000-000000000001", json!(true)),
            ],
        });

        let workspace = PersonalWorkspace::parse(Some(&document));

        assert_eq!(
            workspace.read.get(ROOM),
            Some(&ReadPosition {
                at: 1_000,
                event_id: "$latest".to_owned()
            })
        );
        assert!(workspace.muted.contains(QUIET));
        assert!(!workspace.muted.contains(ROOM));
        assert_eq!(workspace.pause, Pause::Until(5_000));
    }

    #[test]
    fn 暂停提醒_零是没暂停_空是一直暂停() {
        let paused = |value| {
            PersonalWorkspace::parse(Some(&json!({
                "schema": 1,
                "entries": [entry("dnd", "global", value)],
            })))
            .pause
        };
        assert_eq!(paused(json!(0)), Pause::Off);
        assert_eq!(paused(json!(null)), Pause::Indefinite);
        assert_eq!(paused(json!(42)), Pause::Until(42));
    }

    #[test]
    fn 没写过_认不出的版本和坏条目都当空的() {
        assert_eq!(PersonalWorkspace::parse(None), PersonalWorkspace::empty());
        assert_eq!(
            PersonalWorkspace::parse(Some(&json!({ "schema": 2, "entries": [] }))),
            PersonalWorkspace::empty()
        );
        let broken = json!({
            "schema": 1,
            "entries": [
                entry("read", ROOM, json!({ "eventId": "", "timestamp": 1 })),
                entry("read", QUIET, json!("not a cursor")),
                { "kind": "mute" },
                "not an entry",
            ],
        });
        assert_eq!(
            PersonalWorkspace::parse(Some(&broken)),
            PersonalWorkspace::empty()
        );
    }
}
