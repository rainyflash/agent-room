use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
};

use agent_room_application::{
    authentication::{
        AuthenticationDependencies, AuthenticationFailureKind, AuthenticationIntent,
        AuthenticationPolicy, AuthenticationRequirement, AuthenticationService,
        AuthenticationUseCases, BeginLogin, CompleteLogin, ExchangeDesktopAuthorization,
        LoginCompletion, WebLoginCompletion,
    },
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        Clock, DesktopAuthorizationCodeRegistration, DesktopClientState,
        DesktopLoginCompletionTransaction, DesktopSessionExchangeTransaction,
        DesktopSessionRegistration, IdentifierFactory, LoginAttempt, LoginAttemptStore,
        LoginCompletionTransaction, LoginDelivery, OidcAuthorizationOptions,
        OidcAuthorizationRequest, OidcCodeExchange, OidcGateway, OidcInteraction, OidcResult,
        PkceCodeChallenge, PortFuture, PrincipalAccount, PrincipalRegistration,
        PrincipalSuspensionTransaction, ProfileImportConsent, SafeReturnPath, SecretDigest,
        SecretFactory, SecretGenerationFailure, SecretValue, StoredWebSession,
        VerifiedOidcIdentity, WebSessionRegistration, WebSessionStore,
    },
};
use agent_room_domain::{
    identity::Principal,
    ids::{
        AdapterBindingId, AgentCardSnapshotId, AgentId, AgentInstanceId, AutomationGrantId,
        ContentId, DeviceAccessTokenId, DeviceId, DeviceRefreshTokenId, DeviceTokenFamilyId,
        HandoffId, LoginAttemptId, OutboxEventId, PrincipalId, RoomCatalogId, RoomInstanceId,
        RoomReservationId, WebSessionId,
    },
    time::{DurationMillis, UtcMillis},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

const NOW: i64 = 1_700_000_000_000;
const STATE: &str = "state-token";

struct TestClock(AtomicI64);

impl TestClock {
    fn set(&self, value: i64) {
        self.0.store(value, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now(&self) -> UtcMillis {
        time(self.0.load(Ordering::SeqCst))
    }
}

struct TestIdentifiers;

impl IdentifierFactory for TestIdentifiers {
    fn principal_id(&self) -> PrincipalId {
        PrincipalId::from_uuid(Uuid::now_v7())
    }

    fn login_attempt_id(&self) -> LoginAttemptId {
        LoginAttemptId::from_uuid(Uuid::now_v7())
    }

    fn web_session_id(&self) -> WebSessionId {
        WebSessionId::from_uuid(Uuid::now_v7())
    }

    fn device_id(&self) -> DeviceId {
        DeviceId::from_uuid(Uuid::now_v7())
    }

    fn device_token_family_id(&self) -> DeviceTokenFamilyId {
        DeviceTokenFamilyId::from_uuid(Uuid::now_v7())
    }

    fn device_access_token_id(&self) -> DeviceAccessTokenId {
        DeviceAccessTokenId::from_uuid(Uuid::now_v7())
    }

    fn device_refresh_token_id(&self) -> DeviceRefreshTokenId {
        DeviceRefreshTokenId::from_uuid(Uuid::now_v7())
    }

    fn agent_id(&self) -> AgentId {
        AgentId::from_uuid(Uuid::now_v7())
    }

    fn agent_card_snapshot_id(&self) -> AgentCardSnapshotId {
        AgentCardSnapshotId::from_uuid(Uuid::now_v7())
    }

    fn adapter_binding_id(&self) -> AdapterBindingId {
        AdapterBindingId::from_uuid(Uuid::now_v7())
    }

    fn agent_instance_id(&self) -> AgentInstanceId {
        AgentInstanceId::from_uuid(Uuid::now_v7())
    }

    fn room_catalog_id(&self) -> RoomCatalogId {
        RoomCatalogId::from_uuid(Uuid::now_v7())
    }

    fn room_instance_id(&self) -> RoomInstanceId {
        RoomInstanceId::from_uuid(Uuid::now_v7())
    }

    fn room_reservation_id(&self) -> RoomReservationId {
        RoomReservationId::from_uuid(Uuid::now_v7())
    }

    fn content_id(&self) -> ContentId {
        ContentId::from_uuid(Uuid::now_v7())
    }

    fn handoff_id(&self) -> HandoffId {
        HandoffId::from_uuid(Uuid::now_v7())
    }

    fn automation_grant_id(&self) -> AutomationGrantId {
        AutomationGrantId::from_uuid(Uuid::now_v7())
    }

    fn outbox_event_id(&self) -> OutboxEventId {
        OutboxEventId::from_uuid(Uuid::now_v7())
    }
}

#[derive(Default)]
struct TestSecrets(AtomicUsize);

impl SecretFactory for TestSecrets {
    fn generate(&self) -> Result<SecretValue, SecretGenerationFailure> {
        let sequence = self.0.fetch_add(1, Ordering::SeqCst);
        SecretValue::new(format!("generated-secret-{sequence}"))
            .map_err(|_| SecretGenerationFailure::EntropyUnavailable)
    }

    fn digest(&self, value: &str) -> SecretDigest {
        let mut digest = [0_u8; 32];
        for (index, byte) in value.bytes().enumerate() {
            let slot = index % digest.len();
            digest[slot] = digest[slot]
                .wrapping_mul(31)
                .wrapping_add(byte)
                .wrapping_add(u8::try_from(index % 251).expect("索引可转换"));
        }
        SecretDigest::from_array(digest)
    }
}

struct TestOidc {
    identity: Mutex<VerifiedOidcIdentity>,
    exchanges: AtomicUsize,
    requested_profile: Mutex<Option<bool>>,
    requested_interaction: Mutex<Option<OidcInteraction>>,
}

impl TestOidc {
    fn new(identity: VerifiedOidcIdentity) -> Self {
        Self {
            identity: Mutex::new(identity),
            exchanges: AtomicUsize::new(0),
            requested_profile: Mutex::new(None),
            requested_interaction: Mutex::new(None),
        }
    }
}

impl OidcGateway for TestOidc {
    fn begin_authorization(
        &self,
        options: OidcAuthorizationOptions,
    ) -> PortFuture<'_, OidcResult<OidcAuthorizationRequest>> {
        *self.requested_profile.lock().expect("测试锁可用") = Some(options.request_profile);
        *self.requested_interaction.lock().expect("测试锁可用") = Some(options.interaction);
        Box::pin(async {
            Ok(OidcAuthorizationRequest {
                authorization_url: "https://identity.example/authorize".to_owned(),
                state: SecretValue::new(STATE).expect("state 有效"),
                nonce: SecretValue::new("nonce-token").expect("nonce 有效"),
                pkce_verifier: SecretValue::new("v".repeat(43)).expect("PKCE verifier 有效"),
            })
        })
    }

    fn exchange_code<'a>(
        &'a self,
        exchange: OidcCodeExchange<'a>,
    ) -> PortFuture<'a, OidcResult<VerifiedOidcIdentity>> {
        self.exchanges.fetch_add(1, Ordering::SeqCst);
        let identity = self.identity.lock().expect("测试锁可用").clone();
        Box::pin(async move {
            assert_eq!(exchange.code, "authorization-code");
            assert_eq!(exchange.expected_nonce.expose(), "nonce-token");
            assert_eq!(exchange.pkce_verifier.expose(), "v".repeat(43));
            Ok(identity)
        })
    }
}

