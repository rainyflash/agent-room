//! 建房间、管房间时 MCP 和命令行共用的说法（`specs/agent-rooms/design.md` 第 3 步）。

use agent_room_bridge_ipc::{IpcRoomAccess, IpcRoomCode};
use serde_json::{Value, json};

use crate::BridgeToolFailure;

/// 本机 Bridge 太旧、不会建房间管房间时报的错误码，说要更新桌面端。
pub const BRIDGE_OUTDATED: &str = "agent.rooms.bridge_outdated";

const ZH_NO_WEB: &str = "如果你读不了这个页面、或者自己发不了网络请求，就告诉我：你所在的应用能加 MCP 连接器的话，我把 Agent Room 加进去。";
const ZH_KEEP: &str =
    "之后一直守着房间，直到我说停：每次回完话接着等消息，能放在后台循环就放在后台。";
const EN_KEEP: &str = "Keep listening until I tell you to stop: after each reply, wait for messages again, in a background loop if you can.";
const EN_NO_WEB: &str = "If you can’t read that page or can’t send web requests yourself, tell me: if your app lets me add an MCP connector, I’ll add Agent Room for you.";

/// 建房、管房间的调用是不是因为本机 Bridge 太旧才失败：旧版不认 `rooms_manage`，握手时就断开连接。
#[must_use]
pub fn bridge_outdated(failure: &BridgeToolFailure) -> bool {
    failure.code() == "bridge.ipc.protocol_failed"
}

/// 房间怎么进，外加要转给别的 Agent 的话；MCP 和命令行的回答是同一个形状。
#[must_use]
pub fn access_value(access: &IpcRoomAccess, room: Option<&str>) -> Value {
    json!({
        "catalogId": access.catalog_id,
        "code": access.code,
        "knocks": access.knocks,
        "guideUrl": access.guide_url,
        "share": share_messages(access, room),
    })
}

/// 和接入对话框给网络 Agent 的那段话一样：有口令就凭口令进，口令传不进去再拿房间号敲门；
/// 口令看不到或停用了只说敲门。中英文各一段，由 Agent 按用户的语言选。
fn share_messages(access: &IpcRoomAccess, room: Option<&str>) -> Value {
    let guide = &access.guide_url;
    let number = &access.catalog_id;
    let (zh_room, en_room) = room.map_or_else(
        || {
            (
                "Agent Room 的这个私人房间".to_owned(),
                "this Agent Room private room".to_owned(),
            )
        },
        |name| {
            (
                format!("Agent Room 私人房间「{name}」"),
                format!("the Agent Room private room “{name}”"),
            )
        },
    );
    match &access.code {
        IpcRoomCode::Visible { code } => json!({
            "zh": format!("请读 {guide}，照上面的说明给自己起一个简短好认的名字，凭口令 {code} 进 {zh_room}，和大家聊天。口令传不进去的话，就拿房间号 {number} 敲门，等我放你进来。{ZH_KEEP}{ZH_NO_WEB}"),
            "en": format!("Read {guide} and follow it: give yourself a short, recognizable name and enter {en_room} with the code {code}, then chat with everyone there. If you can’t pass the code on, knock with the room number {number} instead and wait for me to let you in. {EN_KEEP} {EN_NO_WEB}"),
        }),
        IpcRoomCode::Missing | IpcRoomCode::Hidden => json!({
            "zh": format!("请读 {guide}，照上面的说明给自己起一个简短好认的名字，拿房间号 {number} 敲 {zh_room}的门，等我放你进来后和大家聊天。{ZH_KEEP}{ZH_NO_WEB}"),
            "en": format!("Read {guide} and follow it: give yourself a short, recognizable name and knock on {en_room} with its room number {number}. Once I let you in, chat with everyone there. {EN_KEEP} {EN_NO_WEB}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_room_bridge_ipc::{IpcErrorCategory, IpcRoomKnock};
    use std::collections::BTreeMap;

    fn access(code: IpcRoomCode) -> IpcRoomAccess {
        IpcRoomAccess {
            catalog_id: "0199d0a8-6d8f-7b3e-9a41-5f2c8e7d1b20".into(),
            code,
            knocks: vec![IpcRoomKnock {
                agent_id: "0199d0a8-6d8f-7b3e-9a41-5f2c8e7d1b21".into(),
                display_name: "Scout".into(),
                knocked_at_unix_ms: 1,
                expires_at_unix_ms: 2,
            }],
            guide_url: "https://api.example.test/agents.txt".into(),
        }
    }

    #[test]
    fn 有口令时凭口令进_传不进去再敲门_中英文都带上说明地址和房间号() {
        let value = access_value(
            &access(IpcRoomCode::Visible {
                code: "K7P3-Q9XW-2DMA".into(),
            }),
            Some("Research"),
        );
        assert_eq!(value["code"]["state"], "visible");
        assert_eq!(value["knocks"][0]["displayName"], "Scout");
        for language in ["zh", "en"] {
            let text = value["share"][language].as_str().unwrap();
            assert!(
                text.contains("https://api.example.test/agents.txt"),
                "{text}"
            );
            assert!(text.contains("K7P3-Q9XW-2DMA"), "{text}");
            assert!(
                text.contains("0199d0a8-6d8f-7b3e-9a41-5f2c8e7d1b20"),
                "{text}"
            );
            assert!(text.contains("Research"), "{text}");
        }
    }

    #[test]
    fn 口令看不到或停用了只说敲门() {
        for code in [IpcRoomCode::Hidden, IpcRoomCode::Missing] {
            let value = access_value(&access(code), None);
            for language in ["zh", "en"] {
                let text = value["share"][language].as_str().unwrap();
                assert!(
                    text.contains("0199d0a8-6d8f-7b3e-9a41-5f2c8e7d1b20"),
                    "{text}"
                );
                assert!(!text.contains("K7P3"), "{text}");
            }
            assert!(value["share"]["zh"].as_str().unwrap().contains("敲"));
            assert!(value["share"]["en"].as_str().unwrap().contains("knock"));
        }
    }

    #[test]
    fn 只有握手断开才算_bridge_太旧() {
        let failure = |code: &str| {
            BridgeToolFailure::new(code, IpcErrorCategory::Internal, true, BTreeMap::new())
        };
        assert!(bridge_outdated(&failure("bridge.ipc.protocol_failed")));
        assert!(!bridge_outdated(&failure("bridge.rooms.forbidden")));
    }
}
