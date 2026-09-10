use std::fmt;

use crate::{DomainError, DomainResult};

pub const AUTOMATION_MAX_TEXT_BYTES: usize = 48 * 1_024;

/// Immutable, bounded plaintext sent to the server's scanner before publication.
#[derive(Clone, PartialEq, Eq)]
pub struct AutomationMessageText(String);

impl AutomationMessageText {
    /// # Errors
    /// Rejects empty messages and bodies above the automatic-message limit.
    pub fn new(text: String) -> DomainResult<Self> {
        if text.is_empty() || text.len() > AUTOMATION_MAX_TEXT_BYTES {
            return Err(DomainError::Validation {
                field: "automation_message_text",
                reason: "自动消息正文必须为 1 到 49152 字节",
            });
        }
        Ok(Self(text))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AutomationMessageText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AutomationMessageText")
            .field("bytes", &self.0.len())
            .finish_non_exhaustive()
    }
}
