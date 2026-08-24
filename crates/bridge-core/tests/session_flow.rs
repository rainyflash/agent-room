use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use agent_room_application::{
    devices::{AuthenticatedDevice, DeviceCredentials},
    ports::{Clock, DeviceSignature, PortFuture, PrincipalAccount, SecretFactory, SecretValue},
};
use agent_room_bridge_core::{
    ports::{
        BridgeCredentialResult, BridgeCredentialState, ControlPlaneDeviceFailure,
        ControlPlaneDeviceFailureKind, ControlPlaneDeviceGateway, ControlPlaneDeviceResult,
        DeviceCredentialVault, DeviceSigningIdentity, DeviceSigningIdentityStore,
        RefreshBridgeDevice, RegisterBridgeDevice, StoredBridgeDeviceCredentials,
    },
    session::{
        BridgeSessionDependencies, BridgeSessionFailureKind, BridgeSessionPolicy,
        BridgeSessionService,
    },
};
use agent_room_domain::{
    devices::DevicePublicSigningKey,
    identity::Principal,
    ids::{DeviceId, PrincipalId},
    time::{DurationMillis, UtcMillis},
};
use agent_room_identity_adapter::SecureSecretFactory;
use uuid::Uuid;

struct 测试时钟(UtcMillis);

impl Clock for 测试时钟 {
    fn now(&self) -> UtcMillis {
        self.0
    }
}

struct 测试签名身份 {
    messages: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl DeviceSigningIdentity for 测试签名身份 {
    fn public_key(&self) -> BridgeCredentialResult<DevicePublicSigningKey> {
        Ok(DevicePublicSigningKey::new(vec![8; 32]).expect("测试公钥有效"))
    }

    fn sign(&self, message: &[u8]) -> BridgeCredentialResult<DeviceSignature> {
        self.messages
            .lock()
            .expect("签名记录锁可用")
            .push(message.to_vec());
        Ok(DeviceSignature::new(vec![9; 64]).expect("测试签名有效"))
    }
}

struct 测试签名存储 {
    identity: Arc<测试签名身份>,
}

impl DeviceSigningIdentityStore for 测试签名存储 {
    fn load_or_create(&self) -> BridgeCredentialResult<Arc<dyn DeviceSigningIdentity>> {
        Ok(self.identity.clone())
    }
}

struct 内存凭据库 {
    value: Mutex<Option<StoredBridgeDeviceCredentials>>,
    writes: Mutex<Vec<BridgeCredentialState>>,
}

impl 内存凭据库 {
    fn new(value: StoredBridgeDeviceCredentials) -> Self {
        Self {
            value: Mutex::new(Some(value)),
            writes: Mutex::new(Vec::new()),
        }
    }
}

impl DeviceCredentialVault for 内存凭据库 {
    fn load(&self) -> BridgeCredentialResult<Option<StoredBridgeDeviceCredentials>> {
        Ok(self.value.lock().expect("凭据锁可用").clone())
    }

    fn replace(&self, credentials: &StoredBridgeDeviceCredentials) -> BridgeCredentialResult<()> {
        self.writes
            .lock()
            .expect("写入记录锁可用")
            .push(credentials.state);
        *self.value.lock().expect("凭据锁可用") = Some(credentials.clone());
        Ok(())
    }

