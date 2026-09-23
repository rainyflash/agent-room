use serde::{Deserialize, Serialize};
use uuid::{Uuid, Version};

use crate::tools::{IpcMethodValidationFailure, failure};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcOpenHostSessionRequest {
    pub session_key: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<IpcHostRoomTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcHostRoomTarget {
    pub catalog_id: String,
    /// 私人房间与已知实例的公开大厅带具体 Matrix 房间；只按目录进入公开大厅时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
}

impl IpcOpenHostSessionRequest {
    pub(crate) fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_session_id(&self.session_key)?;
        validate_room(self.room.as_ref())?;
        validate_display_name(&self.display_name)
    }
}

/// 桌面端接入面板挂在 Bridge 上的邀请。名字可以不定：由接上的 Agent 自己起。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcInvitationOffer {
    pub session_key: String,
    /// 面板里的人定下的名字；为空时用接上的 Agent 自己起的名字。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<IpcHostRoomTarget>,
}

impl IpcInvitationOffer {
    pub(crate) fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_session_id(&self.session_key)?;
        validate_room(self.room.as_ref())?;
        self.display_name
            .as_deref()
            .map_or(Ok(()), validate_display_name)
    }

    /// 接上这份邀请的会话请求：面板定了名字就用它，否则用 Agent 自己起的。
    #[must_use]
    pub fn open_request(&self, chosen: impl FnOnce() -> String) -> IpcOpenHostSessionRequest {
        IpcOpenHostSessionRequest {
            session_key: self.session_key.clone(),
            display_name: self.display_name.clone().unwrap_or_else(chosen),
            room: self.room.clone(),
        }
    }
}

fn validate_room(room: Option<&IpcHostRoomTarget>) -> Result<(), IpcMethodValidationFailure> {
    if let Some(room) = room {
        validate_session_id(&room.catalog_id)?;
        if let Some(room_id) = &room.room_id {
            agent_room_domain::rooms::MatrixRoomReference::new(room_id.clone())
                .map_err(|_| failure("bridge.ipc.room_invalid"))?;
        }
    }
    Ok(())
}

fn validate_display_name(name: &str) -> Result<(), IpcMethodValidationFailure> {
    if name.trim() != name
        || name.is_empty()
        || name.chars().count() > 128
        || name.chars().any(char::is_control)
    {
        return Err(failure("bridge.ipc.session_name_invalid"));
    }
    Ok(())
}

/// 桌面端接入面板挂在 Bridge 上、等 Agent 来接的人物。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcPendingInvitation {
    pub invitation: IpcInvitationOffer,
    /// 还有多久失效；面板开着时会定期续期。
    pub expires_in_ms: u64,
}

/// 面板关闭时撤回；只撤回同一会话键的那份，旧面板撤不掉新面板的邀请。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcWithdrawInvitationRequest {
    pub session_key: String,
}

impl IpcWithdrawInvitationRequest {
    pub(crate) fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_session_id(&self.session_key)
    }
}

/// 凭私人房间口令查看它对应的房间，CLI 与 MCP 据此决定在这个房间里用哪个人物。
/// 不让任何 Agent 加入。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcResolveJoinCodeRequest {
    pub code: String,
}

impl IpcResolveJoinCodeRequest {
    pub(crate) fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_join_code(&self.code)
    }
}

/// 凭口令让这个会话键的人物加入私人房间，随后照常用返回的房间开会话。名字要与开会话时一致：
/// 同一个会话键始终是同一个人物。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcRedeemJoinCodeRequest {
    pub session_key: String,
    pub display_name: String,
    pub code: String,
}

impl IpcRedeemJoinCodeRequest {
    pub(crate) fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_session_id(&self.session_key)?;
        validate_display_name(&self.display_name)?;
        validate_join_code(&self.code)
    }
}

// 口令就是房间的钥匙，调试输出里不能出现。
impl std::fmt::Debug for IpcResolveJoinCodeRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IpcResolveJoinCodeRequest")
            .field("code", &"[已脱敏]")
            .finish()
    }
}

impl std::fmt::Debug for IpcRedeemJoinCodeRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IpcRedeemJoinCodeRequest")
            .field("session_key", &self.session_key)
            .field("display_name", &self.display_name)
            .field("code", &"[已脱敏]")
            .finish()
    }
}

