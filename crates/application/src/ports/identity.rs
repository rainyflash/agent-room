use std::fmt;

use agent_room_domain::{
    identity::Principal,
    ids::{ContentId, LoginAttemptId, PrincipalId, WebSessionId},
    time::{DurationMillis, UtcMillis},
};

use crate::persistence::RepositoryResult;

use super::PortFuture;

pub const SECRET_DIGEST_LENGTH: usize = 32;
const MAX_SECRET_LENGTH: usize = 4_096;
const MAX_RETURN_PATH_LENGTH: usize = 2_048;
const MAX_ISSUER_LENGTH: usize = 2_048;
const MAX_SUBJECT_LENGTH: usize = 512;
const MAX_DISPLAY_NAME_LENGTH: usize = 128;

/// 只在必须调用协议或持久化边界时短暂暴露的敏感值。
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl SecretValue {
    /// 创建长度受限且不包含空字节的敏感值。
    ///
    /// # Errors
    ///
    /// 空值、超长值或含空字节时返回校验错误。
    pub fn new(value: impl Into<String>) -> Result<Self, IdentityValueError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_SECRET_LENGTH || value.contains('\0') {
            return Err(IdentityValueError::InvalidSecret);
        }
        Ok(Self(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[已脱敏]")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SecretDigest([u8; SECRET_DIGEST_LENGTH]);

impl SecretDigest {
    pub const fn from_array(value: [u8; SECRET_DIGEST_LENGTH]) -> Self {
        Self(value)
    }

    pub const fn as_bytes(&self) -> &[u8; SECRET_DIGEST_LENGTH] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeReturnPath(String);

impl SafeReturnPath {
    /// 只接受同源绝对路径，拒绝协议相对地址、反斜杠和控制字符。
    ///
    /// # Errors
    ///
    /// 输入可逃逸固定前端源或长度超限时返回校验错误。
    pub fn new(value: impl Into<String>) -> Result<Self, IdentityValueError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_RETURN_PATH_LENGTH
            || !value.starts_with('/')
            || value.starts_with("//")
            || value.contains('\\')
            || value.chars().any(char::is_control)
        {
            return Err(IdentityValueError::UnsafeReturnPath);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProfileImportConsent {
    pub display_name: bool,
    pub locale: bool,
}

impl ProfileImportConsent {
    pub const fn requests_profile_scope(self) -> bool {
        self.display_name || self.locale
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcAuthorizationRequest {
    pub authorization_url: String,
    pub state: SecretValue,
    pub nonce: SecretValue,
    pub pkce_verifier: SecretValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OidcAuthorizationOptions {
    pub request_profile: bool,
    pub maximum_authentication_age: DurationMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcCodeExchange<'a> {
    pub code: &'a str,
    pub pkce_verifier: &'a SecretValue,
    pub expected_nonce: &'a SecretValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedOidcIdentity {
    issuer: String,
    subject: String,
    display_name: Option<String>,
    locale: Option<String>,
    authenticated_at: Option<UtcMillis>,
}

impl VerifiedOidcIdentity {
    /// 构造已经过签名、issuer、audience、期限和 nonce 校验的主体声明。
    ///
    /// # Errors
    ///
    /// 稳定主体键不满足边界约束时返回校验错误；非法可选资料被丢弃。
    pub fn new(
        issuer: impl Into<String>,
        subject: impl Into<String>,
        display_name: Option<String>,
        locale: Option<String>,
        authenticated_at: Option<UtcMillis>,
    ) -> Result<Self, IdentityValueError> {
        let issuer = issuer.into();
        let subject = subject.into();
        validate_identity_text(&issuer, MAX_ISSUER_LENGTH)
            .map_err(|()| IdentityValueError::InvalidIssuer)?;
        validate_identity_text(&subject, MAX_SUBJECT_LENGTH)
            .map_err(|()| IdentityValueError::InvalidSubject)?;

        Ok(Self {
            issuer,
            subject,
            display_name: display_name.and_then(|value| {
                if value.chars().any(char::is_control) {
                    return None;
                }
                let trimmed = value.trim();
                valid_display_name(trimmed).then(|| trimmed.to_owned())
            }),
            locale: locale.filter(|value| valid_locale(value)),
            authenticated_at,
        })
    }

    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }

    pub fn locale(&self) -> Option<&str> {
        self.locale.as_deref()
    }

    pub const fn authenticated_at(&self) -> Option<UtcMillis> {
        self.authenticated_at
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OidcFailureKind {
    DependencyUnavailable,
    ProviderRejected,
    InvalidIdentityToken,
    InvalidConfiguration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OidcFailure {
    kind: OidcFailureKind,
}

impl OidcFailure {
    pub const fn new(kind: OidcFailureKind) -> Self {
        Self { kind }
    }

    pub const fn kind(self) -> OidcFailureKind {
        self.kind
    }
}

pub type OidcResult<T> = Result<T, OidcFailure>;

pub trait OidcGateway: Send + Sync {
    fn begin_authorization(
        &self,
        options: OidcAuthorizationOptions,
    ) -> PortFuture<'_, OidcResult<OidcAuthorizationRequest>>;

    fn exchange_code<'a>(
        &'a self,
        exchange: OidcCodeExchange<'a>,
    ) -> PortFuture<'a, OidcResult<VerifiedOidcIdentity>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretGenerationFailure {
    EntropyUnavailable,
}

pub trait SecretFactory: Send + Sync {
    /// 生成至少 256 位熵的不可预测敏感值。
    ///
    /// # Errors
    ///
    /// 操作系统安全随机源不可用时返回错误，不允许退化为伪随机值。
    fn generate(&self) -> Result<SecretValue, SecretGenerationFailure>;
    fn digest(&self, value: &str) -> SecretDigest;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginAttempt {
    pub id: LoginAttemptId,
    pub browser_secret_digest: SecretDigest,
    pub state_digest: SecretDigest,
    pub nonce: SecretValue,
    pub pkce_verifier: SecretValue,
    pub return_path: SafeReturnPath,
    pub profile_import: ProfileImportConsent,
    pub created_at: UtcMillis,
    pub expires_at: UtcMillis,
}

pub trait LoginAttemptStore: Send + Sync {
    fn create<'a>(&'a self, attempt: &'a LoginAttempt) -> PortFuture<'a, RepositoryResult<()>>;

    fn consume<'a>(
        &'a self,
        browser_secret_digest: &'a SecretDigest,
        state_digest: &'a SecretDigest,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<LoginAttempt>>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrincipalRegistration {
    pub principal: Principal,
    pub oidc_issuer: String,
    pub oidc_subject: String,
    pub matrix_user_id: String,
    pub display_name: String,
    pub avatar_content_id: Option<ContentId>,
    pub locale: String,
    pub registered_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrincipalAccount {
    pub principal: Principal,
    pub matrix_user_id: String,
    pub display_name: String,
    pub avatar_content_id: Option<ContentId>,
    pub locale: String,
}

pub trait PrincipalRepository: Send + Sync {
    fn find(&self, id: PrincipalId) -> PortFuture<'_, RepositoryResult<Option<Principal>>>;

    fn create<'a>(
        &'a self,
        registration: &'a PrincipalRegistration,
    ) -> PortFuture<'a, RepositoryResult<Principal>>;

    fn save<'a>(&'a self, principal: &'a Principal) -> PortFuture<'a, RepositoryResult<Principal>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSessionRegistration {
    pub id: WebSessionId,
    pub secret_digest: SecretDigest,
    pub authenticated_at: UtcMillis,
    pub created_at: UtcMillis,
    pub expires_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredWebSession {
    pub id: WebSessionId,
    pub account: PrincipalAccount,
    pub authenticated_at: UtcMillis,
    pub created_at: UtcMillis,
    pub expires_at: UtcMillis,
}

pub trait LoginCompletionTransaction: Send + Sync {
    fn complete<'a>(
        &'a self,
        principal: &'a PrincipalRegistration,
        session: &'a WebSessionRegistration,
    ) -> PortFuture<'a, RepositoryResult<StoredWebSession>>;
}

pub trait WebSessionStore: Send + Sync {
    fn find_active<'a>(
        &'a self,
        secret_digest: &'a SecretDigest,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<StoredWebSession>>>;

    fn revoke<'a>(
        &'a self,
        secret_digest: &'a SecretDigest,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<bool>>;
}

pub trait PrincipalSuspensionTransaction: Send + Sync {
    fn suspend(
        &self,
        principal_id: PrincipalId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Principal>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityValueError {
    InvalidSecret,
    UnsafeReturnPath,
    InvalidIssuer,
    InvalidSubject,
}

fn validate_identity_text(value: &str, maximum_length: usize) -> Result<(), ()> {
    if value.is_empty() || value.len() > maximum_length || value.chars().any(char::is_control) {
        return Err(());
    }
    Ok(())
}

fn valid_display_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_DISPLAY_NAME_LENGTH
        && !value.chars().any(char::is_control)
}

fn valid_locale(value: &str) -> bool {
    let mut parts = value.split('-');
    let Some(language) = parts.next() else {
        return false;
    };
    if !(2..=8).contains(&language.len())
        || !language.bytes().all(|byte| byte.is_ascii_alphabetic())
    {
        return false;
    }
    parts.all(|part| {
        (1..=8).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
    })
}

#[cfg(test)]
mod tests {
    use super::{SafeReturnPath, SecretValue, VerifiedOidcIdentity};

    #[test]
    fn 敏感值调试输出不会泄漏正文() {
        let secret = SecretValue::new("不可出现在日志中的值").expect("敏感值有效");
        assert_eq!(format!("{secret:?}"), "[已脱敏]");
    }

    #[test]
    fn 回跳地址只允许同源路径() {
        assert!(SafeReturnPath::new("/rooms/alpha?focus=1").is_ok());
        for unsafe_value in [
            "https://attacker.example",
            "//attacker.example/path",
            "/\\attacker.example",
            "/rooms\r\nlocation:https://attacker.example",
        ] {
            assert!(SafeReturnPath::new(unsafe_value).is_err());
        }
    }

    #[test]
    fn 非法第三方资料被丢弃而不是写入主体投影() {
        let identity = VerifiedOidcIdentity::new(
            "https://issuer.example",
            "stable-subject",
            Some("\n伪造名称".to_owned()),
            Some("not_a_locale".to_owned()),
            None,
        )
        .expect("稳定主体键有效");

        assert_eq!(identity.display_name(), None);
        assert_eq!(identity.locale(), None);
    }
}
