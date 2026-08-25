use std::{collections::HashMap, fmt};

use agent_room_application::ports::{
    ContentReadTicket, ContentReadTicketClaims, ContentReadTicketCodec, ContentTicketFailure,
    ContentTicketFailureKind, ContentTicketResult, MatrixEventId, MatrixRoomId, PortFuture,
    SecretValue,
};
use agent_room_domain::{
    content::{ContentByteLength, ContentMediaType, Sha256Digest},
    ids::{AgentId, ContentId, PrincipalId},
    time::UtcMillis,
};
use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, decode_header, encode,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::encoding::{decode_sha256, lower_hex};

const TOKEN_ISSUER: &str = "agent-room-control-plane";
const TOKEN_AUDIENCE: &str = "agent-room-content";
const TOKEN_TYPE: &str = "ARCT";
const MAX_KEY_ID_LENGTH: usize = 64;
const MINIMUM_SECRET_BYTES: usize = 32;
const MAXIMUM_VERIFICATION_KEYS: usize = 4;
const MAXIMUM_TICKET_LIFETIME_MILLIS: u64 = 5 * 60 * 1_000;

#[derive(Clone)]
pub struct ContentTicketSigningKey {
    key_id: String,
    secret: SecretValue,
}

impl ContentTicketSigningKey {
    /// 创建带公开轮换标识的 HMAC 签名密钥。
    ///
    /// # Errors
    ///
    /// 密钥编号含不安全字符或密钥熵长度不足时返回错误。
    pub fn new(
        key_id: impl Into<String>,
        secret: SecretValue,
    ) -> Result<Self, ContentTicketCodecConfigError> {
        let key_id = key_id.into();
        let valid_key_id = !key_id.is_empty()
            && key_id.len() <= MAX_KEY_ID_LENGTH
            && key_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
        if !valid_key_id {
            return Err(ContentTicketCodecConfigError::InvalidKeyId);
        }
        if secret.expose().len() < MINIMUM_SECRET_BYTES {
            return Err(ContentTicketCodecConfigError::WeakSecret);
        }
        Ok(Self { key_id, secret })
    }

    pub fn key_id(&self) -> &str {
        &self.key_id
    }
}

impl fmt::Debug for ContentTicketSigningKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContentTicketSigningKey")
            .field("key_id", &self.key_id)
            .field("secret", &"[已脱敏]")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ContentTicketCodecConfigError {
    #[error("内容票据密钥编号非法")]
    InvalidKeyId,
    #[error("内容票据签名密钥少于 32 字节")]
    WeakSecret,
    #[error("内容票据密钥编号重复")]
    DuplicateKeyId,
    #[error("内容票据校验密钥超过轮换上限")]
    TooManyKeys,
}

pub struct HmacContentReadTicketCodec {
    active_key_id: String,
    keys: HashMap<String, SecretValue>,
}

impl HmacContentReadTicketCodec {
    /// 创建支持有限旧密钥窗口的短期票据编解码器。
    ///
    /// # Errors
    ///
    /// 密钥编号重复或总密钥数超过四个时返回错误。
    pub fn new(
        active: ContentTicketSigningKey,
        previous: impl IntoIterator<Item = ContentTicketSigningKey>,
    ) -> Result<Self, ContentTicketCodecConfigError> {
        let active_key_id = active.key_id.clone();
        let mut keys = HashMap::from([(active.key_id, active.secret)]);
        for key in previous {
            if keys.len() >= MAXIMUM_VERIFICATION_KEYS {
                return Err(ContentTicketCodecConfigError::TooManyKeys);
            }
            if keys.insert(key.key_id, key.secret).is_some() {
                return Err(ContentTicketCodecConfigError::DuplicateKeyId);
            }
        }
        Ok(Self {
            active_key_id,
            keys,
        })
    }

    fn issue_internal(
        &self,
        claims: &ContentReadTicketClaims,
    ) -> ContentTicketResult<ContentReadTicket> {
        let active_secret = self.keys.get(&self.active_key_id).ok_or_else(|| {
            ContentTicketFailure::new(
                "content.ticket.issue",
                ContentTicketFailureKind::Unavailable,
            )
        })?;
        let wire = WireTicketClaims::from_domain(claims)?;
        let mut header = Header::new(Algorithm::HS256);
        header.typ = Some(TOKEN_TYPE.to_owned());
        header.kid = Some(self.active_key_id.clone());
        let encoded = encode(
            &header,
            &wire,
            &EncodingKey::from_secret(active_secret.expose().as_bytes()),
        )
        .map_err(|_| {
            ContentTicketFailure::new(
                "content.ticket.issue",
                ContentTicketFailureKind::Unavailable,
            )
        })?;
        ContentReadTicket::new(encoded).map_err(|_| invalid_ticket("content.ticket.issue"))
    }

