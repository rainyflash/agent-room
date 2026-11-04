//! 房间的常驻公告（`specs/public-rooms/design.md`“常驻公告”）：房主和管理员写的一段纯文本，一直挂在
//! 房间顶上，每间房只有一条，新写的盖掉旧的，清空就是撤掉。公开的、私密的房间都有，公共大厅没有。

use crate::{DomainError, DomainResult};

/// 公告最多写多少个字（按 Unicode 字符数，和协议里 `RoomNoticeState.text` 的 `maxLength` 一样）。
pub const MAX_ROOM_NOTICE_CHARACTERS: usize = 2_000;

/// 公告的正文：去掉首尾空白，最多 2000 个字；能换行、能有制表符，不收回车和别的控制字符，和消息
/// 正文一样。空的就是撤掉公告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomNoticeText(String);

impl RoomNoticeText {
    /// 解析房主或管理员写的公告。
    ///
    /// # Errors
    ///
    /// 超过 2000 个字，或带着换行和制表符以外的控制字符时返回错误。
    pub fn parse(input: &str) -> DomainResult<Self> {
        let text = input.trim();
        if text.chars().count() > MAX_ROOM_NOTICE_CHARACTERS
            || text
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
        {
            return Err(DomainError::Validation {
                field: "room_notice",
                reason: "公告最多 2000 个字，只能带换行和制表符这两种控制字符",
            });
        }
        Ok(Self(text.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 空的公告就是撤掉。
    #[must_use]
    pub fn is_removal(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 去掉首尾空白_能换行和制表符_空的是撤掉() {
        let notice = RoomNoticeText::parse("  只在被点名时说话。\n\t回复要短。\n ").unwrap();
        assert_eq!(notice.as_str(), "只在被点名时说话。\n\t回复要短。");
        assert!(!notice.is_removal());
        assert!(RoomNoticeText::parse(" \n ").unwrap().is_removal());
    }

    #[test]
    fn 回车和别的控制字符不收() {
        for input in ["第一行\r\n第二行", "响铃\u{7}", "退格\u{8}"] {
            assert!(RoomNoticeText::parse(input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn 按字数算_两千个字可以_多一个不行() {
        let longest = "公".repeat(MAX_ROOM_NOTICE_CHARACTERS);
        assert!(RoomNoticeText::parse(&longest).is_ok());
        assert!(RoomNoticeText::parse(&format!("{longest}告")).is_err());
    }
}
