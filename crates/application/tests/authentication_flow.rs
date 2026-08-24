use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
};

use agent_room_application::{
    authentication::{
        AuthenticationDependencies, AuthenticationFailureKind, AuthenticationPolicy,
        AuthenticationRequirement, AuthenticationService, AuthenticationUseCases, BeginLogin,
        CompleteLogin,
    },
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        Clock, IdentifierFactory, LoginAttempt, LoginAttemptStore, LoginCompletionTransaction,
        OidcAuthorizationOptions, OidcAuthorizationRequest, OidcCodeExchange, OidcGateway,
        OidcResult, PortFuture, PrincipalAccount, PrincipalRegistration,
        PrincipalSuspensionTransaction, ProfileImportConsent, SafeReturnPath, SecretDigest,
        SecretFactory, SecretGenerationFailure, SecretValue, StoredWebSession,
        VerifiedOidcIdentity, WebSessionRegistration, WebSessionStore,
    },
};
use agent_room_domain::{
    identity::Principal,
    ids::{
        AgentId, AgentInstanceId, AutomationGrantId, ContentId, DeviceId, HandoffId,
        LoginAttemptId, OutboxEventId, PrincipalId, RoomCatalogId, RoomInstanceId, WebSessionId,
    },
    time::{DurationMillis, UtcMillis},
};
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

    fn agent_id(&self) -> AgentId {
        AgentId::from_uuid(Uuid::now_v7())
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
}

impl TestOidc {
    fn new(identity: VerifiedOidcIdentity) -> Self {
        Self {
            identity: Mutex::new(identity),
            exchanges: AtomicUsize::new(0),
            requested_profile: Mutex::new(None),
        }
    }
}

impl OidcGateway for TestOidc {
    fn begin_authorization(
        &self,
        options: OidcAuthorizationOptions,
    ) -> PortFuture<'_, OidcResult<OidcAuthorizationRequest>> {
        *self.requested_profile.lock().expect("测试锁可用") = Some(options.request_profile);
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
                return_path: SafeReturnPath::new("/rooms/lobby").expect("返回路径有效"),
                profile_import,
            })
            .await
            .expect("登录应开始")
    }

    async fn complete(
        &self,
        browser_secret: &SecretValue,
        state: &str,
    ) -> agent_room_application::authentication::AuthenticationResult<
        agent_room_application::authentication::LoginCompletion,
    > {
        self.service
            .complete_login(CompleteLogin {
                code: "authorization-code",
                returned_state: state,
                browser_secret,
            })
            .await
    }
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
