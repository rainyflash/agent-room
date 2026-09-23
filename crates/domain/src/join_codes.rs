//! 私人房间的 Agent 口令：房主生成、交给 Agent，Agent 凭它以“Agent 成员”身份进入房间。
//!
//! 口令是 12 位 Crockford Base32（约 60 位熵），给人看时分三组 `XXXX-XXXX-XXXX`。只在生成时显示
//! 一次，服务器只存摘要。输入时容忍大小写、空白和连字符，并把易混的 O 当作 0、I 和 L 当作 1。

use std::fmt;

use crate::{DomainError, DomainResult};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const LENGTH: usize = 12;
/// 输入里最多容忍这么多字符（含分隔），超出直接拒绝，不做逐字处理。
const MAX_INPUT: usize = 64;

#[derive(Clone, PartialEq, Eq)]
pub struct PrivateRoomJoinCode(String);

impl PrivateRoomJoinCode {
    /// 用随机数生成口令，只取前 60 位。
    #[must_use]
    pub fn from_entropy(entropy: [u8; 8]) -> Self {
        let mut value = u64::from_be_bytes(entropy);
        let mut code = String::with_capacity(LENGTH);
        for _ in 0..LENGTH {
            let index = u8::try_from(value % 32).map_or(0, usize::from);
            code.push(char::from(ALPHABET[index]));
            value /= 32;
        }
        Self(code)
    }

    /// 解析房主转交、Agent 输入的口令。
    ///
    /// # Errors
    ///
    /// 去掉分隔后不是 12 个 Crockford Base32 字符时返回错误。
    pub fn parse(input: &str) -> DomainResult<Self> {
        if input.len() > MAX_INPUT {
            return Err(invalid());
        }
        let mut code = String::with_capacity(LENGTH);
        for character in input.chars() {
            let normalized = match character.to_ascii_uppercase() {
                '-' | ' ' => continue,
                'O' => '0',
                'I' | 'L' => '1',
                other => other,
            };
            if !u8::try_from(normalized).is_ok_and(|byte| ALPHABET.contains(&byte)) {
                return Err(invalid());
            }
            code.push(normalized);
        }
        if code.len() != LENGTH {
            return Err(invalid());
        }
        Ok(Self(code))
    }

    /// 规范形式：12 个大写字符、不带分隔。服务器按它计算摘要。
    #[must_use]
    pub fn normalized(&self) -> &str {
        &self.0
    }

    /// 给人看的形式：`XXXX-XXXX-XXXX`。
    #[must_use]
    pub fn display(&self) -> String {
        format!("{}-{}-{}", &self.0[..4], &self.0[4..8], &self.0[8..])
    }
}

impl fmt::Debug for PrivateRoomJoinCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PrivateRoomJoinCode([已脱敏])")
    }
}

const fn invalid() -> DomainError {
    DomainError::Validation {
        field: "join_code",
        reason: "口令应为 12 个字母或数字",
    }
}

/// 凭口令进来的 Agent 成员的状态。被移出后只有用移出之后生成的新口令才能再进来。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateRoomAgentMemberStatus {
    Joined,
    Removed,
}

impl PrivateRoomAgentMemberStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Joined => "joined",
            Self::Removed => "removed",
        }
    }

    /// # Errors
    ///
    /// 不是已知状态时返回错误。
    pub fn parse(value: &str) -> DomainResult<Self> {
        match value {
            "joined" => Ok(Self::Joined),
            "removed" => Ok(Self::Removed),
            _ => Err(DomainError::Validation {
                field: "private_room_agent_member_status",
                reason: "未知的 Agent 成员状态",
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 生成的口令是十二位_crockford_字符且随随机数变化() {
        let first =
            PrivateRoomJoinCode::from_entropy([0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0]);
        let second =
            PrivateRoomJoinCode::from_entropy([0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf1]);
        for code in [&first, &second] {
            assert_eq!(code.normalized().len(), 12);
            assert!(
                code.normalized()
                    .bytes()
                    .all(|byte| ALPHABET.contains(&byte))
            );
        }
        assert_ne!(first, second);
        let display = first.display();
        assert_eq!(display.len(), 14);
        assert_eq!(PrivateRoomJoinCode::parse(&display).unwrap(), first);
        assert_eq!(
            PrivateRoomJoinCode::from_entropy([0; 8]).display(),
            "0000-0000-0000"
        );
    }

    #[test]
    fn 输入容忍大小写分隔和易混字符() {
        let code = PrivateRoomJoinCode::parse("K7P3-Q9XW-2DMA").unwrap();
        assert_eq!(code.normalized(), "K7P3Q9XW2DMA");
        for input in ["k7p3 q9xw 2dma", " K7P3Q9XW2DMA ", "k7p3-q9xw-2dma"] {
            assert_eq!(PrivateRoomJoinCode::parse(input).unwrap(), code);
        }
        assert_eq!(
            PrivateRoomJoinCode::parse("OIL0-0000-0000")
                .unwrap()
                .normalized(),
            "011000000000"
        );
    }

    #[test]
    fn 拒绝长度不对_非字母数字_保留字符和超长输入() {
        for input in [
            "",
            "K7P3-Q9XW-2DM",
            "K7P3-Q9XW-2DMAA",
            "K7P3-Q9XW-2DMU",
            "K7P3_Q9XW_2DMA",
            "口令口令-Q9XW-2DMA",
            &"-".repeat(65),
        ] {
            assert!(PrivateRoomJoinCode::parse(input).is_err(), "{input}");
        }
    }

    #[test]
    fn 调试输出不泄露口令() {
        let code = PrivateRoomJoinCode::parse("K7P3-Q9XW-2DMA").unwrap();
        assert!(!format!("{code:?}").contains("K7P3"));
    }

    #[test]
    fn agent_成员状态可往返() {
        for status in [
            PrivateRoomAgentMemberStatus::Joined,
            PrivateRoomAgentMemberStatus::Removed,
        ] {
            assert_eq!(
                PrivateRoomAgentMemberStatus::parse(status.as_str()).unwrap(),
                status
            );
        }
        assert!(PrivateRoomAgentMemberStatus::parse("banned").is_err());
    }
}
