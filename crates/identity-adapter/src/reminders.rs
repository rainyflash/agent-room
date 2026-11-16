//! 提醒邮件的退订令牌（`specs/redesign/reminders.md` 的“退订”）：账户 ID 后面接 HMAC-SHA256 签名，
//! 整个用 URL 安全的 Base64 写成 64 个字符。签名里带着用途，和别处用同一把钥匙签的东西对不上。
//!
//! 令牌不过期，可以反复用：退订以后再点还是“已经退了”。钥匙在部署的 `secrets/` 下；换了钥匙，以前
//! 发出去的退订链接就都不认了，邮件里的“去设置里改”照样能用。

use agent_room_application::ports::ReminderUnsubscribeTokens;
use agent_room_domain::ids::PrincipalId;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac as _};
use sha2::Sha256;
use thiserror::Error;
use uuid::Uuid;

const PURPOSE: &[u8] = b"agent-room/reminder-unsubscribe/v1\0";
const KEY_BYTES: usize = 32;
const ID_BYTES: usize = 16;
const TAG_BYTES: usize = 32;
/// 48 字节写成不带填充的 Base64。
const TOKEN_CHARS: usize = 64;

pub struct HmacReminderUnsubscribeTokens {
    /// 已经装好钥匙的 HMAC：每次签名、核对都从它复制一份。
    keyed: Hmac<Sha256>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ReminderTokenConfigurationError {
    #[error("退订令牌的钥匙必须至少包含 256 位熵")]
    WeakKey,
}

impl HmacReminderUnsubscribeTokens {
    /// # Errors
    ///
    /// 钥匙少于 256 位时拒绝启动。
    pub fn new(key: &[u8]) -> Result<Self, ReminderTokenConfigurationError> {
        if key.len() < KEY_BYTES {
            return Err(ReminderTokenConfigurationError::WeakKey);
        }
        let keyed = <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(key)
            .map_err(|_| ReminderTokenConfigurationError::WeakKey)?;
        Ok(Self { keyed })
    }

    fn signer(&self, id: &[u8]) -> Hmac<Sha256> {
        let mut mac = self.keyed.clone();
        mac.update(PURPOSE);
        mac.update(id);
        mac
    }
}

impl ReminderUnsubscribeTokens for HmacReminderUnsubscribeTokens {
    fn issue(&self, principal_id: PrincipalId) -> String {
        let id = principal_id.as_uuid();
        let mut bytes = Vec::with_capacity(ID_BYTES + TAG_BYTES);
        bytes.extend_from_slice(id.as_bytes());
        bytes.extend_from_slice(&self.signer(id.as_bytes()).finalize().into_bytes());
        URL_SAFE_NO_PAD.encode(bytes)
    }

    fn verify(&self, token: &str) -> Option<PrincipalId> {
        if token.len() != TOKEN_CHARS {
            return None;
        }
        let bytes = URL_SAFE_NO_PAD.decode(token).ok()?;
        let (id, tag) = bytes.split_at_checked(ID_BYTES)?;
        if tag.len() != TAG_BYTES {
            return None;
        }
        // 按常数时间比对签名。
        self.signer(id).verify_slice(tag).ok()?;
        Some(PrincipalId::from_uuid(Uuid::from_slice(id).ok()?))
    }
}

#[cfg(test)]
mod tests {
    use agent_room_application::ports::ReminderUnsubscribeTokens as _;
    use agent_room_domain::ids::PrincipalId;
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use uuid::Uuid;

    use super::{HmacReminderUnsubscribeTokens, ReminderTokenConfigurationError};

    fn tokens(byte: u8) -> HmacReminderUnsubscribeTokens {
        HmacReminderUnsubscribeTokens::new(&[byte; 32]).unwrap()
    }

    #[test]
    fn 签出来的令牌认得回同一个账户_每次都一样() {
        let principal = PrincipalId::from_uuid(Uuid::now_v7());
        let signer = tokens(7);
        let token = signer.issue(principal);
        assert_eq!(token.len(), 64);
        assert!(
            token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        assert_eq!(signer.issue(principal), token);
        assert_eq!(signer.verify(&token), Some(principal));
    }

    #[test]
    fn 换了钥匙_改了一个字_换了账户都不认() {
        let principal = PrincipalId::from_uuid(Uuid::now_v7());
        let token = tokens(7).issue(principal);
        assert_eq!(tokens(8).verify(&token), None);

        let mut bytes = URL_SAFE_NO_PAD.decode(&token).unwrap();
        bytes[40] ^= 1;
        assert_eq!(tokens(7).verify(&URL_SAFE_NO_PAD.encode(&bytes)), None);

        // 把别人的账户 ID 拼到这个签名前面。
        let other = PrincipalId::from_uuid(Uuid::now_v7());
        let mut forged = URL_SAFE_NO_PAD.decode(&token).unwrap();
        forged[..16].copy_from_slice(other.as_uuid().as_bytes());
        assert_eq!(tokens(7).verify(&URL_SAFE_NO_PAD.encode(&forged)), None);
    }

    #[test]
    fn 格式不对的一律不认() {
        let signer = tokens(7);
        for token in ["", "abc", &"A".repeat(64), &"!".repeat(64), &"A".repeat(65)] {
            assert_eq!(signer.verify(token), None, "{token}");
        }
    }

    #[test]
    fn 钥匙太短不让启动() {
        assert_eq!(
            HmacReminderUnsubscribeTokens::new(&[1; 31]).err(),
            Some(ReminderTokenConfigurationError::WeakKey)
        );
    }
}