    fn verify_internal(
        &self,
        ticket: &ContentReadTicket,
        expected_principal_id: PrincipalId,
        now: UtcMillis,
    ) -> ContentTicketResult<ContentReadTicketClaims> {
        let header = decode_header(ticket.expose())
            .map_err(|_| invalid_ticket("content.ticket.verify.header"))?;
        if header.alg != Algorithm::HS256 || header.typ.as_deref() != Some(TOKEN_TYPE) {
            return Err(invalid_ticket("content.ticket.verify.header"));
        }
        let key_id = header
            .kid
            .as_deref()
            .ok_or_else(|| invalid_ticket("content.ticket.verify.key"))?;
        let secret = self
            .keys
            .get(key_id)
            .ok_or_else(|| invalid_ticket("content.ticket.verify.key"))?;
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = false;
        validation.set_audience(&[TOKEN_AUDIENCE]);
        validation.set_issuer(&[TOKEN_ISSUER]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        let decoded = decode::<WireTicketClaims>(
            ticket.expose(),
            &DecodingKey::from_secret(secret.expose().as_bytes()),
            &validation,
        )
        .map_err(|_| invalid_ticket("content.ticket.verify.signature"))?;
        decoded.claims.into_domain(expected_principal_id, now)
    }
}

impl fmt::Debug for HmacContentReadTicketCodec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut key_ids = self.keys.keys().collect::<Vec<_>>();
        key_ids.sort_unstable();
        formatter
            .debug_struct("HmacContentReadTicketCodec")
            .field("active_key_id", &self.active_key_id)
            .field("verification_key_ids", &key_ids)
            .finish()
    }
}

impl ContentReadTicketCodec for HmacContentReadTicketCodec {
    fn issue<'a>(
        &'a self,
        claims: &'a ContentReadTicketClaims,
    ) -> PortFuture<'a, ContentTicketResult<ContentReadTicket>> {
        Box::pin(async move { self.issue_internal(claims) })
    }

    fn verify<'a>(
        &'a self,
        ticket: &'a ContentReadTicket,
        expected_principal_id: PrincipalId,
        now: UtcMillis,
    ) -> PortFuture<'a, ContentTicketResult<ContentReadTicketClaims>> {
        Box::pin(async move { self.verify_internal(ticket, expected_principal_id, now) })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WireTicketClaims {
    iss: String,
    aud: String,
    sub: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    actor_agent_id: Option<String>,
    exp: u64,
    iat: u64,
    issued_at_ms: u64,
    expires_at_ms: u64,
    content_id: String,
    matrix_room_id: String,
    matrix_event_id: String,
    sha256: String,
    byte_length: u64,
    media_type: String,
}

impl WireTicketClaims {
    fn from_domain(claims: &ContentReadTicketClaims) -> ContentTicketResult<Self> {
        let issued_at_ms = u64::try_from(claims.issued_at.value())
            .map_err(|_| invalid_ticket("content.ticket.issue.claims"))?;
        let expires_at_ms = u64::try_from(claims.expires_at.value())
            .map_err(|_| invalid_ticket("content.ticket.issue.claims"))?;
        validate_lifetime(issued_at_ms, expires_at_ms)
            .map_err(|()| invalid_ticket("content.ticket.issue.claims"))?;
        Ok(Self {
            iss: TOKEN_ISSUER.to_owned(),
            aud: TOKEN_AUDIENCE.to_owned(),
            sub: claims.principal_id.to_string(),
            actor_agent_id: claims.actor_agent_id.map(|value| value.to_string()),
            exp: expires_at_ms.div_ceil(1_000),
            iat: issued_at_ms / 1_000,
            issued_at_ms,
            expires_at_ms,
            content_id: claims.content_id.to_string(),
            matrix_room_id: claims.matrix_room_id.as_str().to_owned(),
            matrix_event_id: claims.matrix_event_id.as_str().to_owned(),
            sha256: lower_hex(claims.digest.as_bytes()),
            byte_length: claims.byte_length.value(),
            media_type: claims.media_type.as_str().to_owned(),
        })
    }