    fn clear(&self) -> BridgeCredentialResult<()> {
        self.value.lock().expect("凭据锁可用").take();
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum 刷新结果 {
    成功,
    结果未知,
}

struct 测试控制平面 {
    outcome: 刷新结果,
    requests: Mutex<Vec<RefreshBridgeDevice>>,
    calls: AtomicUsize,
}

impl 测试控制平面 {
    fn new(outcome: 刷新结果) -> Self {
        Self {
            outcome,
            requests: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
        }
    }
}

impl ControlPlaneDeviceGateway for 测试控制平面 {
    fn register(
        &self,
        _request: RegisterBridgeDevice,
    ) -> PortFuture<'_, ControlPlaneDeviceResult<DeviceCredentials>> {
        Box::pin(async { unreachable!("会话测试不会注册设备") })
    }

    fn refresh(
        &self,
        request: RefreshBridgeDevice,
    ) -> PortFuture<'_, ControlPlaneDeviceResult<DeviceCredentials>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.requests.lock().expect("刷新请求锁可用").push(request);
        let outcome = self.outcome;
        Box::pin(async move {
            match outcome {
                刷新结果::成功 => Ok(refreshed_credentials()),
                刷新结果::结果未知 => Err(ControlPlaneDeviceFailure::new(
                    ControlPlaneDeviceFailureKind::UnknownCommit,
                )),
            }
        })
    }
}

#[tokio::test]
async fn 尚有足够有效期的访问令牌不会触发刷新() {
    let vault = Arc::new(内存凭据库::new(stored_credentials(time(10_000))));
    let control_plane = Arc::new(测试控制平面::new(刷新结果::成功));
    let service = service(
        vault,
        control_plane.clone(),
        Arc::new(Mutex::new(Vec::new())),
    );

    let session = service.active_session().await.expect("现有会话可直接使用");

    assert_eq!(session.access_token.expose(), "old-access-token");
    assert_eq!(control_plane.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn 临近过期时先持久化刷新中状态再原子替换新凭据() {
    let vault = Arc::new(内存凭据库::new(stored_credentials(time(1_050))));
    let control_plane = Arc::new(测试控制平面::new(刷新结果::成功));
    let messages = Arc::new(Mutex::new(Vec::new()));
    let service = service(vault.clone(), control_plane.clone(), messages.clone());

    let session = service.active_session().await.expect("刷新会话成功");

    assert_eq!(session.access_token.expose(), "new-access-token");
    assert_eq!(
        vault.writes.lock().expect("写入记录锁可用").as_slice(),
        [
            BridgeCredentialState::RefreshPending,
            BridgeCredentialState::Ready
        ]
    );
    let requests = control_plane.requests.lock().expect("刷新请求锁可用");
    let request = requests.first().expect("刷新请求已发送");
    assert_eq!(request.refresh_token.expose(), "old-refresh-token");
    assert_eq!(request.proof.device_id(), device_id());
    assert_eq!(request.proof.method(), "POST");
    assert_eq!(request.proof.request_target(), "/auth/devices/refresh");
    let expected_message = request
        .proof
        .payload()
        .signing_message(&SecureSecretFactory.digest("old-refresh-token"));
    assert_eq!(
        messages.lock().expect("签名记录锁可用").as_slice(),
        [expected_message]
    );
}

#[tokio::test]
async fn 刷新结果未知会持久停在待决状态且绝不重放旧令牌() {
    let vault = Arc::new(内存凭据库::new(stored_credentials(time(1_050))));
    let control_plane = Arc::new(测试控制平面::new(刷新结果::结果未知));
    let service = service(
        vault.clone(),
        control_plane.clone(),
        Arc::new(Mutex::new(Vec::new())),
    );

    let first = service
        .active_session()
        .await
        .expect_err("未知提交不能继续使用旧会话");
    let second = service
        .active_session()
        .await
        .expect_err("再次调用也不能重放旧刷新令牌");

    assert_eq!(
        first.kind(),
        BridgeSessionFailureKind::RefreshOutcomeUnknown
    );
    assert_eq!(
        second.kind(),
        BridgeSessionFailureKind::RefreshOutcomeUnknown
    );
    assert_eq!(control_plane.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        vault
            .value
            .lock()
            .expect("凭据锁可用")
            .as_ref()
            .expect("待决凭据保留用于诊断")
            .state,
        BridgeCredentialState::RefreshPending
    );
}

fn service(
    vault: Arc<内存凭据库>,
    control_plane: Arc<测试控制平面>,
    messages: Arc<Mutex<Vec<Vec<u8>>>>,
) -> BridgeSessionService {
    BridgeSessionService::new(
        BridgeSessionDependencies {
            signing_identities: Arc::new(测试签名存储 {
                identity: Arc::new(测试签名身份 { messages }),
            }),
            control_plane,
            credentials: vault,
            secrets: Arc::new(SecureSecretFactory),
            clock: Arc::new(测试时钟(time(1_000))),
        },
        BridgeSessionPolicy::new(DurationMillis::new(100).expect("提前量有效")),
    )
}

fn stored_credentials(access_token_expires_at: UtcMillis) -> StoredBridgeDeviceCredentials {
    StoredBridgeDeviceCredentials {
        state: BridgeCredentialState::Ready,
        device_id: device_id(),
        access_token: SecretValue::new("old-access-token").expect("旧访问令牌有效"),
        access_token_expires_at,
        refresh_token: SecretValue::new("old-refresh-token").expect("旧刷新令牌有效"),
        refresh_token_expires_at: time(100_000),
    }
}

fn refreshed_credentials() -> DeviceCredentials {
    DeviceCredentials {
        device: AuthenticatedDevice {
            account: PrincipalAccount {
                principal: Principal::new(PrincipalId::from_uuid(Uuid::from_u128(1))),
                matrix_user_id: "@bridge:matrix.example".to_owned(),
                display_name: "Bridge 用户".to_owned(),
                avatar_content_id: None,
                locale: "zh-CN".to_owned(),
            },
            device_id: device_id(),
            access_token_expires_at: time(10_000),
        },
        access_token: SecretValue::new("new-access-token").expect("新访问令牌有效"),
        refresh_token: SecretValue::new("new-refresh-token").expect("新刷新令牌有效"),
        refresh_token_expires_at: time(100_000),
    }
}

fn device_id() -> DeviceId {
    DeviceId::from_uuid(Uuid::from_u128(2))
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
