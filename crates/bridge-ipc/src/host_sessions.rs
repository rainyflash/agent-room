use serde::{Deserialize, Serialize};
use uuid::{Uuid, Version};

use crate::tools::{IpcMethodValidationFailure, failure};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcOpenHostSessionRequest {
    pub session_key: String,
    pub display_name: String,
}

impl IpcOpenHostSessionRequest {
    pub(crate) fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_session_id(&self.session_key)?;
        if self.display_name.trim() != self.display_name
            || self.display_name.is_empty()
            || self.display_name.chars().count() > 128
            || self.display_name.chars().any(char::is_control)
        {
            return Err(failure("bridge.ipc.session_name_invalid"));
        }
        Ok(())
    }
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
                session_key: session_key.into(),
                display_name: "调试人物".into(),
            };
            assert!(request.validate().is_err());
        }
        let request = IpcOpenHostSessionRequest {
            session_key: Uuid::now_v7().to_string(),
            display_name: " ".into(),
        };
        assert!(request.validate().is_err());
    }

    #[test]
    fn 会话不能包裹桌面初始化或会话管理操作() {
        let wrapped = IpcMethod::WithSession {
            session_id: Uuid::now_v7().to_string(),
            method: Box::new(IpcMethod::OpenHostSession(IpcOpenHostSessionRequest {
                session_key: Uuid::now_v7().to_string(),
                display_name: "调试人物".into(),
            })),
        };
        assert!(wrapped.validate().is_err());
    }
}