    fn into_domain(
        self,
        expected_principal_id: PrincipalId,
        now: UtcMillis,
    ) -> ContentTicketResult<ContentReadTicketClaims> {
        let principal_id = parse_principal_id(&self.sub)?;
        if principal_id != expected_principal_id {
            return Err(ContentTicketFailure::new(
                "content.ticket.verify.audience",
                ContentTicketFailureKind::AudienceMismatch,
            ));
        }
        validate_lifetime(self.issued_at_ms, self.expires_at_ms)
            .map_err(|()| invalid_ticket("content.ticket.verify.time"))?;
        let now =
            u64::try_from(now.value()).map_err(|_| invalid_ticket("content.ticket.verify.time"))?;
        if now >= self.expires_at_ms {
            return Err(ContentTicketFailure::new(
                "content.ticket.verify.time",
                ContentTicketFailureKind::Expired,
            ));
        }
        if self.issued_at_ms > now
            || self.iat != self.issued_at_ms / 1_000
            || self.exp != self.expires_at_ms.div_ceil(1_000)
        {
            return Err(invalid_ticket("content.ticket.verify.time"));
        }
        Ok(ContentReadTicketClaims {
            principal_id,
            actor_agent_id: self
                .actor_agent_id
                .as_deref()
                .map(parse_agent_id)
                .transpose()?,
            content_id: parse_content_id(&self.content_id)?,
            matrix_room_id: MatrixRoomId::new(self.matrix_room_id)
                .map_err(|_| invalid_ticket("content.ticket.verify.claims"))?,
            matrix_event_id: MatrixEventId::new(self.matrix_event_id)
                .map_err(|_| invalid_ticket("content.ticket.verify.claims"))?,
            digest: Sha256Digest::from_bytes(
                decode_sha256(&self.sha256)
                    .ok_or_else(|| invalid_ticket("content.ticket.verify.claims"))?,
            ),
            byte_length: ContentByteLength::new(self.byte_length)
                .map_err(|_| invalid_ticket("content.ticket.verify.claims"))?,
            media_type: ContentMediaType::new(self.media_type)
                .map_err(|_| invalid_ticket("content.ticket.verify.claims"))?,
            issued_at: parse_time(self.issued_at_ms)?,
            expires_at: parse_time(self.expires_at_ms)?,
        })
    }
}

fn validate_lifetime(issued_at_ms: u64, expires_at_ms: u64) -> Result<(), ()> {
    let lifetime = expires_at_ms.checked_sub(issued_at_ms).ok_or(())?;
    if lifetime == 0 || lifetime > MAXIMUM_TICKET_LIFETIME_MILLIS {
        return Err(());
    }
    Ok(())
}

fn parse_principal_id(value: &str) -> ContentTicketResult<PrincipalId> {
    Uuid::parse_str(value)
        .map(PrincipalId::from_uuid)
        .map_err(|_| invalid_ticket("content.ticket.verify.claims"))
}

fn parse_content_id(value: &str) -> ContentTicketResult<ContentId> {
    Uuid::parse_str(value)
        .map(ContentId::from_uuid)
        .map_err(|_| invalid_ticket("content.ticket.verify.claims"))
}

fn parse_agent_id(value: &str) -> ContentTicketResult<AgentId> {
    Uuid::parse_str(value)
        .map(AgentId::from_uuid)
        .map_err(|_| invalid_ticket("content.ticket.verify.claims"))
}

fn parse_time(value: u64) -> ContentTicketResult<UtcMillis> {
    i64::try_from(value)
        .ok()
        .and_then(|value| UtcMillis::new(value).ok())
        .ok_or_else(|| invalid_ticket("content.ticket.verify.claims"))
}

const fn invalid_ticket(operation: &'static str) -> ContentTicketFailure {
    ContentTicketFailure::new(operation, ContentTicketFailureKind::Invalid)
}

#[cfg(test)]
mod tests {
    use agent_room_application::ports::{
        ContentReadTicket, ContentReadTicketClaims, ContentReadTicketCodec,
        ContentTicketFailureKind, MatrixEventId, MatrixRoomId, SecretValue,
    };
    use agent_room_domain::{
        content::{ContentByteLength, ContentMediaType, Sha256Digest},
        ids::{AgentId, ContentId, PrincipalId},
        time::UtcMillis,
    };
    use uuid::Uuid;

