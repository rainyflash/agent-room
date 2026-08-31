use std::sync::Arc;

use agent_room_domain::{
    ids::PrincipalId,
    time::{DurationMillis, UtcMillis},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest as _, Sha256};

use crate::{
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{
        Clock, DesktopAuthorizationCodeRegistration, DesktopAuthorizationGrant, DesktopClientState,
        DesktopLoginCompletionTransaction, DesktopSessionAuthorizationTransaction,
        DesktopSessionExchangeTransaction, DesktopSessionRegistration, IdentifierFactory,
        LoginAttempt, LoginAttemptStore, LoginCompletionTransaction, LoginDelivery,
        OidcAuthorizationOptions, OidcCodeExchange, OidcFailureKind, OidcGateway, OidcInteraction,
        PkceCodeChallenge, PortFuture, PrincipalSuspensionTransaction, ProfileImportConsent,
        SafeReturnPath, SecretFactory, SecretValue, StoredWebSession, WebSessionRegistration,
        WebSessionStore,
    },
};

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
    pub delivery: LoginDelivery,
    pub profile_import: ProfileImportConsent,
    pub intent: AuthenticationIntent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationIntent {
    SignIn,
    Register,
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
pub struct WebLoginCompletion {
    pub session_secret: SecretValue,
    pub return_path: SafeReturnPath,
    pub principal: AuthenticatedPrincipal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopLoginCompletion {
    pub authorization_code: SecretValue,
    pub client_state: DesktopClientState,
    pub return_path: SafeReturnPath,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizeDesktopSession<'a> {
    pub session_secret: &'a SecretValue,
    pub client_state: DesktopClientState,
    pub code_challenge: PkceCodeChallenge,
    pub return_path: SafeReturnPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginCompletion {
    Web(WebLoginCompletion),
    Desktop(DesktopLoginCompletion),
}

#[derive(Clone, PartialEq, Eq)]
pub struct ExchangeDesktopAuthorization<'a> {
    pub authorization_code: &'a str,
    pub pkce_verifier: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopSessionCompletion {
    pub session_secret: SecretValue,
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

    fn exchange_desktop_authorization<'a>(
        &'a self,
        _request: ExchangeDesktopAuthorization<'a>,
    ) -> PortFuture<'a, AuthenticationResult<DesktopSessionCompletion>> {
        Box::pin(async {
            Err(AuthenticationFailure::new(
                "authentication.exchange_desktop_authorization",
                AuthenticationFailureKind::InvalidRequest,
            ))
        })
    }

    fn authorize_desktop_session<'a>(
        &'a self,
        _request: AuthorizeDesktopSession<'a>,
    ) -> PortFuture<'a, AuthenticationResult<DesktopLoginCompletion>> {
        Box::pin(async {
            Err(AuthenticationFailure::new(
                "authentication.authorize_desktop_session",
                AuthenticationFailureKind::InvalidRequest,
            ))
        })
    }

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
    desktop_login_completion: Arc<dyn DesktopLoginCompletionTransaction>,
    desktop_session_authorization: Arc<dyn DesktopSessionAuthorizationTransaction>,
    desktop_session_exchange: Arc<dyn DesktopSessionExchangeTransaction>,
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
    pub desktop_login_completion: Arc<dyn DesktopLoginCompletionTransaction>,
    pub desktop_session_authorization: Arc<dyn DesktopSessionAuthorizationTransaction>,
    pub desktop_session_exchange: Arc<dyn DesktopSessionExchangeTransaction>,
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
            desktop_login_completion: dependencies.desktop_login_completion,
            desktop_session_authorization: dependencies.desktop_session_authorization,
            desktop_session_exchange: dependencies.desktop_session_exchange,
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
                interaction: match request.intent {
                    AuthenticationIntent::SignIn => OidcInteraction::SignIn,
                    AuthenticationIntent::Register => OidcInteraction::CreateAccount,
                },
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
            delivery: request.delivery,
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
        let principal_registration = crate::principal_projection::principal_registration(
            principal_id,
            &identity,
            attempt.profile_import,
            now,
            &self.policy.matrix_server_name,
        );
        match attempt.delivery {
            LoginDelivery::Web { return_path } => {
                let session_secret = self
                    .secrets
                    .generate()
                    .map_err(|_| internal_failure("authentication.complete_login"))?;
                let session_registration = self.session_registration(
                    &session_secret,
                    authenticated_at,
                    now,
                    "authentication.complete_login",
                )?;
                let session = self
                    .login_completion
                    .complete(&principal_registration, &session_registration)
                    .await
                    .map_err(|error| {
                        map_repository_failure("authentication.complete_login", &error)
                    })?;
                Ok(LoginCompletion::Web(WebLoginCompletion {
                    session_secret,
                    return_path,
                    principal: session_view(&session, now, &self.policy),
                }))
            }
            LoginDelivery::Desktop {
                client_state,
                code_challenge,
                return_path,
            } => {
                let authorization_code = self
                    .secrets
                    .generate()
                    .map_err(|_| internal_failure("authentication.complete_desktop_login"))?;
                let expires_at = now
                    .checked_add(self.policy.login_attempt_ttl)
                    .map_err(|_| internal_failure("authentication.complete_desktop_login"))?;
                let authorization = DesktopAuthorizationCodeRegistration {
                    code_digest: self.secrets.digest(authorization_code.expose()),
                    code_challenge,
                    authenticated_at,
                    created_at: now,
                    expires_at,
                };
                self.desktop_login_completion
                    .complete_desktop(&principal_registration, &authorization)
                    .await
                    .map_err(|error| {
                        map_repository_failure("authentication.complete_desktop_login", &error)
                    })?;
                Ok(LoginCompletion::Desktop(DesktopLoginCompletion {
                    authorization_code,
                    client_state,
                    return_path,
                }))
            }
        }
    }

    async fn exchange_desktop_authorization_internal(
        &self,
        request: ExchangeDesktopAuthorization<'_>,
    ) -> AuthenticationResult<DesktopSessionCompletion> {
        validate_authorization_value(request.authorization_code)
            .and_then(|()| validate_pkce_verifier(request.pkce_verifier))
            .map_err(|()| {
                AuthenticationFailure::new(
                    "authentication.exchange_desktop_authorization",
                    AuthenticationFailureKind::InvalidRequest,
                )
            })?;
        let code_challenge = derive_pkce_challenge(request.pkce_verifier).map_err(|()| {
            AuthenticationFailure::new(
                "authentication.exchange_desktop_authorization",
                AuthenticationFailureKind::InvalidRequest,
            )
        })?;
        let now = self.clock.now();
        let session_secret = self
            .secrets
            .generate()
            .map_err(|_| internal_failure("authentication.exchange_desktop_authorization"))?;
        let expires_at = now
            .checked_add(self.policy.web_session_ttl)
            .map_err(|_| internal_failure("authentication.exchange_desktop_authorization"))?;
        let session_registration = DesktopSessionRegistration {
            id: self.identifiers.web_session_id(),
            secret_digest: self.secrets.digest(session_secret.expose()),
            created_at: now,
            expires_at,
        };
        let code_digest = self.secrets.digest(request.authorization_code);
        let session = self
            .desktop_session_exchange
            .exchange_desktop(&code_digest, &code_challenge, &session_registration, now)
            .await
            .map_err(|error| {
                map_repository_failure("authentication.exchange_desktop_authorization", &error)
            })?
            .ok_or_else(|| {
                AuthenticationFailure::new(
                    "authentication.exchange_desktop_authorization",
                    AuthenticationFailureKind::InvalidLoginState,
                )
            })?;
        Ok(DesktopSessionCompletion {
            session_secret,
            principal: session_view(&session, now, &self.policy),
        })
    }

    async fn authorize_desktop_session_internal(
        &self,
        request: AuthorizeDesktopSession<'_>,
    ) -> AuthenticationResult<DesktopLoginCompletion> {
        let now = self.clock.now();
        let authorization_code = self
            .secrets
            .generate()
            .map_err(|_| internal_failure("authentication.authorize_desktop_session"))?;
        let expires_at = now
            .checked_add(self.policy.login_attempt_ttl)
            .map_err(|_| internal_failure("authentication.authorize_desktop_session"))?;
        let grant = DesktopAuthorizationGrant {
            code_digest: self.secrets.digest(authorization_code.expose()),
            code_challenge: request.code_challenge,
            created_at: now,
            expires_at,
        };
        let session_digest = self.secrets.digest(request.session_secret.expose());
        self.desktop_session_authorization
            .authorize_desktop_session(&session_digest, &grant, now)
            .await
            .map_err(|error| {
                map_repository_failure("authentication.authorize_desktop_session", &error)
            })?
            .ok_or_else(|| {
                AuthenticationFailure::new(
                    "authentication.authorize_desktop_session",
                    AuthenticationFailureKind::InvalidSession,
                )
            })?;
        Ok(DesktopLoginCompletion {
            authorization_code,
            client_state: request.client_state,
            return_path: request.return_path,
        })
    }

    fn session_registration(
        &self,
        session_secret: &SecretValue,
        authenticated_at: UtcMillis,
        now: UtcMillis,
        operation: &'static str,
    ) -> AuthenticationResult<WebSessionRegistration> {
        let expires_at = now
            .checked_add(self.policy.web_session_ttl)
            .map_err(|_| internal_failure(operation))?;
        Ok(WebSessionRegistration {
            id: self.identifiers.web_session_id(),
            secret_digest: self.secrets.digest(session_secret.expose()),
            authenticated_at,
            created_at: now,
            expires_at,
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

    fn exchange_desktop_authorization<'a>(
        &'a self,
        request: ExchangeDesktopAuthorization<'a>,
    ) -> PortFuture<'a, AuthenticationResult<DesktopSessionCompletion>> {
        Box::pin(self.exchange_desktop_authorization_internal(request))
    }

    fn authorize_desktop_session<'a>(
        &'a self,
        request: AuthorizeDesktopSession<'a>,
    ) -> PortFuture<'a, AuthenticationResult<DesktopLoginCompletion>> {
        Box::pin(self.authorize_desktop_session_internal(request))
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

fn validate_pkce_verifier(value: &str) -> Result<(), ()> {
    if !(43..=128).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
    {
        return Err(());
    }
    Ok(())
}

fn derive_pkce_challenge(verifier: &str) -> Result<PkceCodeChallenge, ()> {
    PkceCodeChallenge::new(URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())))
        .map_err(|_| ())
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
    use super::AuthenticationPolicy;
    use crate::ports::{ProfileImportConsent, VerifiedOidcIdentity};
    use crate::principal_projection::principal_registration;
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
