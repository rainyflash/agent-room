use std::{collections::BTreeSet, fmt};

use crate::{
    DomainError, DomainResult,
    content::{ContentMediaType, Sha256Digest},
    ids::{ContentEncryptionContextId, ContentId, MessageId},
};
use zeroize::Zeroizing;

const MAX_TITLE_CHARACTERS: usize = 120;
const MAX_SUMMARY_CHARACTERS: usize = 500;
const MAX_LANGUAGE_LENGTH: usize = 35;
const MAX_RISK_FLAG_LENGTH: usize = 64;
const MAX_RISK_FLAGS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageProvenance {
    Human,
    HumanConfirmedAgent,
    AutonomousAgent,
}

impl MessageProvenance {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::HumanConfirmedAgent => "human_confirmed_agent",
            Self::AutonomousAgent => "autonomous_agent",
        }
    }
}

impl TryFrom<&str> for MessageProvenance {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "human" => Ok(Self::Human),
            "human_confirmed_agent" => Ok(Self::HumanConfirmedAgent),
            "autonomous_agent" => Ok(Self::AutonomousAgent),
            _ => Err(DomainError::Validation {
                field: "message_provenance",
                reason: "不是支持的来源模式",
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageSensitivity {
    Normal,
    Sensitive,
    Restricted,
}

impl MessageSensitivity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Sensitive => "sensitive",
            Self::Restricted => "restricted",
        }
    }
}

impl TryFrom<&str> for MessageSensitivity {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "normal" => Ok(Self::Normal),
            "sensitive" => Ok(Self::Sensitive),
            "restricted" => Ok(Self::Restricted),
            _ => Err(DomainError::Validation {
                field: "message_sensitivity",
                reason: "不是支持的敏感级别",
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRevisionKind {
    Replace,
    Redact,
    Moderate,
}

impl MessageRevisionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::Redact => "redact",
            Self::Moderate => "moderate",
        }
    }
}

