use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use agent_room_application::{
    devices::{
        AuthenticateDeviceRequest, DeviceAuthorizationDependencies, DeviceAuthorizationFailureKind,
        DeviceAuthorizationPolicy, DeviceAuthorizationService, DeviceAuthorizationUseCases,
        DeviceRequestProof, DeviceRequestProofPayload, RefreshDeviceSession, RegisterDevice,
        VerifiedDeviceAuthorization,
    },
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        Clock, DeviceProofNonceStore, DeviceProofVerifier, DeviceRefreshContext,
        DeviceRefreshOutcome, DeviceRegistrationTransaction, DeviceRepository,
        DeviceRevocationOutcome, DeviceRevocationTransaction, DeviceSecurityEvent,
        DeviceSessionRegistration, DeviceSessionStore, DeviceSignature, DeviceTokenReplacement,
        IdentifierFactory, PortFuture, PrincipalAccount, PrincipalRegistration,
        ProfileImportConsent, SecretDigest, SecretFactory, SecretGenerationFailure, SecretValue,
        StoredDeviceSession, VerifiedOidcIdentity,
    },
};
use agent_room_domain::{
    devices::{Device, DevicePlatform, DevicePublicSigningKey, DeviceTrustState},
    ids::{
        AdapterBindingId, AgentCardSnapshotId, AgentId, AgentInstanceId, AutomationGrantId,
        ContentId, DeviceAccessTokenId, DeviceId, DeviceRefreshTokenId, DeviceTokenFamilyId,
        HandoffId, LoginAttemptId, OutboxEventId, PrincipalId, RoomCatalogId, RoomInstanceId,
        WebSessionId,
    },
    time::{DurationMillis, UtcMillis},
};
use uuid::Uuid;

const NOW: i64 = 1_700_000_000_000;

#[derive(Default)]
struct MemoryDeviceStore {
    state: Mutex<MemoryState>,
}

#[derive(Default)]
struct MemoryState {
    session: Option<StoredDeviceSession>,
    access_token_digest: Option<SecretDigest>,
    refresh_tokens: HashMap<SecretDigest, bool>,
}

impl DeviceRegistrationTransaction for MemoryDeviceStore {
    fn register<'a>(
        &'a self,
        principal: &'a PrincipalRegistration,
        device: &'a Device,
        session: &'a DeviceSessionRegistration,
    ) -> PortFuture<'a, RepositoryResult<StoredDeviceSession>> {
        Box::pin(async move {
            let stored = StoredDeviceSession {
                account: PrincipalAccount {
                    principal: principal.principal.clone(),
                    matrix_user_id: principal.matrix_user_id.clone(),
                    display_name: principal.display_name.clone(),
                    avatar_content_id: principal.avatar_content_id,
                    locale: principal.locale.clone(),
                },
                device: device.clone(),
                family: session.family.clone(),
                access_token_expires_at: session.access_token_expires_at,
            };
            let mut state = self.state.lock().expect("测试设备仓储锁不得中毒");
            state.access_token_digest = Some(session.access_token_digest);
            state
                .refresh_tokens
                .insert(session.refresh_token_digest, false);
            state.session = Some(stored.clone());
            Ok(stored)
        })
    }
}

impl DeviceSessionStore for MemoryDeviceStore {
    fn find_active_access<'a>(
        &'a self,
        access_token_digest: &'a SecretDigest,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<Option<StoredDeviceSession>>> {
        Box::pin(async move {
            let state = self.state.lock().expect("测试设备仓储锁不得中毒");
            let active = state.access_token_digest.as_ref() == Some(access_token_digest)
                && state.session.as_ref().is_some_and(|session| {
                    session.device.accepts_authenticated_requests()
                        && session.family.allows_rotation(now)
                        && now < session.access_token_expires_at
                });
            Ok(active.then(|| state.session.clone()).flatten())
        })
    }

    fn find_refresh_context<'a>(
        &'a self,
        refresh_token_digest: &'a SecretDigest,
    ) -> PortFuture<'a, RepositoryResult<Option<DeviceRefreshContext>>> {
        Box::pin(async move {
            let state = self.state.lock().expect("测试设备仓储锁不得中毒");
            if !state.refresh_tokens.contains_key(refresh_token_digest) {
                return Ok(None);
            }
            Ok(state.session.as_ref().map(|session| DeviceRefreshContext {
                account: session.account.clone(),
                device: session.device.clone(),
                family: session.family.clone(),
            }))
        })
    }

    fn rotate_refresh<'a>(
        &'a self,
        refresh_token_digest: &'a SecretDigest,
        replacement: &'a DeviceTokenReplacement,
        security_event: DeviceSecurityEvent,
    ) -> PortFuture<'a, RepositoryResult<DeviceRefreshOutcome>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("测试设备仓储锁不得中毒");
            let Some(consumed) = state.refresh_tokens.get(refresh_token_digest).copied() else {
                return Ok(DeviceRefreshOutcome::Rejected);
            };
            if consumed {
                let session = state.session.as_mut().ok_or_else(|| {
                    RepositoryError::new("device.refresh", RepositoryErrorKind::CorruptData)
                })?;
                session
                    .device
                    .revoke(security_event.occurred_at)
                    .map_err(|_| {
                        RepositoryError::new("device.refresh", RepositoryErrorKind::CorruptData)
                    })?;
                session
                    .family
                    .mark_compromised(security_event.occurred_at)
                    .map_err(|_| {
                        RepositoryError::new("device.refresh", RepositoryErrorKind::CorruptData)
                    })?;
                return Ok(DeviceRefreshOutcome::ReuseDetected {
                    device_id: session.device.id(),
                    principal_id: session.account.principal.id(),
                });
            }

            state.refresh_tokens.insert(*refresh_token_digest, true);
            state
                .refresh_tokens
                .insert(replacement.refresh_token_digest, false);
            state.access_token_digest = Some(replacement.access_token_digest);
            let session = state.session.as_mut().ok_or_else(|| {
                RepositoryError::new("device.refresh", RepositoryErrorKind::CorruptData)
            })?;
            session.access_token_expires_at = replacement.access_token_expires_at;
            Ok(DeviceRefreshOutcome::Rotated {
                refresh_token_expires_at: session.family.expires_at(),
                session: Box::new(session.clone()),
            })
        })
    }
}

