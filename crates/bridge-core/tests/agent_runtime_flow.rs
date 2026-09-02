use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use agent_room_application::ports::{
    DeviceSignature, MatrixDeviceId, MatrixSession, MatrixSessionMetadata, MatrixUserId,
    PortFuture, SecretValue,
};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    agent_runtime::{
        AgentRuntimeCredentialVault, AgentRuntimeRegistrationIntent, AgentRuntimeRequestIdFactory,
        AgentRuntimeSessionConfig, AgentRuntimeSessionDependencies, AgentRuntimeSessionFailureKind,
        AgentRuntimeSessionService, ControlPlaneAgentRuntimeFailure,
        ControlPlaneAgentRuntimeFailureKind, ControlPlaneAgentRuntimeGateway,
        ControlPlaneAgentRuntimeResult, RegisteredAgentRuntime, StoredAgentRuntimeCredentials,
    },
    ports::{BridgeCredentialResult, DeviceSigningIdentity, DeviceSigningIdentityStore},
};
use agent_room_domain::{
    devices::DevicePublicSigningKey,
    ids::{AdapterBindingId, AgentId, AgentInstanceId, AgentInstanceRegistrationRequestId},
};
use uuid::Uuid;

const AGENT_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
const OTHER_AGENT_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e45";
const INSTANCE_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e47";
const BINDING_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e48";
const REQUEST_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e49";

struct 固定签名身份;

impl DeviceSigningIdentity for 固定签名身份 {
    fn public_key(&self) -> BridgeCredentialResult<DevicePublicSigningKey> {
        Ok(DevicePublicSigningKey::new(vec![7; 32]).expect("测试公钥有效"))
    }

    fn sign(&self, _message: &[u8]) -> BridgeCredentialResult<DeviceSignature> {
        Ok(DeviceSignature::new(vec![9; 64]).expect("测试签名有效"))
    }
}

struct 固定签名身份库;

impl DeviceSigningIdentityStore for 固定签名身份库 {
    fn load_or_create(&self) -> BridgeCredentialResult<Arc<dyn DeviceSigningIdentity>> {
        Ok(Arc::new(固定签名身份))
    }
}

#[derive(Default)]
struct 内存运行凭据库 {
    current: Mutex<Option<StoredAgentRuntimeCredentials>>,
    writes: Mutex<Vec<StoredAgentRuntimeCredentials>>,
}

impl AgentRuntimeCredentialVault for 内存运行凭据库 {
    fn load(&self) -> BridgeCredentialResult<Option<StoredAgentRuntimeCredentials>> {
        Ok(self.current.lock().expect("凭据锁可用").clone())
    }

    fn replace(&self, credentials: &StoredAgentRuntimeCredentials) -> BridgeCredentialResult<()> {
        self.writes
            .lock()
            .expect("写入记录锁可用")
            .push(credentials.clone());
        *self.current.lock().expect("凭据锁可用") = Some(credentials.clone());
        Ok(())
    }

    fn clear(&self) -> BridgeCredentialResult<()> {
        *self.current.lock().expect("凭据锁可用") = None;
        Ok(())
    }
}

struct 固定请求标识;

impl AgentRuntimeRequestIdFactory for 固定请求标识 {
    fn registration_request_id(&self) -> AgentInstanceRegistrationRequestId {
        AgentInstanceRegistrationRequestId::from_uuid(uuid(REQUEST_ID))
    }
}

struct 队列控制面 {
    responses: Mutex<VecDeque<ControlPlaneAgentRuntimeResult<RegisteredAgentRuntime>>>,
    requests: Mutex<Vec<AgentRuntimeRegistrationIntent>>,
}

impl 队列控制面 {
    fn new(
        responses: impl IntoIterator<Item = ControlPlaneAgentRuntimeResult<RegisteredAgentRuntime>>,
    ) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
            requests: Mutex::new(Vec::new()),
        }
    }
}

impl ControlPlaneAgentRuntimeGateway for 队列控制面 {
    fn register<'a>(
        &'a self,
        intent: &'a AgentRuntimeRegistrationIntent,
    ) -> PortFuture<'a, ControlPlaneAgentRuntimeResult<RegisteredAgentRuntime>> {
        let response = self
            .responses
            .lock()
            .expect("响应队列锁可用")
            .pop_front()
            .expect("测试已配置控制面响应");
        self.requests
            .lock()
            .expect("请求记录锁可用")
            .push(intent.clone());
        Box::pin(async move { response })
    }
}