impl TryFrom<&str> for MessageRevisionKind {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "replace" => Ok(Self::Replace),
            "redact" => Ok(Self::Redact),
            "moderate" => Ok(Self::Moderate),
            _ => Err(DomainError::Validation {
                field: "message_revision_kind",
                reason: "不是支持的修订类型",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageTitle(String);

impl MessageTitle {
    /// 创建消息标题。
    ///
    /// # Errors
    ///
    /// 标题为空、包含控制字符或超过协议上限时返回错误。
    pub fn new(value: impl Into<String>) -> DomainResult<Self> {
        let value = value.into();
        validate_bounded_text("message_title", &value, MAX_TITLE_CHARACTERS)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageSummary(String);

impl MessageSummary {
    /// 创建消息摘要。
    ///
    /// # Errors
    ///
    /// 摘要为空、包含控制字符或超过协议上限时返回错误。
    pub fn new(value: impl Into<String>) -> DomainResult<Self> {
        let value = value.into();
        validate_bounded_text("message_summary", &value, MAX_SUMMARY_CHARACTERS)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageLanguage(String);

impl MessageLanguage {
    /// 创建受限 BCP 47 语言标签。
    ///
    /// # Errors
    ///
    /// 标签长度或字符结构不满足协议约束时返回错误。
    pub fn new(value: impl Into<String>) -> DomainResult<Self> {
        let value = value.into();
        let valid = (2..=MAX_LANGUAGE_LENGTH).contains(&value.len())
            && value
                .split('-')
                .enumerate()
                .all(|(index, part)| language_part_is_valid(index, part));
        if !valid {
            return Err(DomainError::Validation {
                field: "message_language",
                reason: "不是受支持的 BCP 47 语言标签",
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MessageRiskFlag(String);

impl MessageRiskFlag {
    /// 创建可扩展风险标签。
    ///
    /// # Errors
    ///
    /// 标签不符合小写蛇形标识约束时返回错误。
    pub fn new(value: impl Into<String>) -> DomainResult<Self> {
        let value = value.into();
        let mut bytes = value.bytes();
        let starts_with_letter = bytes.next().is_some_and(|byte| byte.is_ascii_lowercase());
        let remainder_is_valid =
            bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
        if value.len() > MAX_RISK_FLAG_LENGTH || !starts_with_letter || !remainder_is_valid {
            return Err(DomainError::Validation {
                field: "message_risk_flag",
                reason: "必须是长度受限的小写蛇形标识",
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRiskFlags(BTreeSet<MessageRiskFlag>);

impl MessageRiskFlags {
    /// 创建去重后的风险标签集合。
    ///
    /// # Errors
    ///
    /// 去重后的标签数量超过协议上限时返回错误。
    pub fn new(flags: impl IntoIterator<Item = MessageRiskFlag>) -> DomainResult<Self> {
        let flags = flags.into_iter().collect::<BTreeSet<_>>();
        if flags.len() > MAX_RISK_FLAGS {
            return Err(DomainError::Validation {
                field: "message_risk_flags",
                reason: "风险标签不能超过 16 个",
            });
        }
        Ok(Self(flags))
    }

    pub fn iter(&self) -> impl Iterator<Item = &MessageRiskFlag> {
        self.0.iter()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessagePreview {
    title: MessageTitle,
    summary: MessageSummary,
    content_type: ContentMediaType,
    language: Option<MessageLanguage>,
    sensitivity: MessageSensitivity,
    risk_flags: MessageRiskFlags,
}

impl MessagePreview {
    pub const fn new(
        title: MessageTitle,
        summary: MessageSummary,
        content_type: ContentMediaType,
        language: Option<MessageLanguage>,
        sensitivity: MessageSensitivity,
        risk_flags: MessageRiskFlags,
    ) -> Self {
        Self {
            title,
            summary,
            content_type,
            language,
            sensitivity,
            risk_flags,
        }
    }

    pub const fn title(&self) -> &MessageTitle {
        &self.title
    }

    pub const fn summary(&self) -> &MessageSummary {
        &self.summary
    }

    pub const fn content_type(&self) -> &ContentMediaType {
        &self.content_type
    }

    pub const fn language(&self) -> Option<&MessageLanguage> {
        self.language.as_ref()
    }

    pub const fn sensitivity(&self) -> MessageSensitivity {
        self.sensitivity
    }

    pub const fn risk_flags(&self) -> &MessageRiskFlags {
        &self.risk_flags
    }
}

pub const CLIENT_CONTENT_KEY_BYTES: usize = 32;
pub const CLIENT_CONTENT_NONCE_BYTES: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientContentEncryptionAlgorithm {
    Aes256GcmV1,
}

impl ClientContentEncryptionAlgorithm {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Aes256GcmV1 => "io.github.rainyflash.agentroom.content.aes-256-gcm.v1",
        }
    }
}

impl TryFrom<&str> for ClientContentEncryptionAlgorithm {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "io.github.rainyflash.agentroom.content.aes-256-gcm.v1" => Ok(Self::Aes256GcmV1),
            _ => Err(DomainError::Validation {
                field: "client_content_encryption_algorithm",
                reason: "不支持的客户端正文加密算法",
            }),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ClientContentEncryption {
    algorithm: ClientContentEncryptionAlgorithm,
    context_id: ContentEncryptionContextId,
    key: Zeroizing<[u8; CLIENT_CONTENT_KEY_BYTES]>,
    nonce: [u8; CLIENT_CONTENT_NONCE_BYTES],
    plaintext_size_bytes: u64,
}

impl ClientContentEncryption {
    /// 创建只允许随 Matrix 加密事件传递的正文解密材料。
    ///
    /// # Errors
    ///
    /// 明文字节数为空或超过全局正文上限时返回错误。
    pub fn new(
        algorithm: ClientContentEncryptionAlgorithm,
        context_id: ContentEncryptionContextId,
        key: [u8; CLIENT_CONTENT_KEY_BYTES],
        nonce: [u8; CLIENT_CONTENT_NONCE_BYTES],
        plaintext_size_bytes: u64,
    ) -> DomainResult<Self> {
        if !(1..=25 * 1_024 * 1_024).contains(&plaintext_size_bytes) {
            return Err(DomainError::Validation {
                field: "client_content_plaintext_size",
                reason: "必须在 1 字节到 25 MiB 之间",
            });
        }
        Ok(Self {
            algorithm,
            context_id,
            key: Zeroizing::new(key),
            nonce,
            plaintext_size_bytes,
        })
    }

    pub const fn algorithm(&self) -> ClientContentEncryptionAlgorithm {
        self.algorithm
    }

    pub const fn context_id(&self) -> ContentEncryptionContextId {
        self.context_id
    }

    pub fn key(&self) -> &[u8; CLIENT_CONTENT_KEY_BYTES] {
        &self.key
    }

    pub const fn nonce(&self) -> &[u8; CLIENT_CONTENT_NONCE_BYTES] {
        &self.nonce
    }

    pub const fn plaintext_size_bytes(&self) -> u64 {
        self.plaintext_size_bytes
    }
}

impl fmt::Debug for ClientContentEncryption {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientContentEncryption")
            .field("algorithm", &self.algorithm)
            .field("context_id", &self.context_id)
            .field("key", &"[已隐藏]")
            .field("nonce", &"[已隐藏]")
            .field("plaintext_size_bytes", &self.plaintext_size_bytes)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageContentReference {
    content_id: ContentId,
    digest: Sha256Digest,
    size_bytes: u64,
    client_encryption: Option<ClientContentEncryption>,
}

impl MessageContentReference {
    /// 创建只包含按需读取元数据的正文引用。
    ///
    /// # Errors
    ///
    /// 字节长度为零或超过全局内容上限时返回错误。
    pub fn new(content_id: ContentId, digest: Sha256Digest, size_bytes: u64) -> DomainResult<Self> {
        if !(1..=25 * 1_024 * 1_024).contains(&size_bytes) {
            return Err(DomainError::Validation {
                field: "message_content_size",
                reason: "必须在 1 字节到 25 MiB 之间",
            });
        }
        Ok(Self {
            content_id,
            digest,
            size_bytes,
            client_encryption: None,
        })
    }

    #[must_use]
    pub fn with_client_encryption(mut self, encryption: ClientContentEncryption) -> Self {
        self.client_encryption = Some(encryption);
        self
    }

    pub const fn content_id(&self) -> ContentId {
        self.content_id
    }

    pub const fn digest(&self) -> Sha256Digest {
        self.digest
    }

    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    pub const fn client_encryption(&self) -> Option<&ClientContentEncryption> {
        self.client_encryption.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRelation {
    ReplyTo(MessageId),
}

fn validate_bounded_text(
    field: &'static str,
    value: &str,
    maximum_characters: usize,
) -> DomainResult<()> {
    if value.is_empty()
        || value.chars().count() > maximum_characters
        || value.chars().any(char::is_control)
    {
        return Err(DomainError::Validation {
            field,
            reason: "不能为空、超长或包含控制字符",
        });
    }
    Ok(())
}

fn language_part_is_valid(index: usize, part: &str) -> bool {
    let valid_length = if index == 0 {
        (2..=8).contains(&part.len())
    } else {
        (1..=8).contains(&part.len())
    };
    valid_length
        && if index == 0 {
            part.bytes().all(|byte| byte.is_ascii_alphabetic())
        } else {
            part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        }
}

#[cfg(test)]
mod tests {
    use super::{
        MessageLanguage, MessageRiskFlag, MessageRiskFlags, MessageSensitivity, MessageSummary,
        MessageTitle,
    };

    #[test]
    fn 文本边界按_unicode_字符而不是_utf8_字节计算() {
        assert!(MessageTitle::new("界".repeat(120)).is_ok());
        assert!(MessageTitle::new("界".repeat(121)).is_err());
        assert!(MessageSummary::new("界".repeat(500)).is_ok());
        assert!(MessageSummary::new("界".repeat(501)).is_err());
    }

    #[test]
    fn 语言与风险标签拒绝宽松格式() {
        assert!(MessageLanguage::new("zh-CN").is_ok());
        assert!(MessageLanguage::new("中文").is_err());
        assert!(MessageRiskFlag::new("untrusted_instructions").is_ok());
        assert!(MessageRiskFlag::new("Untrusted-Instructions").is_err());
    }

    #[test]
    fn 风险标签集合去重并执行硬上限() {
        let repeated = MessageRiskFlag::new("external_links").expect("标签有效");
        let flags = MessageRiskFlags::new([repeated.clone(), repeated]).expect("重复标签可去重");
        assert_eq!(flags.iter().count(), 1);

        let overflow =
            (0..17).map(|index| MessageRiskFlag::new(format!("risk_{index}")).expect("标签有效"));
        assert!(MessageRiskFlags::new(overflow).is_err());
        assert_eq!(MessageSensitivity::Normal.as_str(), "normal");
    }
}