/// 格式在本机就核对，抄错的口令不必连到服务器。
fn validate_join_code(code: &str) -> Result<(), IpcMethodValidationFailure> {
    agent_room_domain::join_codes::PrivateRoomJoinCode::parse(code)
        .map(|_| ())
        .map_err(|_| failure("bridge.ipc.join_code_invalid"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcCloseHostSessionRequest {
    pub session_id: String,
}

impl IpcCloseHostSessionRequest {
    pub(crate) fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_session_id(&self.session_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcHostSessionState {
    Starting,
    Ready,
    Failed,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcHostSessionSummary {
    pub session_id: String,
    pub state: IpcHostSessionState,
    pub agent_id: Option<String>,
    pub error_code: Option<String>,
}

/// Read-only evidence from successful host calls; observing this never renews a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcHostSessionDiagnostics {
    pub session: IpcHostSessionSummary,
    pub display_name: String,
    #[serde(default)]
    pub room_id: Option<String>,
    #[serde(default)]
    pub requested_room: Option<IpcHostRoomTarget>,
    #[serde(default)]
    pub session_key: Option<String>,
    #[serde(default)]
    pub reception_offer: Option<IpcReceptionOffer>,
    pub last_inbox_read_ago_ms: Option<u64>,
    pub last_message_received_ago_ms: Option<u64>,
    pub last_message_sent_ago_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcRegisterReceptionRequest {
    #[serde(default)]
    pub host_type: IpcReceptionHost,
    pub task_id: String,
    pub workspace: String,
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcReceptionHost {
    #[default]
    Codex,
    ClaudeCode,
}
impl IpcRegisterReceptionRequest {
    pub(crate) fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        let id =
            Uuid::parse_str(&self.task_id).map_err(|_| failure("bridge.ipc.host_task_invalid"))?;
        if id.is_nil() || id.to_string() != self.task_id {
            return Err(failure("bridge.ipc.host_task_invalid"));
        }
        if self.workspace.is_empty()
            || self.workspace.len() > 4096
            || self.workspace.chars().any(char::is_control)
        {
            return Err(failure("bridge.ipc.workspace_invalid"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcReceptionOffer {
    pub room_catalog_id: Option<String>,
    pub instance_id: String,
    pub task: IpcRegisterReceptionRequest,
    pub room_id: String,
}

pub(crate) fn validate_session_id(value: &str) -> Result<(), IpcMethodValidationFailure> {
    let parsed = Uuid::parse_str(value).map_err(|_| failure("bridge.ipc.session_id_invalid"))?;
    if parsed.get_version() != Some(Version::SortRand)
        || parsed.get_variant() != uuid::Variant::RFC4122
        || parsed.to_string() != value
    {
        return Err(failure("bridge.ipc.session_id_invalid"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use agent_room_bridge_core::ipc::IpcScope;

    use super::*;
    use crate::{IpcMethod, IpcOpenContentRequest};

    #[test]
    fn 会话包装保留原操作的授权范围且拒绝嵌套包装() {
        let session_id = Uuid::now_v7().to_string();
        let method = IpcMethod::WithSession {
            session_id: session_id.clone(),
            method: Box::new(IpcMethod::OpenContent(IpcOpenContentRequest {
                room_id: None,
                content_id: Uuid::now_v7().to_string(),
            })),
        };
        assert!(method.validate().is_ok());
        assert_eq!(method.required_scope(), IpcScope::ContentRead);
        let nested = IpcMethod::WithSession {
            session_id,
            method: Box::new(method),
        };
        assert_eq!(
            nested.validate().expect_err("嵌套不得被路由").code(),
            "bridge.ipc.session_method_invalid"
        );
    }

    #[test]
    fn 会话键不能用路径或名称冒充且拒绝空名称() {
        for session_key in [
            "../other-agent",
            "Agent A",
            "",
            "01A07063-4799-7a29-88e1-c9f43de239ef",
            "01a07063-4799-7a29-08e1-c9f43de239ef",
        ] {
            let request = IpcOpenHostSessionRequest {
                room: None,
                session_key: session_key.into(),
                display_name: "调试人物".into(),
            };
            assert!(request.validate().is_err());
        }
        let request = IpcOpenHostSessionRequest {
            room: None,
            session_key: Uuid::now_v7().to_string(),
            display_name: " ".into(),
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn 等待接入的邀请可以不定名字_开会话时面板定的名字优先() {
        let unnamed = IpcInvitationOffer {
            room: None,
            session_key: Uuid::now_v7().to_string(),
            display_name: None,
        };
        assert!(
            IpcMethod::OfferInvitation(unnamed.clone())
                .validate()
                .is_ok()
        );
        let encoded = serde_json::to_value(&unnamed).expect("可编码");
        assert!(encoded.get("displayName").is_none(), "{encoded}");
        let request = unnamed.open_request(|| "Scout".into());
        assert_eq!(request.display_name, "Scout");
        assert!(request.validate().is_ok());
        let named = IpcInvitationOffer {
            display_name: Some("面板里的名字".into()),
            ..unnamed.clone()
        };
        assert_eq!(
            named.open_request(|| "Scout".into()).display_name,
            "面板里的名字"
        );
        for name in ["", " 前后有空格 ", "换\n行"] {
            let invalid = IpcInvitationOffer {
                display_name: Some(name.into()),
                ..unnamed.clone()
            };
            assert!(IpcMethod::OfferInvitation(invalid).validate().is_err());
        }
    }

    #[test]
    fn 等待接入的三个方法各有权限且不能包进会话() {
        let invitation = IpcInvitationOffer {
            room: None,
            session_key: Uuid::now_v7().to_string(),
            display_name: Some("调试人物".into()),
        };
        let offer = IpcMethod::OfferInvitation(invitation.clone());
        assert!(offer.validate().is_ok());
        assert_eq!(offer.required_scope(), IpcScope::AgentBootstrap);
        let withdraw = IpcMethod::WithdrawInvitation(IpcWithdrawInvitationRequest {
            session_key: invitation.session_key.clone(),
        });
        assert!(withdraw.validate().is_ok());
        assert_eq!(withdraw.required_scope(), IpcScope::AgentBootstrap);
        assert_eq!(
            IpcMethod::ReadInvitation.required_scope(),
            IpcScope::HostSessionsManage
        );
        assert!(
            IpcMethod::WithdrawInvitation(IpcWithdrawInvitationRequest {
                session_key: "../other".into(),
            })
            .validate()
            .is_err()
        );
        for method in [offer, withdraw, IpcMethod::ReadInvitation] {
            let wrapped = IpcMethod::WithSession {
                session_id: Uuid::now_v7().to_string(),
                method: Box::new(method),
            };
            assert_eq!(
                wrapped.validate().expect_err("不能包进会话").code(),
                "bridge.ipc.session_method_invalid"
            );
        }
        let response = crate::IpcResponse::Invitation {
            invitation: Some(IpcPendingInvitation {
                invitation,
                expires_in_ms: 1_000,
            }),
        };
        let encoded = serde_json::to_value(&response).expect("可编码");
        assert_eq!(encoded["type"], "invitation");
        assert_eq!(encoded["invitation"]["expiresInMs"], 1_000);
        assert_eq!(
            serde_json::from_value::<crate::IpcResponse>(encoded).expect("可解码"),
            response
        );
    }

    #[test]
    fn 口令方法在开会话之前调用_格式在本机核对_口令不进调试输出() {
        let resolve = IpcMethod::ResolveJoinCode(IpcResolveJoinCodeRequest {
            code: "k7p3 q9xw 2dma".into(),
        });
        assert!(resolve.validate().is_ok());
        assert_eq!(resolve.required_scope(), IpcScope::HostSessionsManage);
        let redeem = IpcRedeemJoinCodeRequest {
            session_key: Uuid::now_v7().to_string(),
            display_name: "Scout".into(),
            code: "K7P3-Q9XW-2DMA".into(),
        };
        let method = IpcMethod::RedeemJoinCode(redeem.clone());
        assert!(method.validate().is_ok());
        assert_eq!(method.required_scope(), IpcScope::HostSessionsManage);
        let debug = format!("{resolve:?} {method:?}").to_lowercase();
        assert!(!debug.contains("q9xw"), "{debug}");
        assert!(debug.contains("scout"));
        let encoded = serde_json::to_value(&method).expect("可编码");
        assert_eq!(
            encoded["redeem_join_code"]["sessionKey"],
            redeem.session_key
        );
        assert_eq!(encoded["redeem_join_code"]["code"], "K7P3-Q9XW-2DMA");
        for code in ["", "K7P3-Q9XW-2DM", "K7P3_Q9XW_2DMA", "口令"] {
            let invalid =
                IpcMethod::ResolveJoinCode(IpcResolveJoinCodeRequest { code: code.into() });
            assert_eq!(
                invalid.validate().expect_err("格式不对").code(),
                "bridge.ipc.join_code_invalid"
            );
        }
        for invalid in [
            IpcRedeemJoinCodeRequest {
                session_key: "../other".into(),
                ..redeem.clone()
            },
            IpcRedeemJoinCodeRequest {
                display_name: " ".into(),
                ..redeem.clone()
            },
            IpcRedeemJoinCodeRequest {
                code: "K7P3".into(),
                ..redeem.clone()
            },
        ] {
            assert!(IpcMethod::RedeemJoinCode(invalid).validate().is_err());
        }
        for method in [resolve, method] {
            let wrapped = IpcMethod::WithSession {
                session_id: Uuid::now_v7().to_string(),
                method: Box::new(method),
            };
            assert_eq!(
                wrapped.validate().expect_err("不能包进会话").code(),
                "bridge.ipc.session_method_invalid"
            );
        }
        let response = crate::IpcResponse::JoinCodeRoom {
            room: crate::IpcRoomSummary {
                kind: crate::IpcRoomKind::PrivateRoom,
                catalog_id: Uuid::now_v7().to_string(),
                matrix_room_id: Some("!project:matrix.test".into()),
                name: "项目室".into(),
                slug: None,
                membership: None,
            },
        };
        let encoded = serde_json::to_value(&response).expect("可编码");
        assert_eq!(encoded["type"], "join_code_room");
        assert_eq!(encoded["room"]["matrixRoomId"], "!project:matrix.test");
        assert_eq!(
            serde_json::from_value::<crate::IpcResponse>(encoded).expect("可解码"),
            response
        );
    }

    #[test]
    fn 会话不能包裹桌面初始化或会话管理操作() {
        let wrapped = IpcMethod::WithSession {
            session_id: Uuid::now_v7().to_string(),
            method: Box::new(IpcMethod::OpenHostSession(IpcOpenHostSessionRequest {
                room: None,
                session_key: Uuid::now_v7().to_string(),
                display_name: "调试人物".into(),
            })),
        };
        assert!(wrapped.validate().is_err());
    }
}
