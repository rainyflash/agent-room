//! 私人房间的邀请链接（`specs/room-people/design.md`）：拿到链接的人登录后就能进来。
//!
//! 链接里的秘密是 128 位随机数，写成 26 个小写 Crockford Base32 字符，放在 `/join#` 后面，浏览器
//! 不会把这一段发给服务器，也不进 Referer。服务器按摘要认，另存一份封存的给有邀请权限的人再复制。

use std::fmt;

use crate::{
    DomainError, DomainResult,
    time::{DurationMillis, UtcMillis},
};

const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
/// 128 位每 5 位一个字符，最后一个字符只用到 3 位。
const LENGTH: usize = 26;
/// 输入里最多容忍这么多字符（含首尾空白），超出直接拒绝，不做逐字处理。
const MAX_INPUT: usize = 64;

/// 邀请链接 7 天后过期，有效期内谁拿到都能用。
const LIFETIME_MILLIS: u64 = 7 * 24 * 60 * 60 * 1_000;

/// 在 `created_at` 生成的链接什么时候过期。
///
/// # Errors
///
/// 时间溢出时返回错误。
pub fn invite_link_expires_at(created_at: UtcMillis) -> DomainResult<UtcMillis> {
    created_at.checked_add(DurationMillis::new(LIFETIME_MILLIS)?)
}

#[derive(Clone, PartialEq, Eq)]
pub struct PrivateRoomInviteSecret(String);

impl PrivateRoomInviteSecret {
    #[must_use]
    pub fn from_entropy(entropy: [u8; 16]) -> Self {
        let mut value = u128::from_be_bytes(entropy);
        let mut secret = String::with_capacity(LENGTH);
        for _ in 0..LENGTH {
            let index = usize::try_from(value & 31).unwrap_or(0);
            secret.push(char::from(ALPHABET[index]));
            value >>= 5;
        }
        Self(secret)
    }

    /// 解析网页从链接里读出来的秘密。容忍首尾空白和大写（有人会手抄）。
    ///
    /// # Errors
    ///
    /// 不是 26 个 Crockford Base32 字符时返回错误。
    pub fn parse(input: &str) -> DomainResult<Self> {
        if input.len() > MAX_INPUT {
            return Err(invalid());
        }
        let secret = input.trim().to_ascii_lowercase();
        if secret.len() != LENGTH || !secret.bytes().all(|byte| ALPHABET.contains(&byte)) {
            return Err(invalid());
        }
        Ok(Self(secret))
    }

    /// 规范形式：26 个小写字符。服务器按它计算摘要，链接里也写这个。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PrivateRoomInviteSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PrivateRoomInviteSecret([已脱敏])")
    }
}

const fn invalid() -> DomainError {
    DomainError::Validation {
        field: "invite_secret",
        reason: "邀请链接里的秘密应为 26 个字母或数字",
    }
}

#[cfg(test)]
mod tests {
    use super::{PrivateRoomInviteSecret, invite_link_expires_at};
    use crate::time::UtcMillis;

    #[test]
    fn 秘密是_26_个小写字符_解析容忍大写和空白() {
        let secret = PrivateRoomInviteSecret::from_entropy([0xA5; 16]);
        assert_eq!(secret.as_str().len(), 26);
        assert!(
            secret
                .as_str()
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
        );

        let shouted = format!("  {}\n", secret.as_str().to_ascii_uppercase());
        assert_eq!(
            PrivateRoomInviteSecret::parse(&shouted).expect("大写和空白可以"),
            secret
        );
        assert_ne!(
            PrivateRoomInviteSecret::from_entropy([0x5A; 16]),
            secret,
            "不同的随机数给出不同的秘密"
        );
    }

    #[test]
    fn 长度不对或有不认识的字符就拒绝() {
        let secret = PrivateRoomInviteSecret::from_entropy([7; 16]);
        for input in [
            "",
            &secret.as_str()[..25],
            &format!("{}0", secret.as_str()),
            "uuuuuuuuuuuuuuuuuuuuuuuuuu",
            &"0".repeat(65),
        ] {
            assert!(PrivateRoomInviteSecret::parse(input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn 秘密不进调试输出_链接七天过期() {
        let secret = PrivateRoomInviteSecret::from_entropy([1; 16]);
        assert!(!format!("{secret:?}").contains(secret.as_str()));
        let created = UtcMillis::new(1_000).expect("时间有效");
        assert_eq!(
            invite_link_expires_at(created).expect("不溢出").value(),
            1_000 + 7 * 24 * 60 * 60 * 1_000
        );
    }
}