impl DeviceRepository for MemoryDeviceStore {
    fn list_for_principal(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<Device>>> {
        Box::pin(async move {
            let state = self.state.lock().expect("测试设备仓储锁不得中毒");
            Ok(state
                .session
                .as_ref()
                .filter(|session| session.account.principal.id() == principal_id)
                .map(|session| vec![session.device.clone()])
                .unwrap_or_default())
        })
    }
}

impl DeviceRevocationTransaction for MemoryDeviceStore {
    fn revoke(
        &self,
        principal_id: PrincipalId,
        device_id: DeviceId,
        security_event: DeviceSecurityEvent,
    ) -> PortFuture<'_, RepositoryResult<DeviceRevocationOutcome>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("测试设备仓储锁不得中毒");
            let Some(session) = state.session.as_mut() else {
                return Ok(DeviceRevocationOutcome::NotFound);
            };
            if session.account.principal.id() != principal_id || session.device.id() != device_id {
                return Ok(DeviceRevocationOutcome::NotFound);
            }
            if session.device.trust_state() == DeviceTrustState::Revoked {
                return Ok(DeviceRevocationOutcome::AlreadyRevoked);
            }
            session
                .device
                .revoke(security_event.occurred_at)
                .map_err(|_| {
                    RepositoryError::new("device.revoke", RepositoryErrorKind::CorruptData)
                })?;
            session
                .family
                .revoke(security_event.occurred_at)
                .map_err(|_| {
                    RepositoryError::new("device.revoke", RepositoryErrorKind::CorruptData)
                })?;
            Ok(DeviceRevocationOutcome::Revoked)
        })
    }
}

#[derive(Default)]
struct MemoryNonces(Mutex<HashSet<(DeviceId, SecretDigest)>>);

impl DeviceProofNonceStore for MemoryNonces {
    fn consume<'a>(
        &'a self,
        device_id: DeviceId,
        nonce_digest: &'a SecretDigest,
        _consumed_at: UtcMillis,
        _expires_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .expect("测试 nonce 锁不得中毒")
                .insert((device_id, *nonce_digest)))
        })
    }
}

struct ToggleProofVerifier(AtomicBool);

impl DeviceProofVerifier for ToggleProofVerifier {
    fn verify(
        &self,
        _public_key: &DevicePublicSigningKey,
        _signed_message: &[u8],
        _signature: &DeviceSignature,
    ) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Default)]
struct SequentialSecrets(AtomicUsize);

