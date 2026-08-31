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
const MIN_DESKTOP_CLIENT_STATE_LENGTH: usize = 32;
const MAX_DESKTOP_CLIENT_STATE_LENGTH: usize = 128;
const PKCE_CODE_CHALLENGE_LENGTH: usize = 43;

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

/// 桌面客户端生成并在自定义协议回调中核对的随机状态。
#[derive(Clone, PartialEq, Eq)]
pub struct DesktopClientState(String);

impl DesktopClientState {
    /// 只接受足够长的 base64url 随机值，避免把任意 URL 数据带回桌面进程。
    ///
    /// # Errors
    ///
    /// 长度不足、超限或包含非 base64url 字符时返回校验错误。
    pub fn new(value: impl Into<String>) -> Result<Self, IdentityValueError> {
        let value = value.into();
        if !(MIN_DESKTOP_CLIENT_STATE_LENGTH..=MAX_DESKTOP_CLIENT_STATE_LENGTH)
            .contains(&value.len())
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(IdentityValueError::InvalidDesktopClientState);
        }
        Ok(Self(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for DesktopClientState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[桌面状态已脱敏]")
    }
}

/// RFC 7636 S256 code challenge。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PkceCodeChallenge(String);

impl PkceCodeChallenge {
    /// 创建固定长度的 SHA-256 base64url challenge。
    ///
    /// # Errors
    ///
    /// 值不是 43 字节 base64url 文本时返回校验错误。
    pub fn new(value: impl Into<String>) -> Result<Self, IdentityValueError> {
        let value = value.into();
        if value.len() != PKCE_CODE_CHALLENGE_LENGTH
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(IdentityValueError::InvalidPkceChallenge);
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
pub enum OidcInteraction {
    SignIn,
    CreateAccount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OidcAuthorizationOptions {
    pub request_profile: bool,
    pub maximum_authentication_age: DurationMillis,
    pub interaction: OidcInteraction,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcDeviceAuthorizationPrompt {
    pub user_code: SecretValue,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: DurationMillis,
    pub polling_interval: DurationMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OidcDevicePromptFailure;

pub trait OidcDeviceAuthorizationPromptSink: Send + Sync {
    /// 把短期设备验证码交给交互层展示；实现不得记录完整验证码。
    ///
    /// # Errors
    ///
    /// 当前交互层无法安全展示授权指引时返回错误。
    fn present(
        &self,
        prompt: &OidcDeviceAuthorizationPrompt,
    ) -> Result<(), OidcDevicePromptFailure>;
}

pub trait OidcDeviceGrantGateway: Send + Sync {
    /// 发起 RFC 8628 设备授权并按提供者间隔等待一次性身份断言。
    fn authorize<'a>(
        &'a self,
        prompt_sink: &'a dyn OidcDeviceAuthorizationPromptSink,
    ) -> PortFuture<'a, OidcResult<SecretValue>>;
}

pub trait OidcDeviceAssertionVerifier: Send + Sync {
    /// 校验设备授权返回的 OIDC ID Token，并投影稳定主体声明。
    fn verify_assertion<'a>(
        &'a self,
        assertion: &'a SecretValue,
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
pub enum LoginDelivery {
    Web {
        return_path: SafeReturnPath,
    },
    Desktop {
        client_state: DesktopClientState,
        code_challenge: PkceCodeChallenge,
        return_path: SafeReturnPath,
    },
}

impl LoginDelivery {
    pub const fn return_path(&self) -> &SafeReturnPath {
        match self {
            Self::Web { return_path } | Self::Desktop { return_path, .. } => return_path,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginAttempt {
    pub id: LoginAttemptId,
    pub browser_secret_digest: SecretDigest,
    pub state_digest: SecretDigest,
    pub nonce: SecretValue,
    pub pkce_verifier: SecretValue,
    pub delivery: LoginDelivery,
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
pub struct DesktopAuthorizationCodeRegistration {
    pub code_digest: SecretDigest,
    pub code_challenge: PkceCodeChallenge,
    pub authenticated_at: UtcMillis,
    pub created_at: UtcMillis,
    pub expires_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopAuthorizationGrant {
    pub code_digest: SecretDigest,
    pub code_challenge: PkceCodeChallenge,
    pub created_at: UtcMillis,
    pub expires_at: UtcMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopSessionRegistration {
    pub id: WebSessionId,
    pub secret_digest: SecretDigest,
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

pub trait DesktopLoginCompletionTransaction: Send + Sync {
    fn complete_desktop<'a>(
        &'a self,
        principal: &'a PrincipalRegistration,
        authorization: &'a DesktopAuthorizationCodeRegistration,
    ) -> PortFuture<'a, RepositoryResult<PrincipalAccount>>;
}

pub trait DesktopSessionAuthorizationTransaction: Send + Sync {
    /// 使用仍然有效的浏览器会话签发一次性桌面授权码。
    ///
    /// 实现必须在同一事务中验证会话、主体状态并写入授权码；无效或过期会话返回
    /// `None`，不得退化为未绑定主体的授权。
    fn authorize_desktop_session<'a>(
        &'a self,
        session_secret_digest: &'a SecretDigest,
        grant: &'a DesktopAuthorizationGrant,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<StoredWebSession>>>;
}

pub trait DesktopSessionExchangeTransaction: Send + Sync {
    fn exchange_desktop<'a>(
        &'a self,
        code_digest: &'a SecretDigest,
        code_challenge: &'a PkceCodeChallenge,
        session: &'a DesktopSessionRegistration,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<StoredWebSession>>>;
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
    InvalidDesktopClientState,
    InvalidPkceChallenge,
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
