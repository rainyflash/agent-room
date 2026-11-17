//! 分享一段对话（`specs/redesign/project-rooms.md`“分享一段对话”）：放着实况的房间里，房间里的人能把
//! 连着的最多 30 句分享成一个链接 `/r/<房间分享码>/s/<段码>`，打开不用登录。服务器只记是哪几句
//! （Matrix 事件 ID）、哪间房、谁分享的、什么时候，不存话。

use crate::{DomainError, DomainResult, room_listing::SHARE_CODE_ALPHABET};

/// 一段最多这么多句。
pub const MAX_EXCERPT_MESSAGES: usize = 30;
const EXCERPT_CODE_LENGTH: usize = 8;

/// 一段对话的段码：8 位大小写字母和数字（约 218 万亿种），和房间的分享码同一套字符。它不是秘密：
/// 打开只看得到分享页上本来就放着的话。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RoomExcerptCode(String);

impl RoomExcerptCode {
    /// 由 128 位随机数取 8 位。62 的 8 次方远小于 2 的 128 次方，取余带来的偏差可以不计。
    #[must_use]
    pub fn from_entropy(entropy: [u8; 16]) -> Self {
        let mut value = u128::from_be_bytes(entropy);
        let mut code = String::with_capacity(EXCERPT_CODE_LENGTH);
        for _ in 0..EXCERPT_CODE_LENGTH {
            let index = usize::try_from(value % 62).unwrap_or(0);
            code.push(char::from(SHARE_CODE_ALPHABET[index]));
            value /= 62;
        }
        Self(code)
    }

    /// 解析地址里的段码。大小写有区别，不改写。
    ///
    /// # Errors
    ///
    /// 不是 8 位字母或数字时返回错误。
    pub fn parse(input: &str) -> DomainResult<Self> {
        if input.len() != EXCERPT_CODE_LENGTH
            || !input.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(DomainError::Validation {
                field: "room_excerpt_code",
                reason: "段码应为 8 位字母或数字",
            });
        }
        Ok(Self(input.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::RoomExcerptCode;

    #[test]
    fn 段码是_8_位字母或数字_大小写有区别() {
        let code = RoomExcerptCode::from_entropy([0xA5; 16]);
        assert_eq!(code.as_str().len(), 8);
        assert!(
            code.as_str()
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric())
        );
        assert_eq!(RoomExcerptCode::parse(code.as_str()).expect("能解析"), code);
        assert_ne!(RoomExcerptCode::from_entropy([0x5A; 16]), code);
        assert_eq!(RoomExcerptCode::from_entropy([0; 16]).as_str(), "00000000");

        assert_ne!(
            RoomExcerptCode::parse("aB3xYz12").expect("能解析"),
            RoomExcerptCode::parse("AB3XYZ12").expect("能解析")
        );
        for input in [
            "",
            "aB3xYz1",
            "aB3xYz123",
            "aB3x_z12",
            "aB3xYz1 ",
            "ä1234567",
            "../x/abc",
        ] {
            assert!(RoomExcerptCode::parse(input).is_err(), "{input:?}");
        }
    }
}