impl SecretFactory for SequentialSecrets {
    fn generate(&self) -> Result<SecretValue, SecretGenerationFailure> {
        let sequence = self.0.fetch_add(1, Ordering::SeqCst);
        SecretValue::new(format!("device-secret-{sequence:08}"))
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

struct StaticClock;

impl Clock for StaticClock {
    fn now(&self) -> UtcMillis {
        time(NOW)
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

#[tokio::test]
async fn 设备请求证明只能消费一次() {
    let (service, _, secrets, _) = service(true);
    let credentials = service
        .register_device(registration(&secrets))
        .await
        .expect("设备注册成功");
    let proof = proof(
        credentials.device.device_id,
        "GET",
        "/v1/devices",
        "nonce-0000000001",
    );

    service
        .authenticate_device(AuthenticateDeviceRequest {
            access_token: &credentials.access_token,
            proof: &proof,
        })
        .await
        .expect("首次证明有效");
    let replay = service
        .authenticate_device(AuthenticateDeviceRequest {
            access_token: &credentials.access_token,
            proof: &proof,
        })
        .await
        .expect_err("重复证明必须失败");

    assert_eq!(replay.kind(), DeviceAuthorizationFailureKind::ProofReplay);
}

#[tokio::test]
async fn 旧刷新令牌重用会原子撤销设备和整个_token_族() {
    let (service, store, secrets, _) = service(true);
    let credentials = service
        .register_device(registration(&secrets))
        .await
        .expect("设备注册成功");
    let proof = proof(
        credentials.device.device_id,
        "POST",
        "/v1/device-sessions/refresh",
        "nonce-0000000002",
    );

    service
        .refresh_device_session(RefreshDeviceSession {
            refresh_token: &credentials.refresh_token,
            proof: &proof,
        })
        .await
        .expect("首次轮换成功");
    let reuse = service
        .refresh_device_session(RefreshDeviceSession {
            refresh_token: &credentials.refresh_token,
            proof: &proof,
        })
        .await
        .expect_err("旧刷新令牌不能再次使用");

    assert_eq!(
        reuse.kind(),
        DeviceAuthorizationFailureKind::RefreshTokenReuse
    );
    let state = store.state.lock().expect("测试设备仓储锁不得中毒");
    let session = state.session.as_ref().expect("会话仍保留审计状态");
    assert_eq!(session.device.trust_state(), DeviceTrustState::Revoked);
    assert!(!session.family.allows_rotation(time(NOW)));
}

#[tokio::test]
async fn 公钥持有证明失败时不会写入设备() {
    let (service, store, secrets, _) = service(false);
    let failure = service
        .register_device(registration(&secrets))
        .await
        .expect_err("无效签名必须失败");

    assert_eq!(failure.kind(), DeviceAuthorizationFailureKind::InvalidProof);
    assert!(
        store
            .state
            .lock()
            .expect("测试设备仓储锁不得中毒")
            .session
            .is_none()
    );
}

fn service(
    valid_proof: bool,
) -> (
    DeviceAuthorizationService,
    Arc<MemoryDeviceStore>,
    Arc<SequentialSecrets>,
    Arc<ToggleProofVerifier>,
) {
    let store = Arc::new(MemoryDeviceStore::default());
    let secrets = Arc::new(SequentialSecrets::default());
    let proof_verifier = Arc::new(ToggleProofVerifier(AtomicBool::new(valid_proof)));
    let service = DeviceAuthorizationService::new(
        DeviceAuthorizationDependencies {
            registrations: store.clone(),
            sessions: store.clone(),
            proof_nonces: Arc::new(MemoryNonces::default()),
            proof_verifier: proof_verifier.clone(),
            devices: store.clone(),
            revocations: store.clone(),
            secrets: secrets.clone(),
            identifiers: Arc::new(TestIdentifiers),
            clock: Arc::new(StaticClock),
        },
        DeviceAuthorizationPolicy::new(
            duration(5 * 60 * 1_000),
            duration(30 * 24 * 60 * 60 * 1_000),
            duration(60_000),
            duration(30_000),
            duration(10 * 60 * 1_000),
            "matrix.example.test",
        )
        .expect("设备策略有效"),
    );
    (service, store, secrets, proof_verifier)
}

fn registration(secrets: &SequentialSecrets) -> RegisterDevice {
    let identity = VerifiedOidcIdentity::new(
        "https://issuer.example",
        "stable-subject",
        None,
        None,
        Some(time(NOW)),
    )
    .expect("身份有效");
    RegisterDevice {
        authorization: VerifiedDeviceAuthorization::new(
            identity,
            secrets.digest("short-lived-oidc-token"),
        ),
        label: "开发工作站".to_owned(),
        platform: DevicePlatform::Windows,
        public_signing_key: DevicePublicSigningKey::new(vec![3; 32]).expect("公钥有效"),
        possession_signature: DeviceSignature::new(vec![4; 64]).expect("签名长度有效"),
        profile_import: ProfileImportConsent::default(),
    }
}

fn proof(device_id: DeviceId, method: &str, target: &str, nonce: &str) -> DeviceRequestProof {
    let payload = DeviceRequestProofPayload::new(
        device_id,
        time(NOW),
        SecretValue::new(nonce).expect("nonce 有效"),
        method.to_owned(),
        target.to_owned(),
        SecretDigest::from_array([0; 32]),
    )
    .expect("证明载荷有效");
    DeviceRequestProof::new(
        payload,
        DeviceSignature::new(vec![5; 64]).expect("签名长度有效"),
    )
}

fn duration(value: u64) -> DurationMillis {
    DurationMillis::new(value).expect("测试时长有效")
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
