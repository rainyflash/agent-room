use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::task::{Context, Waker};

use agent_room_application::{
    devices::{AuthenticatedDevice, DeviceCredentials},
    ports::{Clock, DeviceSignature, PortFuture, PrincipalAccount, SecretFactory, SecretValue},
};
use agent_room_bridge_core::{
    ports::{
        BridgeCredentialResult, BridgeCredentialState, ControlPlaneDeviceFailure,
        ControlPlaneDeviceFailureKind, ControlPlaneDeviceGateway, ControlPlaneDeviceResult,
        DeviceCredentialVault, DeviceRefreshAttemptIdFactory, DeviceSigningIdentity,
        DeviceSigningIdentityStore, RefreshBridgeDevice, RegisterBridgeDevice,
        StoredBridgeDeviceCredentials,
    },
    session::{
        ActiveBridgeSession, BridgeSessionDependencies, BridgeSessionFailureKind,
        BridgeSessionPolicy, BridgeSessionService,
    },
};
use agent_room_domain::{
    devices::DevicePublicSigningKey,
    identity::Principal,
    ids::{DeviceId, DeviceRefreshAttemptId, PrincipalId},
    time::{DurationMillis, UtcMillis},
};
use agent_room_identity_adapter::SecureSecretFactory;
use tokio::sync::Semaphore;
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

/// 按顺序发放可预测的尝试号，便于断言重试是否沿用了同一个。
#[derive(Default)]
struct 测试尝试号工厂 {
    issued: AtomicUsize,
}

impl DeviceRefreshAttemptIdFactory for 测试尝试号工厂 {
    fn refresh_attempt_id(&self) -> DeviceRefreshAttemptId {
        attempt(self.issued.fetch_add(1, Ordering::SeqCst) + 1)
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
    依赖不可用,
    结果未知,
    认证拒绝,
}

struct 测试控制平面 {
    outcomes: Mutex<VecDeque<刷新结果>>,
    requests: Mutex<Vec<RefreshBridgeDevice>>,
    calls: AtomicUsize,
    refresh_gate: Option<Semaphore>,
}

impl 测试控制平面 {
    fn new(outcome: 刷新结果) -> Self {
        Self::sequence([outcome])
    }

