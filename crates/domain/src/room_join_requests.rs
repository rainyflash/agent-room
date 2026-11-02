//! 申请进公开房间（`specs/public-rooms/design.md`）：要申请的房间点“申请加入”，可以留一句话；
//! 房主和管理员批准或拒绝。同一个人对同一间房同时只能有一个在等的申请，被拒的 24 小时内不能再
//! 申请；没人处理的一直等着（维护者定），处理完的 30 天后删掉。

use crate::{DomainError, DomainResult};

/// 申请时最多留多少字（按字符算，不按字节）。
pub const MAX_JOIN_REQUEST_MESSAGE_CHARS: usize = 200;

/// 被拒以后多久才能再申请。
pub const JOIN_REQUEST_REJECTION_COOLDOWN_MILLIS: i64 = 24 * 60 * 60 * 1_000;

/// 处理完（批准、拒绝、作废、撤回）的申请留多久，隐私说明页照这个写。
pub const JOIN_REQUEST_RETENTION_MILLIS: i64 = 30 * 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinRequestStatus {
    /// 等着房主或管理员处理。
    Pending,
    Approved,
    Rejected,
    /// 房间改回私密或者归档了，没处理的一起作废。
    Voided,
    /// 申请人自己撤回了。
    Withdrawn,
}

impl JoinRequestStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Voided => "voided",
            Self::Withdrawn => "withdrawn",
        }
    }
}

impl TryFrom<&str> for JoinRequestStatus {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "pending" => Ok(Self::Pending),
            "approved" => Ok(Self::Approved),
            "rejected" => Ok(Self::Rejected),
            "voided" => Ok(Self::Voided),
            "withdrawn" => Ok(Self::Withdrawn),
            _ => Err(DomainError::Validation {
                field: "join_request_status",
                reason: "不认识的申请状态",
            }),
        }
    }
}

/// 申请时留的一句话：可以不留；去掉首尾空白后最多 200 字，能换行，不收回车和别的控制字符。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JoinRequestMessage(String);

impl JoinRequestMessage {
    /// # Errors
    ///
    /// 超过 200 字，或者带着换行以外的控制字符时返回错误。
    pub fn parse(value: &str) -> DomainResult<Self> {
        let value = value.trim();
        if value.chars().count() > MAX_JOIN_REQUEST_MESSAGE_CHARS {
            return Err(DomainError::Validation {
                field: "join_request_message",
                reason: "申请留言最多 200 字",
            });
        }
        if value
            .chars()
            .any(|character| character.is_control() && character != '\n')
        {
            return Err(DomainError::Validation {
                field: "join_request_message",
                reason: "申请留言只能换行，不收别的控制字符",
            });
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 留言去掉首尾空白_最多两百字_能换行不收回车() {
        assert_eq!(JoinRequestMessage::parse("").unwrap().as_str(), "");
        assert_eq!(
            JoinRequestMessage::parse("  想来听听\n大家怎么用  ")
                .unwrap()
                .as_str(),
            "想来听听\n大家怎么用"
        );
        let longest = "字".repeat(MAX_JOIN_REQUEST_MESSAGE_CHARS);
        assert!(
            JoinRequestMessage::parse(&longest).is_ok(),
            "按字符算，不按字节"
        );
        assert!(JoinRequestMessage::parse(&format!("{longest}多")).is_err());
        assert!(JoinRequestMessage::parse("第一行\r\n第二行").is_err());
        assert!(JoinRequestMessage::parse("响铃\u{7}").is_err());
    }

    #[test]
    fn 状态按名字往返() {
        for status in [
            JoinRequestStatus::Pending,
            JoinRequestStatus::Approved,
            JoinRequestStatus::Rejected,
            JoinRequestStatus::Voided,
            JoinRequestStatus::Withdrawn,
        ] {
            assert_eq!(
                JoinRequestStatus::try_from(status.as_str()).unwrap(),
                status
            );
        }
        assert!(JoinRequestStatus::try_from("cancelled").is_err());
    }
}
