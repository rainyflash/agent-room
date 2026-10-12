//! Agent 自己建房间、管房间（`specs/agent-rooms/design.md`）：MCP 和命令行经 Bridge 替这台电脑的账号
//! 建私人房间，管会话所在房间怎么进（口令、敲门）、邀请人、移出 Agent。只管 Agent 建的房间，由控制面
//! 把关。

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    host_sessions::{validate_display_name, validate_session_id},
    tools::{IpcMethodValidationFailure, failure, validate_uuid_v7},
};

/// 房间名的上限，和控制面的一样。
const ROOM_NAME_CHARACTERS: usize = 128;

/// 建私人房间（`CreateRoom`，不要会话）。Bridge 先按会话键建好（或找回）这个人物，拿它当建房的
/// Agent；随后照常用返回的房间开会话。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcCreateRoomRequest {
    /// 先生成、先记下的房间 ID（UUIDv7）。结果没回来时带同一个 ID 重试，拿回的是同一个房间。
    pub catalog_id: String,
    pub name: String,
    pub session_key: String,
    pub display_name: String,
}

impl IpcCreateRoomRequest {
    pub(crate) fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_uuid_v7(&self.catalog_id, "bridge.ipc.room_invalid")?;
        validate_room_name(&self.name)?;
        validate_session_id(&self.session_key)?;
        validate_display_name(&self.display_name)
    }
}

/// 会话所在的房间怎么进（`RoomAccess`，要会话）：口令和等着的敲门。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcRoomAccessRequest {
    #[serde(default)]
    pub action: IpcRoomAccessAction,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcRoomAccessAction {
    /// 只看；没有口令就生成一个。有口令、但看不到时不替人换：换了以后，已经拿到旧口令、还没进来的
    /// Agent 就进不来了。
    #[default]
    Show,
    /// 换一个新口令，旧的马上失效。
    Replace,
    /// 停用口令。
    Disable,
}

/// 放行或拒绝敲门的 Agent（`AnswerKnock`，要会话）。放行时由控制面替它进房间。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcAnswerKnockRequest {
    pub agent_id: String,
    pub admit: bool,
}

impl IpcAnswerKnockRequest {
    pub(crate) fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_uuid_v7(&self.agent_id, "bridge.ipc.agent_id_invalid")
    }
}

/// 按账户 ID 邀请一个人，只给查看和发言（`InvitePerson`，要会话）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcInvitePersonRequest {
    pub account_id: String,
}

impl IpcInvitePersonRequest {
    pub(crate) fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_uuid_v7(&self.account_id, "bridge.ipc.account_id_invalid")
    }
}

/// 移出凭口令或敲门进来的 Agent（`RemoveAgent`，要会话）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcRemoveAgentRequest {
    pub agent_id: String,
}

impl IpcRemoveAgentRequest {
    pub(crate) fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_uuid_v7(&self.agent_id, "bridge.ipc.agent_id_invalid")
    }
}

/// 房间怎么进：房间号（`catalogId`）、口令和等着的敲门。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcRoomAccess {
    pub catalog_id: String,
    pub code: IpcRoomCode,
    /// 等着的、没作废的敲门，先敲的在前。
    pub knocks: Vec<IpcRoomKnock>,
    /// 接入说明的地址，发给别的 Agent 的话里要带上。
    pub guide_url: String,
}

/// 房间现在的口令。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum IpcRoomCode {
    /// 给人看的形式 `XXXX-XXXX-XXXX`。
    Visible { code: String },
    /// 没有口令：停用了，或者从来没生成过。
    Missing,
    /// 有口令，但看不到了：以前生成的，或者服务器没法保管口令。要给别的 Agent 就明说换一个。
    Hidden,
}

// 口令就是房间的钥匙，调试输出里不能出现。
impl fmt::Debug for IpcRoomCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Visible { .. } => formatter
                .debug_struct("Visible")
                .field("code", &"[已脱敏]")
                .finish(),
            Self::Missing => formatter.write_str("Missing"),
            Self::Hidden => formatter.write_str("Hidden"),
        }
    }
}

/// 一次在等的敲门。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcRoomKnock {
    pub agent_id: String,
    pub display_name: String,
    pub knocked_at_unix_ms: i64,
    /// 到这个时候还没人回答就作废。
    pub expires_at_unix_ms: i64,
}

