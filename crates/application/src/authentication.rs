use std::sync::Arc;

use agent_room_domain::{
    identity::Principal,
    ids::PrincipalId,
    time::{DurationMillis, UtcMillis},
};

use crate::{
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        Clock, IdentifierFactory, LoginAttempt, LoginAttemptStore, LoginCompletionTransaction,
        OidcAuthorizationOptions, OidcCodeExchange, OidcFailureKind, OidcGateway, PortFuture,
        PrincipalRegistration, PrincipalSuspensionTransaction, ProfileImportConsent,
        SafeReturnPath, SecretFactory, SecretValue, StoredWebSession, WebSessionRegistration,
        WebSessionStore,
    },
};

const DEFAULT_LOCALE: &str = "en";
const MAX_AUTHORIZATION_VALUE_LENGTH: usize = 4_096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticationPolicy {
    login_attempt_ttl: DurationMillis,
    web_session_ttl: DurationMillis,
    recent_authentication_window: DurationMillis,
    allowed_clock_skew: DurationMillis,
    matrix_server_name: String,
}

impl AuthenticationPolicy {
    /// 创建认证时限与 Matrix 主体映射策略。
    ///
    /// # Errors
    ///
    /// Matrix 服务名包含路径、控制字符或长度超限时返回配置错误。
    pub fn new(
        login_attempt_ttl: DurationMillis,
        web_session_ttl: DurationMillis,
        recent_authentication_window: DurationMillis,
        allowed_clock_skew: DurationMillis,
        matrix_server_name: impl Into<String>,
    ) -> Result<Self, AuthenticationConfigurationError> {
        let matrix_server_name = matrix_server_name.into();
        if matrix_server_name.is_empty()
            || matrix_server_name.len() > 255
            || matrix_server_name.chars().any(char::is_control)
            || matrix_server_name.contains(['/', '\\', '@'])
        {
            return Err(AuthenticationConfigurationError::InvalidMatrixServerName);
        }

        Ok(Self {
            login_attempt_ttl,
            web_session_ttl,
            recent_authentication_window,
            allowed_clock_skew,
            matrix_server_name,
        })
    }