#[derive(Default)]
struct IdentityState {
    attempt: Option<LoginAttempt>,
    desktop_authorizations:
        HashMap<SecretDigest, (DesktopAuthorizationCodeRegistration, PrincipalAccount)>,
    sessions: HashMap<SecretDigest, StoredWebSession>,
    last_registration: Option<PrincipalRegistration>,
}

#[derive(Default)]
struct InMemoryIdentity(Mutex<IdentityState>);

impl LoginAttemptStore for InMemoryIdentity {
    fn create<'a>(&'a self, attempt: &'a LoginAttempt) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            let mut state = self.0.lock().expect("测试锁可用");
            if state.attempt.is_some() {
                return Err(RepositoryError::new(
                    "test.login_attempt.create",
                    RepositoryErrorKind::Conflict,
                ));
            }
            state.attempt = Some(attempt.clone());
            Ok(())
        })
    }

    fn consume<'a>(
        &'a self,
        browser_secret_digest: &'a SecretDigest,
        state_digest: &'a SecretDigest,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<LoginAttempt>>> {
        Box::pin(async move {
            let mut state = self.0.lock().expect("测试锁可用");
            let matches = state.attempt.as_ref().is_some_and(|attempt| {
                attempt.browser_secret_digest == *browser_secret_digest
                    && attempt.state_digest == *state_digest
                    && now < attempt.expires_at
            });
            Ok(matches.then(|| state.attempt.take()).flatten())
        })
    }
}