    fn sequence(outcomes: impl IntoIterator<Item = 刷新结果>) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
            refresh_gate: None,
        }
    }

    fn paused(outcome: 刷新结果) -> Self {
        Self {
            refresh_gate: Some(Semaphore::new(0)),
            ..Self::new(outcome)
        }
    }

    fn release_refresh(&self) {
        self.refresh_gate
            .as_ref()
            .expect("暂停的刷新应有响应门闩")
            .add_permits(1);
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
        let outcome = self
            .outcomes
            .lock()
            .expect("刷新结果锁可用")
            .pop_front()
            .expect("测试必须提供刷新结果");
        Box::pin(async move {
            if let Some(gate) = &self.refresh_gate {
                gate.acquire().await.expect("刷新响应门闩可用").forget();
            }
            match outcome {
                刷新结果::成功 => Ok(refreshed_credentials()),
                刷新结果::依赖不可用 => Err(ControlPlaneDeviceFailure::new(
                    ControlPlaneDeviceFailureKind::DependencyUnavailable,
                )),
                刷新结果::结果未知 => Err(ControlPlaneDeviceFailure::new(
                    ControlPlaneDeviceFailureKind::UnknownCommit,
                )),
                刷新结果::认证拒绝 => Err(ControlPlaneDeviceFailure::new(
                    ControlPlaneDeviceFailureKind::AuthenticationRejected,
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
async fn 控制面请求使用访问令牌签名精确方法目标和正文() {
    let vault = Arc::new(内存凭据库::new(stored_credentials(time(10_000))));
    let control_plane = Arc::new(测试控制平面::new(刷新结果::成功));
    let messages = Arc::new(Mutex::new(Vec::new()));
    let service = service(vault, control_plane.clone(), messages.clone());

    let authorized = service
        .authorize_request(
            "GET",
            "/agent-instances/0198b601-77a1-7bb8-83eb-a8fe68c97e47/verification",
            "",
        )
        .await
        .expect("控制面请求可授权");

    assert_eq!(authorized.access_token.expose(), "old-access-token");
    assert_eq!(authorized.proof.device_id(), device_id());
    assert_eq!(authorized.proof.method(), "GET");
    assert_eq!(
        authorized.proof.request_target(),
        "/agent-instances/0198b601-77a1-7bb8-83eb-a8fe68c97e47/verification"
    );
    assert_eq!(
        authorized.proof.body_digest(),
        &SecureSecretFactory.digest("")
    );
    let expected_message = authorized
        .proof
        .payload()
        .signing_message(&SecureSecretFactory.digest("old-access-token"));
    assert_eq!(
        messages.lock().expect("签名记录锁可用").as_slice(),
        [expected_message]
    );
    assert_eq!(control_plane.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn 控制面请求在访问令牌临期时先刷新再用新令牌签名() {
    let vault = Arc::new(内存凭据库::new(stored_credentials(time(1_050))));
    let control_plane = Arc::new(测试控制平面::new(刷新结果::成功));
    let messages = Arc::new(Mutex::new(Vec::new()));
    let service = service(vault, control_plane.clone(), messages.clone());

    let authorized = service
        .authorize_request("POST", "/messages", r#"{"body":"hello"}"#)
        .await
        .expect("刷新后控制面请求可授权");

    assert_eq!(authorized.access_token.expose(), "new-access-token");
    assert_eq!(control_plane.calls.load(Ordering::SeqCst), 1);
    let expected_message = authorized
        .proof
        .payload()
        .signing_message(&SecureSecretFactory.digest("new-access-token"));
    let signed_messages = messages.lock().expect("签名记录锁可用");
    assert_eq!(signed_messages.len(), 2);
    assert_eq!(signed_messages[1], expected_message);
}

#[tokio::test]
async fn 临近过期时先持久化带尝试号的刷新中状态再原子替换新凭据() {
    let vault = Arc::new(内存凭据库::new(stored_credentials(time(1_050))));
    let control_plane = Arc::new(测试控制平面::new(刷新结果::成功));
    let messages = Arc::new(Mutex::new(Vec::new()));
    let service = service(vault.clone(), control_plane.clone(), messages.clone());

    let session = service.active_session().await.expect("刷新会话成功");

    assert_eq!(session.access_token.expose(), "new-access-token");
    assert_eq!(
        vault.writes.lock().expect("写入记录锁可用").as_slice(),
        [pending(1), BridgeCredentialState::Ready]
    );
    let requests = control_plane.requests.lock().expect("刷新请求锁可用");
    let request = requests.first().expect("刷新请求已发送");
    assert_eq!(request.attempt_id, attempt(1));
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
async fn 并发临期请求等待同一次刷新并使用同一新会话() {
    let vault = Arc::new(内存凭据库::new(stored_credentials(time(1_050))));
    let control_plane = Arc::new(测试控制平面::paused(刷新结果::成功));
    let service = service(
        vault.clone(),
        control_plane.clone(),
        Arc::new(Mutex::new(Vec::new())),
    );
    let mut requests = std::array::from_fn::<_, 8, _>(|_| Box::pin(service.active_session()));

    for request in &mut requests {
        assert_pending(request.as_mut());
    }
    assert_eq!(control_plane.calls.load(Ordering::SeqCst), 1);

    control_plane.release_refresh();
    let expected = ActiveBridgeSession {
        device_id: device_id(),
        access_token: SecretValue::new("new-access-token").expect("新访问令牌有效"),
        access_token_expires_at: time(10_000),
    };
    for request in requests {
        assert_eq!(request.await.expect("并发请求应等待并取得新会话"), expected);
    }
    assert_eq!(control_plane.calls.load(Ordering::SeqCst), 1);
    let stored = vault.load().expect("凭据可读").expect("新凭据已保存");
    assert_eq!(stored.state, BridgeCredentialState::Ready);
    assert_eq!(stored.refresh_token.expose(), "new-refresh-token");
}

#[tokio::test]
async fn 并发刷新结果未知时等待者共享结论且不重复发送() {
    let vault = Arc::new(内存凭据库::new(stored_credentials(time(1_050))));
    let control_plane = Arc::new(测试控制平面::paused(刷新结果::结果未知));
    let service = service(
        vault.clone(),
        control_plane.clone(),
        Arc::new(Mutex::new(Vec::new())),
    );
    let mut requests = std::array::from_fn::<_, 4, _>(|_| Box::pin(service.active_session()));
    for request in &mut requests {
        assert_pending(request.as_mut());
    }

    control_plane.release_refresh();
    for request in requests {
        assert_eq!(
            request.await.expect_err("结果未知时禁止返回会话").kind(),
            BridgeSessionFailureKind::RefreshOutcomeUnknown
        );
    }
    assert_eq!(control_plane.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        vault.load().expect("凭据可读").expect("保留待决凭据").state,
        pending(1)
    );
}

#[tokio::test]
async fn 刷新被拒绝后等待者不能再使用或重放旧凭据() {
    let vault = Arc::new(内存凭据库::new(stored_credentials(time(1_050))));
    let control_plane = Arc::new(测试控制平面::paused(刷新结果::认证拒绝));
    let service = service(
        vault.clone(),
        control_plane.clone(),
        Arc::new(Mutex::new(Vec::new())),
    );
    let mut requests = std::array::from_fn::<_, 4, _>(|_| Box::pin(service.active_session()));
    for request in &mut requests {
        assert_pending(request.as_mut());
    }

    control_plane.release_refresh();
    for request in requests {
        assert_eq!(
            request.await.expect_err("已拒绝的设备不能取得会话").kind(),
            BridgeSessionFailureKind::NotAuthorized
        );
    }
    assert_eq!(control_plane.calls.load(Ordering::SeqCst), 1);
    assert!(vault.load().expect("凭据可读").is_none());
}

#[tokio::test]
async fn 刷新调用取消后释放等待者但保留待决状态() {
    let vault = Arc::new(内存凭据库::new(stored_credentials(time(1_050))));
    let control_plane = Arc::new(测试控制平面::paused(刷新结果::成功));
    let service = service(
        vault.clone(),
        control_plane.clone(),
        Arc::new(Mutex::new(Vec::new())),
    );
    let mut refreshing = Box::pin(service.active_session());
    let mut waiting = Box::pin(service.active_session());
    assert_pending(refreshing.as_mut());
    assert_pending(waiting.as_mut());

    drop(refreshing);
    assert_eq!(
        waiting.await.expect_err("取消后刷新结果仍未知").kind(),
        BridgeSessionFailureKind::RefreshOutcomeUnknown
    );
    assert_eq!(control_plane.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        vault.load().expect("凭据可读").expect("保留待决凭据").state,
        pending(1)
    );
}

#[tokio::test]
async fn 刷新结果未知后以同一尝试号重试并恢复就绪() {
    let vault = Arc::new(内存凭据库::new(stored_credentials(time(1_050))));
    let control_plane = Arc::new(测试控制平面::sequence(
        [刷新结果::结果未知, 刷新结果::成功],
    ));
    let service = service(
        vault.clone(),
        control_plane.clone(),
        Arc::new(Mutex::new(Vec::new())),
    );

    let unknown = service
        .active_session()
        .await
        .expect_err("未知提交不能继续使用旧会话");
    assert_eq!(
        unknown.kind(),
        BridgeSessionFailureKind::RefreshOutcomeUnknown
    );
    assert_eq!(
        vault.load().expect("凭据可读").expect("保留待决凭据").state,
        pending(1)
    );

    let recovered = service
        .active_session()
        .await
        .expect("同一尝试号的重试应取得新会话");

    assert_eq!(recovered.access_token.expose(), "new-access-token");
    let requests = control_plane.requests.lock().expect("刷新请求锁可用");
    assert_eq!(
        requests
            .iter()
            .map(|request| (request.attempt_id, request.refresh_token.expose()))
            .collect::<Vec<_>>(),
        [
            (attempt(1), "old-refresh-token"),
            (attempt(1), "old-refresh-token")
        ],
        "重试必须沿用同一尝试号与旧刷新令牌，服务端才能重放而不是判为重用"
    );
    let stored = vault.load().expect("凭据可读").expect("新凭据已保存");
    assert_eq!(stored.state, BridgeCredentialState::Ready);
    assert_eq!(stored.refresh_token.expose(), "new-refresh-token");
    assert_eq!(
        vault.writes.lock().expect("写入记录锁可用").as_slice(),
        [pending(1), BridgeCredentialState::Ready],
        "重试沿用已持久化的尝试号，不再重写待决状态"
    );
}

#[tokio::test]
async fn 重启后用持久化的尝试号对账而不是使用旧访问令牌() {
    let mut credentials = stored_credentials(time(10_000));
    credentials.state = pending(7);
    let vault = Arc::new(内存凭据库::new(credentials));
    let control_plane = Arc::new(测试控制平面::new(刷新结果::成功));
    let service = service(
        vault.clone(),
        control_plane.clone(),
        Arc::new(Mutex::new(Vec::new())),
    );

    let session = service
        .active_session()
        .await
        .expect("对账成功后应取得新会话");

    assert_eq!(session.access_token.expose(), "new-access-token");
    assert_eq!(control_plane.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        control_plane.requests.lock().expect("刷新请求锁可用")[0].attempt_id,
        attempt(7)
    );
    assert_eq!(
        vault.load().expect("凭据可读").expect("新凭据已保存").state,
        BridgeCredentialState::Ready
    );
}

#[tokio::test]
async fn 旧版本留下的无尝试号待决状态先持久化新尝试号再对账() {
    let mut credentials = stored_credentials(time(10_000));
    credentials.state = BridgeCredentialState::RefreshPending { attempt_id: None };
    let vault = Arc::new(内存凭据库::new(credentials));
    let control_plane = Arc::new(测试控制平面::new(刷新结果::成功));
    let service = service(
        vault.clone(),
        control_plane.clone(),
        Arc::new(Mutex::new(Vec::new())),
    );

    service
        .active_session()
        .await
        .expect("旧令牌未被消费时对账成功");

    assert_eq!(
        control_plane.requests.lock().expect("刷新请求锁可用")[0].attempt_id,
        attempt(1)
    );
    assert_eq!(
        vault.writes.lock().expect("写入记录锁可用").as_slice(),
        [pending(1), BridgeCredentialState::Ready],
        "发请求前必须先记下尝试号，结果未知时才能用它重试"
    );
}

#[tokio::test]
async fn 服务端确认旧令牌不可用时清除本机凭据以便重新授权() {
    let mut credentials = stored_credentials(time(10_000));
    credentials.state = pending(3);
    let vault = Arc::new(内存凭据库::new(credentials));
    let control_plane = Arc::new(测试控制平面::new(刷新结果::认证拒绝));
    let service = service(
        vault.clone(),
        control_plane.clone(),
        Arc::new(Mutex::new(Vec::new())),
    );

    let failure = service.active_session().await.expect_err("旧令牌已不可用");

    assert_eq!(failure.kind(), BridgeSessionFailureKind::NotAuthorized);
    assert!(
        vault.load().expect("凭据可读").is_none(),
        "只有清除本机凭据，启动时才会进入设备授权"
    );
}

#[tokio::test]
async fn 用户要求重新授权时清除本机凭据且不联系控制面() {
    let mut credentials = stored_credentials(time(10_000));
    credentials.state = pending(5);
    let vault = Arc::new(内存凭据库::new(credentials));
    let control_plane = Arc::new(测试控制平面::new(刷新结果::成功));
    let service = service(
        vault.clone(),
        control_plane.clone(),
        Arc::new(Mutex::new(Vec::new())),
    );

    service
        .forget_device_session()
        .await
        .expect("本机凭据可清除");

    assert!(vault.load().expect("凭据可读").is_none());
    assert_eq!(
        service
            .active_session()
            .await
            .expect_err("清除后必须重新授权")
            .kind(),
        BridgeSessionFailureKind::NotAuthorized
    );
    assert_eq!(control_plane.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn 待决期间刷新令牌已过期时不再对账而是清除凭据() {
    let mut credentials = stored_credentials(time(10_000));
    credentials.state = pending(4);
    credentials.refresh_token_expires_at = time(1_000);
    let vault = Arc::new(内存凭据库::new(credentials));
    let control_plane = Arc::new(测试控制平面::new(刷新结果::成功));
    let service = service(
        vault.clone(),
        control_plane.clone(),
        Arc::new(Mutex::new(Vec::new())),
    );

    let failure = service
        .active_session()
        .await
        .expect_err("过期令牌无法对账");

    assert_eq!(failure.kind(), BridgeSessionFailureKind::NotAuthorized);
    assert_eq!(control_plane.calls.load(Ordering::SeqCst), 0);
    assert!(vault.load().expect("凭据可读").is_none());
}

#[tokio::test]
async fn 网络暂时不可用时保留同一尝试号并在恢复后重连() {
    let vault = Arc::new(内存凭据库::new(stored_credentials(time(1_050))));
    let control_plane = Arc::new(测试控制平面::sequence([
        刷新结果::依赖不可用,
        刷新结果::成功,
    ]));
    let service = service(
        vault.clone(),
        control_plane.clone(),
        Arc::new(Mutex::new(Vec::new())),
    );

    let failure = service
        .active_session()
        .await
        .expect_err("网络不可用时本次刷新应明确失败");
    assert_eq!(
        failure.kind(),
        BridgeSessionFailureKind::ControlPlaneUnavailable
    );
    assert_eq!(
        vault.load().expect("凭据可读").expect("旧凭据应保留").state,
        pending(1),
        "无法确认请求是否送达，重连时必须沿用同一尝试号"
    );

    let recovered = service.active_session().await.expect("网络恢复后应可重连");
    assert_eq!(recovered.access_token.expose(), "new-access-token");
    assert_eq!(control_plane.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        control_plane.requests.lock().expect("刷新请求锁可用")[1].attempt_id,
        attempt(1)
    );
}

fn assert_pending<F: Future>(future: Pin<&mut F>) {
    // 逐个推进到刷新或等待点，确保断言不依赖线程调度或定时休眠。
    assert!(
        future
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending(),
        "刷新完成前并发请求应保持等待"
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
            refresh_attempts: Arc::new(测试尝试号工厂::default()),
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

fn attempt(sequence: usize) -> DeviceRefreshAttemptId {
    let sequence = u128::try_from(sequence).expect("测试尝试序号可转换");
    DeviceRefreshAttemptId::from_uuid(Uuid::from_u128(
        0x0198_b601_77a1_7000_8000_0000_0000_0000 | sequence,
    ))
}

fn pending(sequence: usize) -> BridgeCredentialState {
    BridgeCredentialState::RefreshPending {
        attempt_id: Some(attempt(sequence)),
    }
}

fn device_id() -> DeviceId {
    DeviceId::from_uuid(Uuid::from_u128(2))
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