    pub const fn web_session_ttl(&self) -> DurationMillis {
        self.web_session_ttl
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationConfigurationError {
    InvalidMatrixServerName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeginLogin {
    pub return_path: SafeReturnPath,
    pub profile_import: ProfileImportConsent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginRedirect {
    pub authorization_url: String,
    pub browser_secret: SecretValue,
    pub expires_at: UtcMillis,
}

#[derive(Clone, PartialEq, Eq)]
pub struct CompleteLogin<'a> {
    pub code: &'a str,
    pub returned_state: &'a str,
    pub browser_secret: &'a SecretValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedPrincipal {
    pub principal_id: PrincipalId,
    pub matrix_user_id: String,
    pub display_name: String,
    pub locale: String,
    pub authenticated_at: UtcMillis,
    pub expires_at: UtcMillis,
    pub recently_authenticated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginCompletion {
    pub session_secret: SecretValue,
    pub return_path: SafeReturnPath,
    pub principal: AuthenticatedPrincipal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationRequirement {
    ActiveSession,
    RecentAuthentication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationFailureKind {
    InvalidRequest,
    InvalidLoginState,
    ProviderRejected,
    InvalidIdentityToken,
    InvalidSession,
    PrincipalSuspended,
    ReauthenticationRequired,
    DependencyUnavailable,
    Conflict,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthenticationFailure {
    operation: &'static str,
    kind: AuthenticationFailureKind,
}

impl AuthenticationFailure {
    const fn new(operation: &'static str, kind: AuthenticationFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> AuthenticationFailureKind {
        self.kind
    }
}

pub type AuthenticationResult<T> = Result<T, AuthenticationFailure>;

pub trait AuthenticationUseCases: Send + Sync {
    fn begin_login(
        &self,
        request: BeginLogin,
    ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>>;

    fn complete_login<'a>(
        &'a self,
        request: CompleteLogin<'a>,
    ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>>;

    fn authenticate<'a>(
        &'a self,
        session_secret: &'a SecretValue,
        requirement: AuthenticationRequirement,
    ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>>;

    fn logout<'a>(
        &'a self,
        session_secret: &'a SecretValue,
    ) -> PortFuture<'a, AuthenticationResult<()>>;

    fn suspend_principal(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, AuthenticationResult<()>>;
}

pub struct AuthenticationService {
    oidc: Arc<dyn OidcGateway>,
    login_attempts: Arc<dyn LoginAttemptStore>,
    login_completion: Arc<dyn LoginCompletionTransaction>,
    sessions: Arc<dyn WebSessionStore>,
    suspensions: Arc<dyn PrincipalSuspensionTransaction>,
    secrets: Arc<dyn SecretFactory>,
    identifiers: Arc<dyn IdentifierFactory>,
    clock: Arc<dyn Clock>,
    policy: AuthenticationPolicy,
}

pub struct AuthenticationDependencies {
    pub oidc: Arc<dyn OidcGateway>,
    pub login_attempts: Arc<dyn LoginAttemptStore>,
    pub login_completion: Arc<dyn LoginCompletionTransaction>,
    pub sessions: Arc<dyn WebSessionStore>,
    pub suspensions: Arc<dyn PrincipalSuspensionTransaction>,
    pub secrets: Arc<dyn SecretFactory>,
    pub identifiers: Arc<dyn IdentifierFactory>,
    pub clock: Arc<dyn Clock>,
}

impl AuthenticationService {
    pub fn new(dependencies: AuthenticationDependencies, policy: AuthenticationPolicy) -> Self {
        Self {
            oidc: dependencies.oidc,
            login_attempts: dependencies.login_attempts,
            login_completion: dependencies.login_completion,
            sessions: dependencies.sessions,
            suspensions: dependencies.suspensions,
            secrets: dependencies.secrets,
            identifiers: dependencies.identifiers,
            clock: dependencies.clock,
            policy,
        }
    }

    async fn begin_login_internal(
        &self,
        request: BeginLogin,
    ) -> AuthenticationResult<LoginRedirect> {
        let now = self.clock.now();
        let expires_at = now
            .checked_add(self.policy.login_attempt_ttl)
            .map_err(|_| internal_failure("authentication.begin_login"))?;
        let authorization = self
            .oidc
            .begin_authorization(OidcAuthorizationOptions {
                request_profile: request.profile_import.requests_profile_scope(),
                maximum_authentication_age: self.policy.recent_authentication_window,
            })
            .await
            .map_err(|failure| map_oidc_failure("authentication.begin_login", failure.kind()))?;
        let browser_secret = self
            .secrets
            .generate()
            .map_err(|_| internal_failure("authentication.begin_login"))?;
        let attempt = LoginAttempt {
            id: self.identifiers.login_attempt_id(),
            browser_secret_digest: self.secrets.digest(browser_secret.expose()),
            state_digest: self.secrets.digest(authorization.state.expose()),
            nonce: authorization.nonce,
            pkce_verifier: authorization.pkce_verifier,
            return_path: request.return_path,
            profile_import: request.profile_import,
            created_at: now,
            expires_at,
        };
        self.login_attempts
            .create(&attempt)
            .await
            .map_err(|error| map_repository_failure("authentication.begin_login", &error))?;

        Ok(LoginRedirect {
            authorization_url: authorization.authorization_url,
            browser_secret,
            expires_at,
        })
    }

    async fn complete_login_internal(
        &self,
        request: CompleteLogin<'_>,
    ) -> AuthenticationResult<LoginCompletion> {
        validate_authorization_value(request.code)
            .and_then(|()| validate_authorization_value(request.returned_state))
            .map_err(|()| {
                AuthenticationFailure::new(
                    "authentication.complete_login",
                    AuthenticationFailureKind::InvalidRequest,
                )
            })?;
        let now = self.clock.now();
        let browser_digest = self.secrets.digest(request.browser_secret.expose());
        let state_digest = self.secrets.digest(request.returned_state);
        let attempt = self
            .login_attempts
            .consume(&browser_digest, &state_digest, now)
            .await
            .map_err(|error| map_repository_failure("authentication.complete_login", &error))?
            .ok_or_else(|| {
                AuthenticationFailure::new(
                    "authentication.complete_login",
                    AuthenticationFailureKind::InvalidLoginState,
                )
            })?;

        let identity = self
            .oidc
            .exchange_code(OidcCodeExchange {
                code: request.code,
                pkce_verifier: &attempt.pkce_verifier,
                expected_nonce: &attempt.nonce,
            })
            .await
            .map_err(|failure| map_oidc_failure("authentication.complete_login", failure.kind()))?;
        let authenticated_at = validate_authentication_time(&identity, now, &self.policy)?;
        let principal_id = self.identifiers.principal_id();
        let principal_registration = principal_registration(
            principal_id,
            &identity,
            attempt.profile_import,
            now,
            &self.policy.matrix_server_name,
        );
        let session_secret = self
            .secrets
            .generate()
            .map_err(|_| internal_failure("authentication.complete_login"))?;
        let expires_at = now
            .checked_add(self.policy.web_session_ttl)
            .map_err(|_| internal_failure("authentication.complete_login"))?;
        let session_registration = WebSessionRegistration {
            id: self.identifiers.web_session_id(),
            secret_digest: self.secrets.digest(session_secret.expose()),
            authenticated_at,
            created_at: now,
            expires_at,
        };
        let session = self
            .login_completion
            .complete(&principal_registration, &session_registration)
            .await
            .map_err(|error| map_repository_failure("authentication.complete_login", &error))?;
        let principal = session_view(&session, now, &self.policy);

        Ok(LoginCompletion {
            session_secret,
            return_path: attempt.return_path,
            principal,
        })
    }

    async fn authenticate_internal(
        &self,
        session_secret: &SecretValue,
        requirement: AuthenticationRequirement,
    ) -> AuthenticationResult<AuthenticatedPrincipal> {
        let now = self.clock.now();
        let secret_digest = self.secrets.digest(session_secret.expose());
        let session = self
            .sessions
            .find_active(&secret_digest, now)
            .await
            .map_err(|error| map_repository_failure("authentication.authenticate", &error))?
            .ok_or_else(|| {
                AuthenticationFailure::new(
                    "authentication.authenticate",
                    AuthenticationFailureKind::InvalidSession,
                )
            })?;
        if !session.account.principal.allows_authentication() {
            return Err(AuthenticationFailure::new(
                "authentication.authenticate",
                AuthenticationFailureKind::PrincipalSuspended,
            ));
        }
        let principal = session_view(&session, now, &self.policy);
        if matches!(requirement, AuthenticationRequirement::RecentAuthentication)
            && !principal.recently_authenticated
        {
            return Err(AuthenticationFailure::new(
                "authentication.authenticate",
                AuthenticationFailureKind::ReauthenticationRequired,
            ));
        }
        Ok(principal)
    }

    async fn logout_internal(&self, session_secret: &SecretValue) -> AuthenticationResult<()> {
        let now = self.clock.now();
        let secret_digest = self.secrets.digest(session_secret.expose());
        self.sessions
            .revoke(&secret_digest, now)
            .await
            .map_err(|error| map_repository_failure("authentication.logout", &error))?;
        Ok(())
    }

    async fn suspend_principal_internal(
        &self,
        principal_id: PrincipalId,
    ) -> AuthenticationResult<()> {
        self.suspensions
            .suspend(principal_id, self.clock.now())
            .await
            .map(|_| ())
            .map_err(|error| map_repository_failure("authentication.suspend", &error))
    }
}

impl AuthenticationUseCases for AuthenticationService {
    fn begin_login(
        &self,
        request: BeginLogin,
    ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>> {
        Box::pin(self.begin_login_internal(request))
    }

    fn complete_login<'a>(
        &'a self,
        request: CompleteLogin<'a>,
    ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
        Box::pin(self.complete_login_internal(request))
    }

    fn authenticate<'a>(
        &'a self,
        session_secret: &'a SecretValue,
        requirement: AuthenticationRequirement,
    ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>> {
        Box::pin(self.authenticate_internal(session_secret, requirement))
    }

    fn logout<'a>(
        &'a self,
        session_secret: &'a SecretValue,
    ) -> PortFuture<'a, AuthenticationResult<()>> {
        Box::pin(self.logout_internal(session_secret))
    }

    fn suspend_principal(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, AuthenticationResult<()>> {
        Box::pin(self.suspend_principal_internal(principal_id))
    }
}

fn validate_authorization_value(value: &str) -> Result<(), ()> {
    if value.is_empty() || value.len() > MAX_AUTHORIZATION_VALUE_LENGTH || value.contains('\0') {
        return Err(());
    }
    Ok(())
}

fn validate_authentication_time(
    identity: &crate::ports::VerifiedOidcIdentity,
    now: UtcMillis,
    policy: &AuthenticationPolicy,
) -> AuthenticationResult<UtcMillis> {
    let authenticated_at = identity.authenticated_at().ok_or_else(|| {
        AuthenticationFailure::new(
            "authentication.complete_login",
            AuthenticationFailureKind::InvalidIdentityToken,
        )
    })?;
    let latest_allowed = now
        .checked_add(policy.allowed_clock_skew)
        .map_err(|_| internal_failure("authentication.complete_login"))?;
    let oldest_allowed = authenticated_at
        .checked_add(policy.recent_authentication_window)
        .and_then(|value| value.checked_add(policy.allowed_clock_skew))
        .map_err(|_| internal_failure("authentication.complete_login"))?;
    if authenticated_at > latest_allowed || now > oldest_allowed {
        return Err(AuthenticationFailure::new(
            "authentication.complete_login",
            AuthenticationFailureKind::InvalidIdentityToken,
        ));
    }
    Ok(authenticated_at)
}

fn principal_registration(
    id: PrincipalId,
    identity: &crate::ports::VerifiedOidcIdentity,
    consent: ProfileImportConsent,
    registered_at: UtcMillis,
    matrix_server_name: &str,
) -> PrincipalRegistration {
    let compact_id = id.to_string().replace('-', "");
    let default_display_name = format!("Agent Room User {}", &compact_id[..8]);
    let display_name = consent
        .display_name
        .then(|| identity.display_name())
        .flatten()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&default_display_name)
        .to_owned();
    let locale = consent
        .locale
        .then(|| identity.locale())
        .flatten()
        .unwrap_or(DEFAULT_LOCALE)
        .to_owned();

    PrincipalRegistration {
        principal: Principal::new(id),
        oidc_issuer: identity.issuer().to_owned(),
        oidc_subject: identity.subject().to_owned(),
        matrix_user_id: format!("@user-{compact_id}:{matrix_server_name}"),
        display_name,
        avatar_content_id: None,
        locale,
        registered_at,
    }
}

fn session_view(
    session: &StoredWebSession,
    now: UtcMillis,
    policy: &AuthenticationPolicy,
) -> AuthenticatedPrincipal {
    let recent_until = session
        .authenticated_at
        .checked_add(policy.recent_authentication_window)
        .and_then(|value| value.checked_add(policy.allowed_clock_skew));
    AuthenticatedPrincipal {
        principal_id: session.account.principal.id(),
        matrix_user_id: session.account.matrix_user_id.clone(),
        display_name: session.account.display_name.clone(),
        locale: session.account.locale.clone(),
        authenticated_at: session.authenticated_at,
        expires_at: session.expires_at,
        recently_authenticated: recent_until.is_ok_and(|deadline| now <= deadline),
    }
}

const fn map_oidc_failure(operation: &'static str, kind: OidcFailureKind) -> AuthenticationFailure {
    let mapped = match kind {
        OidcFailureKind::DependencyUnavailable => AuthenticationFailureKind::DependencyUnavailable,
        OidcFailureKind::ProviderRejected => AuthenticationFailureKind::ProviderRejected,
        OidcFailureKind::InvalidIdentityToken => AuthenticationFailureKind::InvalidIdentityToken,
        OidcFailureKind::InvalidConfiguration => AuthenticationFailureKind::Internal,
    };
    AuthenticationFailure::new(operation, mapped)
}

const fn map_repository_failure(
    operation: &'static str,
    error: &RepositoryError,
) -> AuthenticationFailure {
    let kind = match error.kind() {
        RepositoryErrorKind::Conflict => AuthenticationFailureKind::Conflict,
        RepositoryErrorKind::Forbidden => AuthenticationFailureKind::PrincipalSuspended,
        RepositoryErrorKind::Unavailable => AuthenticationFailureKind::DependencyUnavailable,
        RepositoryErrorKind::Constraint
        | RepositoryErrorKind::NotFound
        | RepositoryErrorKind::CorruptData => AuthenticationFailureKind::Internal,
    };
    AuthenticationFailure::new(operation, kind)
}

const fn internal_failure(operation: &'static str) -> AuthenticationFailure {
    AuthenticationFailure::new(operation, AuthenticationFailureKind::Internal)
}

#[cfg(test)]
mod tests {
    use super::{AuthenticationPolicy, principal_registration};
    use crate::ports::{ProfileImportConsent, VerifiedOidcIdentity};
    use agent_room_domain::{ids::PrincipalId, time::UtcMillis};
    use uuid::Uuid;

    fn policy() -> AuthenticationPolicy {
        AuthenticationPolicy::new(
            agent_room_domain::time::DurationMillis::new(600_000).expect("时长有效"),
            agent_room_domain::time::DurationMillis::new(28_800_000).expect("时长有效"),
            agent_room_domain::time::DurationMillis::new(300_000).expect("时长有效"),
            agent_room_domain::time::DurationMillis::new(60_000).expect("时长有效"),
            "matrix.example.test",
        )
        .expect("策略有效")
    }

    #[test]
    fn 未明确同意时不导入第三方资料() {
        let identity = VerifiedOidcIdentity::new(
            "https://issuer.example",
            "subject",
            Some("第三方名称".to_owned()),
            Some("zh-CN".to_owned()),
            Some(UtcMillis::new(1_700_000_000_000).expect("时间有效")),
        )
        .expect("声明有效");
        let id = PrincipalId::from_uuid(
            Uuid::parse_str("018c251e-7b5a-7c7f-8a28-2de53f56a9a3").expect("UUID 有效"),
        );

        let registration = principal_registration(
            id,
            &identity,
            ProfileImportConsent::default(),
            UtcMillis::new(1_700_000_000_000).expect("时间有效"),
            &policy().matrix_server_name,
        );

        assert!(registration.display_name.starts_with("Agent Room User "));
        assert_eq!(registration.locale, "en");
    }

    #[test]
    fn 明确同意后只导入已清洗字段() {
        let identity = VerifiedOidcIdentity::new(
            "https://issuer.example",
            "subject",
            Some("  Alice  ".to_owned()),
            Some("zh-CN".to_owned()),
            Some(UtcMillis::new(1_700_000_000_000).expect("时间有效")),
        )
        .expect("声明有效");
        let id = PrincipalId::from_uuid(Uuid::now_v7());
        let registration = principal_registration(
            id,
            &identity,
            ProfileImportConsent {
                display_name: true,
                locale: true,
            },
            UtcMillis::new(1_700_000_000_000).expect("时间有效"),
            &policy().matrix_server_name,
        );

        assert_eq!(registration.display_name, "Alice");
        assert_eq!(registration.locale, "zh-CN");
    }
}