impl LoginCompletionTransaction for InMemoryIdentity {
    fn complete<'a>(
        &'a self,
        registration: &'a PrincipalRegistration,
        session: &'a WebSessionRegistration,
    ) -> PortFuture<'a, RepositoryResult<StoredWebSession>> {
        Box::pin(async move {
            let stored = StoredWebSession {
                id: session.id,
                account: PrincipalAccount {
                    principal: registration.principal.clone(),
                    matrix_user_id: registration.matrix_user_id.clone(),
                    display_name: registration.display_name.clone(),
                    avatar_content_id: registration.avatar_content_id,
                    locale: registration.locale.clone(),
                },
                authenticated_at: session.authenticated_at,
                created_at: session.created_at,
                expires_at: session.expires_at,
            };
            let mut state = self.0.lock().expect("测试锁可用");
            state.last_registration = Some(registration.clone());
            state.sessions.insert(session.secret_digest, stored.clone());
            Ok(stored)
        })
    }
}

impl DesktopLoginCompletionTransaction for InMemoryIdentity {
    fn complete_desktop<'a>(
        &'a self,
        registration: &'a PrincipalRegistration,
        authorization: &'a DesktopAuthorizationCodeRegistration,
    ) -> PortFuture<'a, RepositoryResult<PrincipalAccount>> {
        Box::pin(async move {
            let account = PrincipalAccount {
                principal: registration.principal.clone(),
                matrix_user_id: registration.matrix_user_id.clone(),
                display_name: registration.display_name.clone(),
                avatar_content_id: registration.avatar_content_id,
                locale: registration.locale.clone(),
            };
            let mut state = self.0.lock().expect("测试锁可用");
            state.last_registration = Some(registration.clone());
            state.desktop_authorizations.insert(
                authorization.code_digest,
                (authorization.clone(), account.clone()),
            );
            Ok(account)
        })
    }
}

impl DesktopSessionExchangeTransaction for InMemoryIdentity {
    fn exchange_desktop<'a>(
        &'a self,
        code_digest: &'a SecretDigest,
        code_challenge: &'a PkceCodeChallenge,
        session: &'a DesktopSessionRegistration,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<StoredWebSession>>> {
        Box::pin(async move {
            let mut state = self.0.lock().expect("测试锁可用");
            let matches =
                state
                    .desktop_authorizations
                    .get(code_digest)
                    .is_some_and(|(authorization, _)| {
                        authorization.code_challenge == *code_challenge
                            && now < authorization.expires_at
                    });
            let Some((authorization, account)) = matches
                .then(|| state.desktop_authorizations.remove(code_digest))
                .flatten()
            else {
                return Ok(None);
            };
            let stored = StoredWebSession {
                id: session.id,
                account,
                authenticated_at: authorization.authenticated_at,
                created_at: session.created_at,
                expires_at: session.expires_at,
            };
            state.sessions.insert(session.secret_digest, stored.clone());
            Ok(Some(stored))
        })
    }
}

impl WebSessionStore for InMemoryIdentity {
    fn find_active<'a>(
        &'a self,
        secret_digest: &'a SecretDigest,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<StoredWebSession>>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .expect("测试锁可用")
                .sessions
                .get(secret_digest)
                .filter(|session| now < session.expires_at)
                .cloned())
        })
    }

    fn revoke<'a>(
        &'a self,
        secret_digest: &'a SecretDigest,
        _now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .expect("测试锁可用")
                .sessions
                .remove(secret_digest)
                .is_some())
        })
    }
}