fn validate_room_name(name: &str) -> Result<(), IpcMethodValidationFailure> {
    if name.trim() != name
        || name.is_empty()
        || name.chars().count() > ROOM_NAME_CHARACTERS
        || name.chars().any(char::is_control)
    {
        return Err(failure("bridge.ipc.room_name_invalid"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use agent_room_bridge_core::ipc::IpcScope;
    use uuid::Uuid;

    use super::*;
    use crate::{IpcMethod, IpcResponse};

    /// 第 4 版的 UUID：格式对，版本不对。
    const NOT_V7: &str = "6f9619ff-8b86-4011-b42d-00c04fc964ff";

    fn create_request() -> IpcCreateRoomRequest {
        IpcCreateRoomRequest {
            catalog_id: Uuid::now_v7().to_string(),
            name: "周报".into(),
            session_key: Uuid::now_v7().to_string(),
            display_name: "Scout".into(),
        }
    }

    fn with_session(method: IpcMethod) -> IpcMethod {
        IpcMethod::WithSession {
            session_id: Uuid::now_v7().to_string(),
            method: Box::new(method),
        }
    }

    #[test]
    fn 建房不要会话_放进会话里不认() {
        let request = create_request();
        let method = IpcMethod::CreateRoom(request.clone());
        assert!(method.validate().is_ok());
        assert_eq!(method.name(), "create_room");
        assert_eq!(method.required_scope(), IpcScope::RoomsManage);
        let encoded = serde_json::to_value(&method).expect("可编码");
        assert_eq!(encoded["create_room"]["catalogId"], request.catalog_id);
        assert_eq!(encoded["create_room"]["name"], "周报");
        assert_eq!(encoded["create_room"]["sessionKey"], request.session_key);
        assert_eq!(
            with_session(method)
                .validate()
                .expect_err("建房不在会话里做")
                .code(),
            "bridge.ipc.session_method_invalid"
        );
    }

    #[test]
    fn 建房的房间_id_名字和人物都要合规() {
        let cases = [
            (
                IpcCreateRoomRequest {
                    catalog_id: NOT_V7.into(),
                    ..create_request()
                },
                "bridge.ipc.room_invalid",
            ),
            (
                IpcCreateRoomRequest {
                    catalog_id: Uuid::now_v7().to_string().to_uppercase(),
                    ..create_request()
                },
                "bridge.ipc.room_invalid",
            ),
            (
                IpcCreateRoomRequest {
                    name: String::new(),
                    ..create_request()
                },
                "bridge.ipc.room_name_invalid",
            ),
            (
                IpcCreateRoomRequest {
                    name: " 周报".into(),
                    ..create_request()
                },
                "bridge.ipc.room_name_invalid",
            ),
            (
                IpcCreateRoomRequest {
                    name: "周\n报".into(),
                    ..create_request()
                },
                "bridge.ipc.room_name_invalid",
            ),
            (
                IpcCreateRoomRequest {
                    name: "长".repeat(ROOM_NAME_CHARACTERS + 1),
                    ..create_request()
                },
                "bridge.ipc.room_name_invalid",
            ),
            (
                IpcCreateRoomRequest {
                    session_key: "../other".into(),
                    ..create_request()
                },
                "bridge.ipc.session_id_invalid",
            ),
            (
                IpcCreateRoomRequest {
                    display_name: String::new(),
                    ..create_request()
                },
                "bridge.ipc.session_name_invalid",
            ),
        ];
        for (request, code) in cases {
            assert_eq!(
                IpcMethod::CreateRoom(request)
                    .validate()
                    .expect_err("不合规")
                    .code(),
                code
            );
        }
        let longest = IpcCreateRoomRequest {
            name: "长".repeat(ROOM_NAME_CHARACTERS),
            ..create_request()
        };
        assert!(IpcMethod::CreateRoom(longest).validate().is_ok());
    }

    #[test]
    fn 管房间的方法都在会话里用_要管房间的授权() {
        let agent_id = Uuid::now_v7().to_string();
        let methods = [
            (
                IpcMethod::RoomAccess(IpcRoomAccessRequest::default()),
                "room_access",
            ),
            (
                IpcMethod::AnswerKnock(IpcAnswerKnockRequest {
                    agent_id: agent_id.clone(),
                    admit: true,
                }),
                "answer_knock",
            ),
            (
                IpcMethod::InvitePerson(IpcInvitePersonRequest {
                    account_id: Uuid::now_v7().to_string(),
                }),
                "invite_person",
            ),
            (
                IpcMethod::RemoveAgent(IpcRemoveAgentRequest { agent_id }),
                "remove_agent",
            ),
        ];
        for (method, name) in methods {
            assert_eq!(method.name(), name);
            let wrapped = with_session(method);
            assert!(wrapped.validate().is_ok(), "{name}");
            assert_eq!(wrapped.required_scope(), IpcScope::RoomsManage);
        }
    }

    #[test]
    fn 敲门_邀请和移出的_id_要是_uuid_v7() {
        let not_v7 = NOT_V7.to_owned();
        let cases = [
            (
                IpcMethod::AnswerKnock(IpcAnswerKnockRequest {
                    agent_id: not_v7.clone(),
                    admit: false,
                }),
                "bridge.ipc.agent_id_invalid",
            ),
            (
                IpcMethod::InvitePerson(IpcInvitePersonRequest {
                    account_id: "someone@example.com".into(),
                }),
                "bridge.ipc.account_id_invalid",
            ),
            (
                IpcMethod::RemoveAgent(IpcRemoveAgentRequest { agent_id: not_v7 }),
                "bridge.ipc.agent_id_invalid",
            ),
        ];
        for (method, code) in cases {
            assert_eq!(
                with_session(method).validate().expect_err("不合规").code(),
                code
            );
        }
    }

    #[test]
    fn 看口令默认只看_另外两种要明说() {
        let decoded: IpcRoomAccessRequest = serde_json::from_str("{}").expect("可解码");
        assert_eq!(decoded.action, IpcRoomAccessAction::Show);
        for (text, action) in [
            (r#"{"action":"replace"}"#, IpcRoomAccessAction::Replace),
            (r#"{"action":"disable"}"#, IpcRoomAccessAction::Disable),
        ] {
            let decoded: IpcRoomAccessRequest = serde_json::from_str(text).expect("可解码");
            assert_eq!(decoded.action, action);
        }
        assert!(serde_json::from_str::<IpcRoomAccessRequest>(r#"{"action":"rotate"}"#).is_err());
    }

    #[test]
    fn 房间怎么进的回答_口令三种样子_调试输出里没有口令() {
        let catalog_id = Uuid::now_v7().to_string();
        let response = IpcResponse::RoomAccess {
            access: IpcRoomAccess {
                catalog_id: catalog_id.clone(),
                code: IpcRoomCode::Visible {
                    code: "K7P3-Q9XW-2DMA".into(),
                },
                knocks: vec![IpcRoomKnock {
                    agent_id: Uuid::now_v7().to_string(),
                    display_name: "Helper".into(),
                    knocked_at_unix_ms: 1_000,
                    expires_at_unix_ms: 3_601_000,
                }],
                guide_url: "https://api.agent-room.test/agents.txt".into(),
            },
        };
        let encoded = serde_json::to_value(&response).expect("可编码");
        assert_eq!(encoded["type"], "room_access");
        assert_eq!(encoded["access"]["catalogId"], catalog_id);
        assert_eq!(encoded["access"]["code"]["state"], "visible");
        assert_eq!(encoded["access"]["code"]["code"], "K7P3-Q9XW-2DMA");
        assert_eq!(encoded["access"]["knocks"][0]["displayName"], "Helper");
        assert_eq!(encoded["access"]["knocks"][0]["expiresAtUnixMs"], 3_601_000);
        let decoded: IpcResponse = serde_json::from_value(encoded).expect("可解码");
        assert_eq!(decoded, response);
        assert!(!format!("{response:?}").contains("Q9XW"));
        for (code, state) in [
            (IpcRoomCode::Missing, "missing"),
            (IpcRoomCode::Hidden, "hidden"),
        ] {
            let encoded = serde_json::to_value(&code).expect("可编码");
            assert_eq!(encoded, serde_json::json!({ "state": state }));
            assert_eq!(
                serde_json::from_value::<IpcRoomCode>(encoded).expect("可解码"),
                code
            );
        }
        assert!(
            serde_json::from_value::<IpcRoomCode>(serde_json::json!({ "state": "visible" }))
                .is_err()
        );
    }
}