#[tokio::test]
async fn 首次登记先持久化意图且成功后直接恢复() {
    let credentials = Arc::new(内存运行凭据库::default());
    let gateway = Arc::new(队列控制面::new([Ok(runtime(agent_id()))]));
    let service = service(credentials.clone(), gateway.clone());

    let first = service
        .ensure_session(&config(agent_id(), "codex-desktop"))
        .await
        .expect("首次登记成功");
    let second = service
        .ensure_session(&config(agent_id(), "codex-desktop"))
        .await
        .expect("第二次直接恢复");

    assert_eq!(first, second);
    assert_eq!(gateway.requests.lock().expect("请求记录锁可用").len(), 1);
    let writes = credentials.writes.lock().expect("写入记录锁可用");
    assert!(matches!(
        writes.first(),
        Some(StoredAgentRuntimeCredentials::RegistrationPending(_))
    ));
    assert!(matches!(
        writes.last(),
        Some(StoredAgentRuntimeCredentials::Ready { .. })
    ));
}

#[tokio::test]
async fn 未知提交保留同一请求标识供下次安全重试() {
    let credentials = Arc::new(内存运行凭据库::default());
    let gateway = Arc::new(队列控制面::new([
        Err(ControlPlaneAgentRuntimeFailure::new(
            ControlPlaneAgentRuntimeFailureKind::UnknownCommit,
        )),
        Ok(runtime(agent_id())),
    ]));
    let service = service(credentials.clone(), gateway.clone());

    let failure = service
        .ensure_session(&config(agent_id(), "codex-desktop"))
        .await
        .expect_err("未知提交不能伪装成功");
    assert_eq!(
        failure.kind(),
        AgentRuntimeSessionFailureKind::RegistrationOutcomeUnknown
    );
    assert!(matches!(
        credentials.current.lock().expect("凭据锁可用").as_ref(),
        Some(StoredAgentRuntimeCredentials::RegistrationPending(_))
    ));

    service
        .ensure_session(&config(agent_id(), "codex-desktop"))
        .await
        .expect("相同意图可安全重试");
    let requests = gateway.requests.lock().expect("请求记录锁可用");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].request_id(), requests[1].request_id());
}

#[tokio::test]
async fn 已持久化意图拒绝静默切换_agent_或适配器() {
    let credentials = Arc::new(内存运行凭据库::default());
    let gateway = Arc::new(队列控制面::new([Ok(runtime(agent_id()))]));
    let service = service(credentials, gateway);
    service
        .ensure_session(&config(agent_id(), "codex-desktop"))
        .await
        .expect("首次登记成功");

    for changed in [
        config(other_agent_id(), "codex-desktop"),
        config(agent_id(), "other-adapter"),
    ] {
        let failure = service
            .ensure_session(&changed)
            .await
            .expect_err("配置漂移必须显式失败");
        assert_eq!(
            failure.kind(),
            AgentRuntimeSessionFailureKind::ConfigurationConflict
        );
    }
}

#[tokio::test]
async fn 控制面返回其他_agent_身份时不写入就绪凭据() {
    let credentials = Arc::new(内存运行凭据库::default());
    let gateway = Arc::new(队列控制面::new([Ok(runtime(other_agent_id()))]));
    let service = service(credentials.clone(), gateway);

    let failure = service
        .ensure_session(&config(agent_id(), "codex-desktop"))
        .await
        .expect_err("身份错配必须失败");

    assert_eq!(
        failure.kind(),
        AgentRuntimeSessionFailureKind::InvalidControlPlaneResponse
    );
    assert!(matches!(
        credentials.current.lock().expect("凭据锁可用").as_ref(),
        Some(StoredAgentRuntimeCredentials::RegistrationPending(_))
    ));
}