impl PrincipalSuspensionTransaction for InMemoryIdentity {
    fn suspend(
        &self,
        principal_id: PrincipalId,
        _now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Principal>> {
        Box::pin(async move {
            let mut state = self.0.lock().expect("测试锁可用");
            let mut suspended = None;
            for session in state.sessions.values_mut() {
                if session.account.principal.id() == principal_id {
                    session.account.principal.suspend().expect("测试主体可暂停");
                    suspended = Some(session.account.principal.clone());
                }
            }
            suspended.ok_or_else(|| {
                RepositoryError::new("test.principal.suspend", RepositoryErrorKind::NotFound)
            })
        })
    }
}

struct Harness {
    service: AuthenticationService,
    oidc: Arc<TestOidc>,
    storage: Arc<InMemoryIdentity>,
    clock: Arc<TestClock>,
}

impl Harness {
    fn new(identity: VerifiedOidcIdentity) -> Self {
        let oidc = Arc::new(TestOidc::new(identity));
        let storage = Arc::new(InMemoryIdentity::default());
        let clock = Arc::new(TestClock(AtomicI64::new(NOW)));
        let service = AuthenticationService::new(
            AuthenticationDependencies {
                oidc: oidc.clone(),
                login_attempts: storage.clone(),
                login_completion: storage.clone(),
                desktop_login_completion: storage.clone(),
                desktop_session_exchange: storage.clone(),
                sessions: storage.clone(),
                suspensions: storage.clone(),
                secrets: Arc::new(TestSecrets::default()),
                identifiers: Arc::new(TestIdentifiers),
                clock: clock.clone(),
            },
            policy(),
        );
        Self {
            service,
            oidc,
            storage,
            clock,
        }
    }

    async fn begin(
        &self,
        profile_import: ProfileImportConsent,
    ) -> agent_room_application::authentication::LoginRedirect {
        self.service
            .begin_login(BeginLogin {
                delivery: LoginDelivery::Web {
                    return_path: SafeReturnPath::new("/rooms/lobby").expect("返回路径有效"),
                },
                profile_import,
                intent: AuthenticationIntent::SignIn,
            })
            .await
            .expect("登录应开始")
    }

