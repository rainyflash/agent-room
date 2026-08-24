use std::fmt;

use agent_room_domain::ids::{AgentId, AgentInstanceId};

const MAX_MATRIX_ID_LENGTH: usize = 512;
const MAX_ROOM_ALIAS_LOCALPART_LENGTH: usize = 255;
const MAX_DEVICE_ID_LENGTH: usize = 255;
const MAX_EVENT_TYPE_LENGTH: usize = 255;
const MAX_TRANSACTION_ID_LENGTH: usize = 255;
const MAX_STATE_KEY_LENGTH: usize = 512;
const MAX_SYNC_TOKEN_LENGTH: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixValueError {
    InvalidAgentLocalpart,
    InvalidUserId,
    InvalidRoomId,
    InvalidRoomAliasLocalpart,
    InvalidEventId,
    InvalidDeviceId,
    InvalidEventType,
    InvalidTransactionId,
    InvalidStateKey,
    InvalidSyncToken,
    InvalidBackfillToken,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MatrixAgentLocalpart(String);

impl MatrixAgentLocalpart {
    pub fn from_agent_id(agent_id: AgentId) -> Self {
        Self(format!("_agent_{}", agent_id.as_uuid().simple()))
    }

    /// 从不可信协议输入恢复 Agent 专属 Localpart。
    ///
    /// # Errors
    ///
    /// 值不符合 `_agent_` 加 UUID 的独占命名规则时返回错误。
    pub fn new(value: impl Into<String>) -> Result<Self, MatrixValueError> {
        let value = value.into();
        if !valid_agent_localpart(&value) {
            return Err(MatrixValueError::InvalidAgentLocalpart);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for MatrixAgentLocalpart {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("MatrixAgentLocalpart")
            .field(&self.0)
            .finish()
    }
}

macro_rules! matrix_string_value {
    ($name:ident, $error:ident, $validator:ident) => {
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// 从不可信协议输入创建受校验的 Matrix 值。
            ///
            /// # Errors
            ///
            /// 输入不满足对应 Matrix 标识边界时返回稳定值错误。
            pub fn new(value: impl Into<String>) -> Result<Self, MatrixValueError> {
                let value = value.into();
                if !$validator(&value) {
                    return Err(MatrixValueError::$error);
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_tuple(stringify!($name))
                    .field(&self.0)
                    .finish()
            }
        }
    };
}

matrix_string_value!(MatrixUserId, InvalidUserId, valid_user_id);
matrix_string_value!(MatrixRoomId, InvalidRoomId, valid_room_id);
matrix_string_value!(
    MatrixRoomAliasLocalpart,
    InvalidRoomAliasLocalpart,
    valid_room_alias_localpart
);
matrix_string_value!(MatrixEventId, InvalidEventId, valid_event_id);
matrix_string_value!(MatrixDeviceId, InvalidDeviceId, valid_device_id);
matrix_string_value!(MatrixEventType, InvalidEventType, valid_event_type);
matrix_string_value!(
    MatrixTransactionId,
    InvalidTransactionId,
    valid_transaction_id
);

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MatrixStateKey(String);

impl MatrixStateKey {
    pub fn from_agent_instance_id(agent_instance_id: AgentInstanceId) -> Self {
        Self(agent_instance_id.to_string())
    }

    /// 从不可信协议输入创建 Matrix 状态键。
    ///
    /// # Errors
    ///
    /// 状态键过长或包含控制字符时返回错误；空键符合 Matrix 规范。
    pub fn new(value: impl Into<String>) -> Result<Self, MatrixValueError> {
        let value = value.into();
        if !valid_state_key(&value) {
            return Err(MatrixValueError::InvalidStateKey);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for MatrixStateKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("MatrixStateKey")
            .field(&self.0)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct MatrixSyncToken(String);

impl MatrixSyncToken {
    /// 创建不透明增量同步游标。
    ///
    /// # Errors
    ///
    /// 空值、超长值或控制字符会被拒绝。
    pub fn new(value: impl Into<String>) -> Result<Self, MatrixValueError> {
        let value = value.into();
        if !valid_opaque_token(&value) {
            return Err(MatrixValueError::InvalidSyncToken);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for MatrixSyncToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[Matrix 同步游标]")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct MatrixBackfillToken(String);

impl MatrixBackfillToken {
    /// 创建不透明历史分页游标。
    ///
    /// # Errors
    ///
    /// 空值、超长值或控制字符会被拒绝。
    pub fn new(value: impl Into<String>) -> Result<Self, MatrixValueError> {
        let value = value.into();
        if !valid_opaque_token(&value) {
            return Err(MatrixValueError::InvalidBackfillToken);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for MatrixBackfillToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[Matrix 回填游标]")
    }
}

fn valid_user_id(value: &str) -> bool {
    valid_sigil_id(value, '@')
}

fn valid_agent_localpart(value: &str) -> bool {
    let Some(identifier) = value.strip_prefix("_agent_") else {
        return false;
    };
    identifier.len() == 32
        && identifier
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_room_id(value: &str) -> bool {
    valid_sigil_id(value, '!')
}

fn valid_room_alias_localpart(value: &str) -> bool {
    (1..=MAX_ROOM_ALIAS_LOCALPART_LENGTH).contains(&value.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'_' | b'=' | b'-')
        })
}

fn valid_event_id(value: &str) -> bool {
    (4..=MAX_MATRIX_ID_LENGTH).contains(&value.len())
        && value.starts_with('$')
        && valid_protocol_text(value)
}

fn valid_sigil_id(value: &str, sigil: char) -> bool {
    (4..=MAX_MATRIX_ID_LENGTH).contains(&value.len())
        && value.starts_with(sigil)
        && value[1..].contains(':')
        && valid_protocol_text(value)
}

fn valid_device_id(value: &str) -> bool {
    (1..=MAX_DEVICE_ID_LENGTH).contains(&value.len()) && valid_protocol_text(value)
}

fn valid_event_type(value: &str) -> bool {
    (3..=MAX_EVENT_TYPE_LENGTH).contains(&value.len())
        && value.contains('.')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_transaction_id(value: &str) -> bool {
    (1..=MAX_TRANSACTION_ID_LENGTH).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'~'))
}

fn valid_state_key(value: &str) -> bool {
    value.len() <= MAX_STATE_KEY_LENGTH && valid_protocol_text(value)
}

fn valid_opaque_token(value: &str) -> bool {
    (1..=MAX_SYNC_TOKEN_LENGTH).contains(&value.len()) && valid_protocol_text(value)
}

fn valid_protocol_text(value: &str) -> bool {
    !value.contains('\0') && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::{
        MatrixAgentLocalpart, MatrixEventType, MatrixRoomAliasLocalpart, MatrixRoomId,
        MatrixStateKey, MatrixSyncToken, MatrixTransactionId, MatrixUserId,
    };
    use agent_room_domain::ids::{AgentId, AgentInstanceId};
    use uuid::Uuid;

    #[test]
    fn 外部标识在进入适配器前拒绝结构错误() {
        assert!(MatrixUserId::new("@agent:example.org").is_ok());
        assert!(MatrixRoomId::new("!room:example.org").is_ok());
        assert!(MatrixUserId::new("agent").is_err());
        assert!(MatrixRoomId::new("#alias:example.org").is_err());
        assert!(MatrixEventType::new("org.agentroom.message.preview.v1").is_ok());
        assert!(MatrixEventType::new("m room message").is_err());
    }

    #[test]
    fn 事务标识限制为可安全放入路径的字符() {
        assert!(MatrixTransactionId::new("message_0198.test-1~retry").is_ok());
        assert!(MatrixTransactionId::new("message/escape").is_err());
        assert!(MatrixTransactionId::new("message\r\nheader").is_err());
    }

    #[test]
    fn 状态键允许空值并可由实例标识稳定派生() {
        assert!(MatrixStateKey::new("").is_ok());
        assert!(MatrixStateKey::new("状态\n键").is_err());
        assert!(MatrixStateKey::new("x".repeat(513)).is_err());

        let instance_id = AgentInstanceId::from_uuid(Uuid::from_u128(7));
        assert_eq!(
            MatrixStateKey::from_agent_instance_id(instance_id).as_str(),
            instance_id.to_string()
        );
    }

    #[test]
    fn 同步游标调试输出不泄漏服务端位置() {
        let token = MatrixSyncToken::new("sensitive-position-token").expect("游标有效");
        assert_eq!(format!("{token:?}"), "[Matrix 同步游标]");
    }

    #[test]
    fn agent_localpart_仅能由稳定业务标识派生() {
        let agent_id = AgentId::from_uuid(
            Uuid::parse_str("01945c1e-7b5a-7c7f-8a28-2de53f56a9a3").expect("UUID 有效"),
        );
        let localpart = MatrixAgentLocalpart::from_agent_id(agent_id);
        assert_eq!(
            localpart.as_str(),
            "_agent_01945c1e7b5a7c7f8a282de53f56a9a3"
        );
        assert!(MatrixAgentLocalpart::new(localpart.as_str()).is_ok());
        assert!(MatrixAgentLocalpart::new("_agent_alpha").is_err());
    }

    #[test]
    fn 房间别名_localpart_限制为可确定性生成的安全字符() {
        assert!(MatrixRoomAliasLocalpart::new("agent-room.general.ap-southeast-1").is_ok());
        assert!(MatrixRoomAliasLocalpart::new("General").is_err());
        assert!(MatrixRoomAliasLocalpart::new("general:example.org").is_err());
        assert!(MatrixRoomAliasLocalpart::new("").is_err());
    }
}