#[tokio::test]
async fn 加密身份恢复只替换同一实例的_matrix_凭据() {
    let credentials = Arc::new(内存运行凭据库::default());
    let initial = runtime_with("matrix-access-token", INSTANCE_ID);
    let recovered = runtime_with("rotated-matrix-access-token", INSTANCE_ID);
    let gateway = Arc::new(队列控制面::new([
        Ok(initial.clone()),
        Ok(recovered.clone()),
    ]));
    let service = service(credentials.clone(), gateway.clone());
    let config = config(agent_id(), "codex-desktop");

    service.ensure_session(&config).await.expect("首次登记成功");
    let result = service
        .recover_matrix_session(&config)
        .await
        .expect("同一身份的会话可以轮换");

    assert_eq!(result, recovered);
    assert_ne!(result.matrix_session(), initial.matrix_session());
    let requests = gateway.requests.lock().expect("请求记录锁可用");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].request_id(), requests[1].request_id());
    assert!(matches!(
        credentials.current.lock().expect("凭据锁可用").as_ref(),
        Some(StoredAgentRuntimeCredentials::Ready { runtime, .. })
            if runtime.as_ref() == &recovered
    ));
}

#[tokio::test]
async fn 加密身份恢复拒绝控制面静默切换实例() {
    let credentials = Arc::new(内存运行凭据库::default());
    let gateway = Arc::new(队列控制面::new([
        Ok(runtime_with("matrix-access-token", INSTANCE_ID)),
        Ok(runtime_with(
            "rotated-matrix-access-token",
            "0198b601-77a1-7bb8-83eb-a8fe68c97e50",
        )),
    ]));
    let service = service(credentials.clone(), gateway);
    let config = config(agent_id(), "codex-desktop");
    let initial = service.ensure_session(&config).await.expect("首次登记成功");

    let failure = service
        .recover_matrix_session(&config)
        .await
        .expect_err("实例漂移必须被拒绝");

    assert_eq!(
        failure.kind(),
        AgentRuntimeSessionFailureKind::InvalidControlPlaneResponse
    );
    assert!(matches!(
        credentials.current.lock().expect("凭据锁可用").as_ref(),
        Some(StoredAgentRuntimeCredentials::Ready { runtime, .. })
            if runtime.as_ref() == &initial
    ));
}

fn service(
    credentials: Arc<内存运行凭据库>,
    control_plane: Arc<队列控制面>,
) -> AgentRuntimeSessionService {
    AgentRuntimeSessionService::new(AgentRuntimeSessionDependencies {
        signing_identities: Arc::new(固定签名身份库),
        control_plane,
        credentials,
        identifiers: Arc::new(固定请求标识),
    })
}

fn config(agent_id: AgentId, adapter_type: &str) -> AgentRuntimeSessionConfig {
    AgentRuntimeSessionConfig::new(agent_id, adapter_type, "2026-08-24")
        .expect("测试运行时配置有效")
}

fn runtime(agent_id: AgentId) -> RegisteredAgentRuntime {
    runtime_with_agent(agent_id, "matrix-access-token", INSTANCE_ID)
}

fn runtime_with(access_token: &str, instance_id: &str) -> RegisteredAgentRuntime {
    runtime_with_agent(agent_id(), access_token, instance_id)
}

fn runtime_with_agent(
    agent_id: AgentId,
    access_token: &str,
    instance_id: &str,
) -> RegisteredAgentRuntime {
    let user_id =
        MatrixUserId::new(format!("@agent_{agent_id}:example.org")).expect("用户标识有效");
    let identity = BridgeAgentIdentity::new(
        agent_id,
        "Codex Builder",
        user_id.as_str(),
        AgentInstanceId::from_uuid(uuid(instance_id)),
    )
    .expect("Agent 身份有效");
    RegisteredAgentRuntime::new(
        identity,
        AdapterBindingId::from_uuid(uuid(BINDING_ID)),
        MatrixSession::new(
            MatrixSessionMetadata::new(
                user_id,
                MatrixDeviceId::new("AR_TEST").expect("Matrix 设备标识有效"),
            ),
            SecretValue::new(access_token).expect("访问令牌有效"),
            Some(SecretValue::new("matrix-refresh-token").expect("刷新令牌有效")),
        ),
    )
    .expect("测试运行时有效")
}

fn agent_id() -> AgentId {
    AgentId::from_uuid(uuid(AGENT_ID))
}

fn other_agent_id() -> AgentId {
    AgentId::from_uuid(uuid(OTHER_AGENT_ID))
}

fn uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).expect("测试 UUID 有效")
}