    async fn complete(
        &self,
        browser_secret: &SecretValue,
        state: &str,
    ) -> agent_room_application::authentication::AuthenticationResult<WebLoginCompletion> {
        let completion = self
            .service
            .complete_login(CompleteLogin {
                code: "authorization-code",
                returned_state: state,
                browser_secret,
            })
            .await?;
        match completion {
            LoginCompletion::Web(completion) => Ok(completion),
            LoginCompletion::Desktop(_) => panic!("测试请求必须完成 Web 登录"),
        }
    }
}

#[tokio::test]
async fn 注册意图被明确映射为_oidc_创建账户交互() {
    let harness = Harness::new(valid_identity(Some(time(NOW))));

    harness
        .service
        .begin_login(BeginLogin {
            delivery: LoginDelivery::Web {
                return_path: SafeReturnPath::new("/onboarding").expect("返回路径有效"),
            },
            profile_import: ProfileImportConsent::default(),
            intent: AuthenticationIntent::Register,
        })
        .await
        .expect("注册授权应开始");

    assert_eq!(
        *harness
            .oidc
            .requested_interaction
            .lock()
            .expect("测试锁可用"),
        Some(OidcInteraction::CreateAccount)
    );
}

#[tokio::test]
async fn 错误浏览器状态不会消费尝试或调用令牌端点() {
    let harness = Harness::new(valid_identity(Some(time(NOW))));
    let redirect = harness.begin(ProfileImportConsent::default()).await;

    let failure = harness
        .complete(&redirect.browser_secret, "attacker-state")
        .await
        .expect_err("伪造 state 必须失败");
    assert_eq!(failure.kind(), AuthenticationFailureKind::InvalidLoginState);
    assert_eq!(harness.oidc.exchanges.load(Ordering::SeqCst), 0);

    harness
        .complete(&redirect.browser_secret, STATE)
        .await
        .expect("错误尝试不应消费正确登录状态");
    assert_eq!(harness.oidc.exchanges.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn 桌面授权码受_pkce_过期和单次消费共同约束() {
    let harness = Harness::new(valid_identity(Some(time(NOW))));
    let verifier = "v".repeat(43);
    let challenge =
        PkceCodeChallenge::new(URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())))
            .expect("PKCE challenge 有效");
    let redirect = harness
        .service
        .begin_login(BeginLogin {
            delivery: LoginDelivery::Desktop {
                client_state: DesktopClientState::new("s".repeat(43)).expect("桌面 state 有效"),
                code_challenge: challenge,
                return_path: SafeReturnPath::new("/workspace").expect("返回路径有效"),
            },
            profile_import: ProfileImportConsent::default(),
            intent: AuthenticationIntent::SignIn,
        })
        .await
        .expect("桌面登录应开始");
    let completion = harness
        .service
        .complete_login(CompleteLogin {
            code: "authorization-code",
            returned_state: STATE,
            browser_secret: &redirect.browser_secret,
        })
        .await
        .expect("OIDC 回调应创建桌面授权码");
    let LoginCompletion::Desktop(completion) = completion else {
        panic!("桌面登录不得提前创建 Web 会话");
    };
    assert_eq!(completion.client_state.expose(), "s".repeat(43));
    assert!(
        harness
            .storage
            .0
            .lock()
            .expect("测试锁可用")
            .sessions
            .is_empty()
    );

    let wrong = harness
        .service
        .exchange_desktop_authorization(ExchangeDesktopAuthorization {
            authorization_code: completion.authorization_code.expose(),
            pkce_verifier: &"x".repeat(43),
        })
        .await
        .expect_err("错误 verifier 必须失败");
    assert_eq!(wrong.kind(), AuthenticationFailureKind::InvalidLoginState);

    let exchanged = harness
        .service
        .exchange_desktop_authorization(ExchangeDesktopAuthorization {
            authorization_code: completion.authorization_code.expose(),
            pkce_verifier: &verifier,
        })
        .await
        .expect("正确 verifier 可交换一次");
    harness
        .service
        .authenticate(
            &exchanged.session_secret,
            AuthenticationRequirement::ActiveSession,
        )
        .await
        .expect("交换后的桌面会话有效");
    let replay = harness
        .service
        .exchange_desktop_authorization(ExchangeDesktopAuthorization {
            authorization_code: completion.authorization_code.expose(),
            pkce_verifier: &verifier,
        })
        .await
        .expect_err("授权码重放必须失败");
    assert_eq!(replay.kind(), AuthenticationFailureKind::InvalidLoginState);

    let expired = Harness::new(valid_identity(Some(time(NOW))));
    let redirect = expired
        .service
        .begin_login(BeginLogin {
            delivery: LoginDelivery::Desktop {
                client_state: DesktopClientState::new("e".repeat(43)).expect("桌面 state 有效"),
                code_challenge: PkceCodeChallenge::new(
                    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
                )
                .expect("PKCE challenge 有效"),
                return_path: SafeReturnPath::new("/workspace").expect("返回路径有效"),
            },
            profile_import: ProfileImportConsent::default(),
            intent: AuthenticationIntent::SignIn,
        })
        .await
        .expect("桌面登录应开始");
    let completion = expired
        .service
        .complete_login(CompleteLogin {
            code: "authorization-code",
            returned_state: STATE,
            browser_secret: &redirect.browser_secret,
        })
        .await
        .expect("OIDC 回调应创建桌面授权码");
    let LoginCompletion::Desktop(completion) = completion else {
        panic!("桌面登录必须返回一次性授权码");
    };
    expired.clock.set(NOW + 600_000);
    let failure = expired
        .service
        .exchange_desktop_authorization(ExchangeDesktopAuthorization {
            authorization_code: completion.authorization_code.expose(),
            pkce_verifier: &verifier,
        })
        .await
        .expect_err("到期授权码必须失败");
    assert_eq!(failure.kind(), AuthenticationFailureKind::InvalidLoginState);
}

#[tokio::test]
async fn 缺失或过旧_auth_time_都拒绝创建会话() {
    for identity in [
        valid_identity(None),
        valid_identity(Some(time(NOW - 361_000))),
        valid_identity(Some(time(NOW + 61_000))),
    ] {
        let harness = Harness::new(identity);
        let redirect = harness.begin(ProfileImportConsent::default()).await;
        let failure = harness
            .complete(&redirect.browser_secret, STATE)
            .await
            .expect_err("不满足近期认证窗口的声明必须失败");
        assert_eq!(
            failure.kind(),
            AuthenticationFailureKind::InvalidIdentityToken
        );
        assert!(
            harness
                .storage
                .0
                .lock()
                .expect("测试锁可用")
                .sessions
                .is_empty()
        );
    }
}

#[tokio::test]
async fn 第三方资料只在逐字段同意后进入主体投影() {
    let without_consent = Harness::new(valid_identity(Some(time(NOW))));
    let redirect = without_consent.begin(ProfileImportConsent::default()).await;
    without_consent
        .complete(&redirect.browser_secret, STATE)
        .await
        .expect("登录成功");
    {
        let state = without_consent.storage.0.lock().expect("测试锁可用");
        let registration = state.last_registration.as_ref().expect("主体投影存在");
        assert!(registration.display_name.starts_with("Agent Room User "));
        assert_eq!(registration.locale, "en");
    }
    assert_eq!(
        *without_consent
            .oidc
            .requested_profile
            .lock()
            .expect("测试锁可用"),
        Some(false)
    );
    let with_consent = Harness::new(valid_identity(Some(time(NOW))));
    let redirect = with_consent
        .begin(ProfileImportConsent {
            display_name: true,
            locale: true,
        })
        .await;
    with_consent
        .complete(&redirect.browser_secret, STATE)
        .await
        .expect("登录成功");
    let state = with_consent.storage.0.lock().expect("测试锁可用");
    let registration = state.last_registration.as_ref().expect("主体投影存在");
    assert_eq!(registration.display_name, "Alice");
    assert_eq!(registration.locale, "zh-CN");
    assert_eq!(
        *with_consent
            .oidc
            .requested_profile
            .lock()
            .expect("测试锁可用"),
        Some(true)
    );
}

#[tokio::test]
async fn 近期认证会过期_普通会话继续_登出后立即失效() {
    let harness = Harness::new(valid_identity(Some(time(NOW))));
    let redirect = harness.begin(ProfileImportConsent::default()).await;
    let completion = harness
        .complete(&redirect.browser_secret, STATE)
        .await
        .expect("登录成功");

    harness
        .service
        .authenticate(
            &completion.session_secret,
            AuthenticationRequirement::RecentAuthentication,
        )
        .await
        .expect("刚登录时满足近期认证");
    harness.clock.set(NOW + 361_000);
    let failure = harness
        .service
        .authenticate(
            &completion.session_secret,
            AuthenticationRequirement::RecentAuthentication,
        )
        .await
        .expect_err("近期认证窗口结束后必须重新认证");
    assert_eq!(
        failure.kind(),
        AuthenticationFailureKind::ReauthenticationRequired
    );
    harness
        .service
        .authenticate(
            &completion.session_secret,
            AuthenticationRequirement::ActiveSession,
        )
        .await
        .expect("普通会话仍有效");
    harness
        .service
        .logout(&completion.session_secret)
        .await
        .expect("登出成功");
    let failure = harness
        .service
        .authenticate(
            &completion.session_secret,
            AuthenticationRequirement::ActiveSession,
        )
        .await
        .expect_err("登出后会话必须失效");
    assert_eq!(failure.kind(), AuthenticationFailureKind::InvalidSession);
}

#[tokio::test]
async fn 主体暂停后已有会话不能继续认证() {
    let harness = Harness::new(valid_identity(Some(time(NOW))));
    let redirect = harness.begin(ProfileImportConsent::default()).await;
    let completion = harness
        .complete(&redirect.browser_secret, STATE)
        .await
        .expect("登录成功");
    harness
        .service
        .suspend_principal(completion.principal.principal_id)
        .await
        .expect("暂停成功");

    let failure = harness
        .service
        .authenticate(
            &completion.session_secret,
            AuthenticationRequirement::ActiveSession,
        )
        .await
        .expect_err("暂停主体不得认证");
    assert_eq!(
        failure.kind(),
        AuthenticationFailureKind::PrincipalSuspended
    );
}

fn valid_identity(authenticated_at: Option<UtcMillis>) -> VerifiedOidcIdentity {
    VerifiedOidcIdentity::new(
        "https://identity.example/realms/agent-room",
        "stable-subject",
        Some("Alice".to_owned()),
        Some("zh-CN".to_owned()),
        authenticated_at,
    )
    .expect("测试身份有效")
}

fn policy() -> AuthenticationPolicy {
    AuthenticationPolicy::new(
        duration(600_000),
        duration(28_800_000),
        duration(300_000),
        duration(60_000),
        "matrix.agent-room.test",
    )
    .expect("认证策略有效")
}

fn duration(value: u64) -> DurationMillis {
    DurationMillis::new(value).expect("测试时长有效")
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
