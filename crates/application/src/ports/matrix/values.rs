use std::fmt;

const MAX_MATRIX_ID_LENGTH: usize = 512;
const MAX_DEVICE_ID_LENGTH: usize = 255;
const MAX_EVENT_TYPE_LENGTH: usize = 255;
const MAX_TRANSACTION_ID_LENGTH: usize = 255;
const MAX_SYNC_TOKEN_LENGTH: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixValueError {
    InvalidUserId,
    InvalidRoomId,
    InvalidEventId,
    InvalidDeviceId,
    InvalidEventType,
    InvalidTransactionId,
    InvalidSyncToken,
    InvalidBackfillToken,
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
matrix_string_value!(MatrixEventId, InvalidEventId, valid_event_id);
matrix_string_value!(MatrixDeviceId, InvalidDeviceId, valid_device_id);
matrix_string_value!(MatrixEventType, InvalidEventType, valid_event_type);
matrix_string_value!(
    MatrixTransactionId,
    InvalidTransactionId,
    valid_transaction_id
);

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

fn valid_room_id(value: &str) -> bool {
    valid_sigil_id(value, '!')
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

fn valid_opaque_token(value: &str) -> bool {
    (1..=MAX_SYNC_TOKEN_LENGTH).contains(&value.len()) && valid_protocol_text(value)
}

fn valid_protocol_text(value: &str) -> bool {
    !value.contains('\0') && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::{
        MatrixEventType, MatrixRoomId, MatrixSyncToken, MatrixTransactionId, MatrixUserId,
    };

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
    fn 同步游标调试输出不泄漏服务端位置() {
        let token = MatrixSyncToken::new("sensitive-position-token").expect("游标有效");
        assert_eq!(format!("{token:?}"), "[Matrix 同步游标]");
    }
}