    use super::{ContentTicketSigningKey, HmacContentReadTicketCodec};

    #[tokio::test]
    async fn 票据完整往返并精确绑定调用主体() {
        let codec = codec("k2", &["k1"]);
        let claims = claims();
        let ticket = codec.issue(&claims).await.expect("签名成功");

        let verified = codec
            .verify(&ticket, claims.principal_id, time(10_500))
            .await
            .expect("验签成功");
        assert_eq!(verified, claims);

        let failure = codec
            .verify(
                &ticket,
                PrincipalId::from_uuid(Uuid::now_v7()),
                time(10_500),
            )
            .await
            .expect_err("其他主体不能复用票据");
        assert_eq!(failure.kind(), ContentTicketFailureKind::AudienceMismatch);
    }

    #[tokio::test]
    async fn 旧密钥票据可在有限轮换窗口内继续验证() {
        let old_codec = codec("k1", &[]);
        let ticket = old_codec.issue(&claims()).await.expect("旧密钥签名成功");
        let rotated = codec("k2", &["k1"]);
        assert!(
            rotated
                .verify(&ticket, claims().principal_id, time(10_500))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn 篡改和精确到期都会被拒绝() {
        let codec = codec("k1", &[]);
        let claims = claims();
        let ticket = codec.issue(&claims).await.expect("签名成功");
        let tampered = tamper(&ticket);
        assert_eq!(
            codec
                .verify(&tampered, claims.principal_id, time(10_500))
                .await
                .expect_err("篡改必须失败")
                .kind(),
            ContentTicketFailureKind::Invalid
        );
        assert_eq!(
            codec
                .verify(&ticket, claims.principal_id, claims.expires_at)
                .await
                .expect_err("到期毫秒立即失败")
                .kind(),
            ContentTicketFailureKind::Expired
        );
    }

    #[test]
    fn 调试输出不会泄漏签名密钥() {
        let codec = codec("k1", &[]);
        let rendered = format!("{codec:?}");
        assert!(!rendered.contains("secret-material"));
        assert!(rendered.contains("k1"));
    }

    fn codec(active: &str, previous: &[&str]) -> HmacContentReadTicketCodec {
        let active = signing_key(active);
        let previous = previous.iter().map(|key_id| signing_key(key_id));
        HmacContentReadTicketCodec::new(active, previous).expect("编解码器配置有效")
    }

    fn signing_key(key_id: &str) -> ContentTicketSigningKey {
        ContentTicketSigningKey::new(
            key_id,
            SecretValue::new(format!("secret-material-{key_id}-0123456789abcdef"))
                .expect("测试密钥有效"),
        )
        .expect("签名密钥有效")
    }

    fn claims() -> ContentReadTicketClaims {
        ContentReadTicketClaims {
            principal_id: PrincipalId::from_uuid(
                Uuid::parse_str("01980000-0000-7000-8000-000000000001").expect("UUID 有效"),
            ),
            actor_agent_id: Some(AgentId::from_uuid(
                Uuid::parse_str("01980000-0000-7000-8000-000000000009").expect("UUID 有效"),
            )),
            content_id: ContentId::from_uuid(
                Uuid::parse_str("01980000-0000-7000-8000-000000000002").expect("UUID 有效"),
            ),
            matrix_room_id: MatrixRoomId::new("!room:example.test").expect("房间 ID 有效"),
            matrix_event_id: MatrixEventId::new("$event").expect("事件 ID 有效"),
            digest: Sha256Digest::from_bytes([0x42; 32]),
            byte_length: ContentByteLength::new(42).expect("长度有效"),
            media_type: ContentMediaType::new("text/plain").expect("媒体类型有效"),
            issued_at: time(10_001),
            expires_at: time(20_001),
        }
    }

    fn tamper(ticket: &ContentReadTicket) -> ContentReadTicket {
        let mut sections = ticket
            .expose()
            .split('.')
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let signature = sections.get_mut(2).expect("JWT 含签名段");
        let replacement = if signature.starts_with('a') { 'b' } else { 'a' };
        signature.replace_range(..1, &replacement.to_string());
        ContentReadTicket::new(sections.join(".")).expect("篡改票据仍满足外层格式")
    }

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("测试时间有效")
    }
}
