use std::{env, future::pending, num::NonZeroU16, time::Duration};

use agent_room_application::ports::{
    MatrixAcceptedEvent, MatrixAgentDeviceSessionRequest, MatrixAgentIdentityProvisioner,
    MatrixAgentLocalpart, MatrixAgentUserRegistration, MatrixClientFactory, MatrixConnection,
    MatrixCreateRoom, MatrixDeviceId, MatrixEvent, MatrixEventId, MatrixEventType, MatrixFailure,
    MatrixFailureKind, MatrixGateway, MatrixLogin, MatrixReceipt, MatrixReceiptKind,
    MatrixRecoveryAction, MatrixRetryPolicy, MatrixRoomAliasLocalpart, MatrixRoomId,
    MatrixRoomKind, MatrixRoomPreset, MatrixRoomSync, MatrixRoomSyncKind, MatrixRoomVisibility,
    MatrixStateEvent, MatrixStateKey, MatrixSyncBatch, MatrixSyncRequest, MatrixSyncToken,
    MatrixTransactionId, MatrixUserId, RoomProvisioningGateway, SecretValue,
};
use agent_room_domain::{ids::AgentId, time::DurationMillis};
use agent_room_matrix_adapter::{
    MatrixRoomProvisioningAdapter, MatrixSdkClientFactory, MatrixSdkConfiguration,
};
use agent_room_matrix_provisioning_adapter::{
    MatrixApplicationServiceConfiguration, MatrixApplicationServiceProvisioner,
};
use serde_json::json;
use tokio::{net::TcpListener, task::JoinHandle, time::sleep, time::timeout};
use uuid::Uuid;

const TEST_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const TEST_SYNC_TIMEOUT_MILLIS: u64 = 100;

#[tokio::test]
async fn 网络断线和超时被映射为不同的可恢复错误() {
    let closing = ClosingServer::start().await;
    let disconnected = factory(closing.url(), Duration::from_millis(200), 3);
    let failure = timeout(
        Duration::from_secs(2),
        disconnected.login(&login("nobody", "not-a-secret")),
    )
    .await
    .expect("断线探测必须在外层预算内结束")
    .expect_err("未监听端口必须拒绝连接");
    assert_eq!(failure.kind(), MatrixFailureKind::DependencyUnavailable);

    let hanging = HangingServer::start().await;
    let stalled = factory(hanging.url(), Duration::from_millis(100), 3);
    let failure = timeout(
        Duration::from_secs(2),
        stalled.login(&login("nobody", "not-a-secret")),
    )
    .await
    .expect("适配器超时必须先于外层预算")
    .expect_err("无响应端点必须触发超时");
    assert_eq!(failure.kind(), MatrixFailureKind::Timeout);
}

#[tokio::test]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse 测试账号"]
async fn 真实_synapse_支持房间生命周期_幂等发送_回执和回填() {
    let base_url = required_environment("AGENT_ROOM_MATRIX_TEST_BASE_URL");
    let developer_user = required_environment("AGENT_ROOM_MATRIX_TEST_ADMIN_USER");
    let developer_password = required_environment("AGENT_ROOM_MATRIX_TEST_ADMIN_PASSWORD");
    let agent_user = required_environment("AGENT_ROOM_MATRIX_TEST_AGENT_USER");
    let agent_password = required_environment("AGENT_ROOM_MATRIX_TEST_AGENT_PASSWORD");
    let factory = factory(&base_url, TEST_REQUEST_TIMEOUT, 3);
    let scenario = prepare_room(
        &factory,
        &developer_user,
        &developer_password,
        &agent_user,
        &agent_password,
    )
    .await;
    let message_flow = verify_message_flow(&scenario).await;
    verify_receipt_and_leave(&scenario, message_flow).await;
    verify_space_alias(&factory, &scenario.developer).await;
}

#[tokio::test]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse Application Service 配置"]
async fn 真实_synapse_可幂等建立独立_agent_用户和设备会话() {
    let base_url = required_environment("AGENT_ROOM_MATRIX_TEST_BASE_URL");
    let application_service_token = required_environment("AGENT_ROOM_MATRIX_TEST_APPSERVICE_TOKEN");
    let configuration = MatrixApplicationServiceConfiguration::new(
        &base_url,
        "matrix.agent-room.localhost",
        SecretValue::new(application_service_token).expect("Application Service Token 有效"),
        TEST_REQUEST_TIMEOUT,
    )
    .expect("Application Service 配置有效");
    let provisioner =
        MatrixApplicationServiceProvisioner::new(configuration).expect("适配器可初始化");
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    let registration =
        MatrixAgentUserRegistration::new(MatrixAgentLocalpart::from_agent_id(agent_id));

    let first = provisioner
        .ensure_user(&registration)
        .await
        .expect("首次创建 Agent 用户成功");
    let repeated = provisioner
        .ensure_user(&registration)
        .await
        .expect("重复创建对账为同一用户");
    assert_eq!(first, repeated);
    let device_id =
        MatrixDeviceId::new(format!("AR_{}", Uuid::now_v7().simple())).expect("设备标识有效");
    let request = MatrixAgentDeviceSessionRequest::new(
        first.clone(),
        device_id.clone(),
        "Agent Room 真实验收实例".to_owned(),
    )
    .expect("设备会话请求有效");
    let session = provisioner
        .issue_device_session(&request)
        .await
        .expect("可签发 Agent 设备会话");
    assert_eq!(session.metadata().user_id(), &first);
    assert_eq!(session.metadata().device_id(), &device_id);

    let connection = factory(&base_url, TEST_REQUEST_TIMEOUT, 3)
        .restore(&session)
        .await
        .expect("新会话必须能由 Matrix SDK 恢复");
    let batch = sync(connection.gateway(), None).await;
    assert!(!batch.next_batch().as_str().is_empty());
}

struct RoomScenario {
    developer: MatrixConnection,
    agent: MatrixConnection,
    room_id: MatrixRoomId,
    developer_baseline: MatrixSyncToken,
    agent_baseline: MatrixSyncToken,
}

struct MessageFlowResult {
    event_id: MatrixEventId,
    agent_next_batch: MatrixSyncToken,
}

async fn prepare_room(
    factory: &MatrixSdkClientFactory,
    developer_user: &str,
    developer_password: &str,
    agent_user: &str,
    agent_password: &str,
) -> RoomScenario {
    let developer = factory
        .login(&login(developer_user, developer_password))
        .await
        .expect("开发者必须能通过标准登录接口认证");
    assert_eq!(
        developer.session().metadata().user_id().as_str(),
        developer_user
    );
    let developer_session = developer.session().clone();
    let developer = factory
        .restore(&developer_session)
        .await
        .expect("访问令牌必须能恢复同一 Matrix 设备会话");
    let agent = factory
        .login(&login(agent_user, agent_password))
        .await
        .expect("Agent 必须能通过标准登录接口认证");
    let agent_user_id = MatrixUserId::new(agent_user).expect("种子 Agent 用户 ID 有效");

    let room_id = create_room_with_retry(developer.gateway(), &room_request()).await;
    let denied = agent
        .gateway()
        .join(&room_id)
        .await
        .expect_err("未受邀用户不得加入私有房间");
    assert_eq!(denied.kind(), MatrixFailureKind::Forbidden);

    invite_with_retry(developer.gateway(), &room_id, &agent_user_id).await;
    let invitation = sync(agent.gateway(), None).await;
    assert_room_kind(&invitation, &room_id, MatrixRoomSyncKind::Invited);

    agent
        .gateway()
        .join(&room_id)
        .await
        .expect("受邀 Agent 必须能加入房间");
    let agent_joined = sync(agent.gateway(), Some(invitation.next_batch().clone())).await;
    assert_room_kind(&agent_joined, &room_id, MatrixRoomSyncKind::Joined);
    let agent_baseline = agent_joined.next_batch().clone();

    let developer_joined = sync(developer.gateway(), None).await;
    assert_room_kind(&developer_joined, &room_id, MatrixRoomSyncKind::Joined);
    let developer_baseline = developer_joined.next_batch().clone();

    RoomScenario {
        developer,
        agent,
        room_id,
        developer_baseline,
        agent_baseline,
    }
}

async fn verify_space_alias(factory: &MatrixSdkClientFactory, connection: &MatrixConnection) {
    let gateway = connection.gateway();
    let alias =
        MatrixRoomAliasLocalpart::new(format!("agent-room-space-test-{}", Uuid::now_v7().simple()))
            .expect("测试 Space 别名有效");
    let request = MatrixCreateRoom::new(
        Some("Agent Room Space 验收".to_owned()),
        Some("仅用于验证 Matrix Space 与确定性别名".to_owned()),
        MatrixRoomVisibility::Private,
        MatrixRoomPreset::PrivateChat,
        false,
        Vec::new(),
    )
    .expect("Space 创建请求有效")
    .with_kind(MatrixRoomKind::Space)
    .with_alias_localpart(alias.clone());
    let room_id = create_room_with_retry(gateway, &request).await;
    let resolved = gateway
        .resolve_room_alias(&alias)
        .await
        .expect("确定性别名必须能解析回 Space");
    assert_eq!(resolved, room_id);

    let child_alias =
        MatrixRoomAliasLocalpart::new(format!("agent-room-child-test-{}", Uuid::now_v7().simple()))
            .expect("测试子房间别名有效");
    let child_request = MatrixCreateRoom::new(
        Some("Agent Room Space 子房间验收".to_owned()),
        Some("仅用于验证 m.space.child 状态".to_owned()),
        MatrixRoomVisibility::Private,
        MatrixRoomPreset::PrivateChat,
        false,
        Vec::new(),
    )
    .expect("子房间创建请求有效")
    .with_alias_localpart(child_alias);
    let child_id = create_room_with_retry(gateway, &child_request).await;
    let provisioning = MatrixRoomProvisioningAdapter::new(connection.gateway_handle());
    attach_with_retry(&provisioning, &room_id, &child_id).await;

    let observer = factory
        .restore(connection.session())
        .await
        .expect("独立验收会话必须能从头同步 Space 状态");
    let space = sync_until_room(observer.gateway(), &room_id, MatrixRoomSyncKind::Joined).await;
    let creation = space
        .state()
        .iter()
        .find(|event| event.event_type().as_str() == "m.room.create")
        .expect("初次同步必须包含 Space 创建状态");
    assert_eq!(creation.content()["type"], "m.space");
    let child = space
        .state()
        .iter()
        .chain(space.timeline().iter())
        .find(|event| {
            event.event_type().as_str() == "m.space.child"
                && event.state_key() == Some(child_id.as_str())
        })
        .expect("Space 初次同步必须包含子房间状态");
    assert_eq!(child.content()["suggested"], true);
    assert_eq!(
        child.content()["via"],
        json!(["matrix.agent-room.localhost"])
    );
    leave_with_retry(gateway, &child_id).await;
    leave_with_retry(gateway, &room_id).await;
}

async fn sync_until_room(
    gateway: &dyn MatrixGateway,
    room_id: &MatrixRoomId,
    kind: MatrixRoomSyncKind,
) -> MatrixRoomSync {
    let mut since = None;
    let mut available = Vec::new();
    for _ in 0..10 {
        let batch = sync(gateway, since).await;
        available.extend(
            batch
                .rooms()
                .iter()
                .map(|room| format!("{}:{:?}", room.room_id().as_str(), room.kind())),
        );
        if let Some(room) = batch
            .rooms()
            .iter()
            .find(|room| room.room_id() == room_id && room.kind() == kind)
        {
            return room.clone();
        }
        since = Some(batch.next_batch().clone());
    }
    panic!(
        "同步结果始终缺少房间 {} 的 {kind:?} 更新；实际房间：{available:?}",
        room_id.as_str()
    )
}

async fn verify_message_flow(scenario: &RoomScenario) -> MessageFlowResult {
    for index in 0..5 {
        let body = format!("历史消息 {index}");
        let event = message_event(unique_value("history"), &body);
        send_with_retry(scenario.developer.gateway(), &scenario.room_id, &event).await;
    }

    let duplicate = message_event(unique_value("stable"), "幂等消息");
    let first = send_with_retry(scenario.developer.gateway(), &scenario.room_id, &duplicate).await;
    let repeated =
        send_with_retry(scenario.developer.gateway(), &scenario.room_id, &duplicate).await;
    assert_eq!(first.event_id(), repeated.event_id());

    let developer_delta = sync(
        scenario.developer.gateway(),
        Some(scenario.developer_baseline.clone()),
    )
    .await;
    let reconciled = developer_delta
        .reconcile_transaction(duplicate.transaction_id())
        .expect("事务映射不得一对多")
        .expect("发送设备同步结果必须包含事务标识");
    assert_eq!(&reconciled, first.event_id());

    let initial_status = status_state_event(1);
    send_state_with_retry(
        scenario.developer.gateway(),
        &scenario.room_id,
        &initial_status,
    )
    .await;
    let current_status = status_state_event(2);
    send_state_with_retry(
        scenario.developer.gateway(),
        &scenario.room_id,
        &current_status,
    )
    .await;

    let agent_delta = sync(
        scenario.agent.gateway(),
        Some(scenario.agent_baseline.clone()),
    )
    .await;
    let room_delta = room_update(&agent_delta, &scenario.room_id, MatrixRoomSyncKind::Joined);
    let status = room_delta
        .timeline()
        .iter()
        .rev()
        .find(|event| {
            event.event_type().as_str() == "org.agentroom.agent.status.v1"
                && event.state_key() == Some("instance-test")
        })
        .or_else(|| {
            room_delta.state().iter().find(|event| {
                event.event_type().as_str() == "org.agentroom.agent.status.v1"
                    && event.state_key() == Some("instance-test")
            })
        })
        .expect("增量同步必须包含最新状态事件");
    assert_eq!(status.content()["revision"], 2);
    assert!(room_delta.timeline_limited());
    let previous_batch = room_delta
        .previous_batch()
        .cloned()
        .expect("受限时间线必须携带回填游标");
    let backfill = scenario
        .agent
        .gateway()
        .backfill(
            &scenario.room_id,
            &agent_room_application::ports::MatrixBackfillRequest::new(
                previous_batch,
                NonZeroU16::new(20).expect("测试页大小非零"),
            )
            .expect("回填请求有效"),
        )
        .await
        .expect("历史消息必须能通过标准 messages 接口回填");
    assert!(!backfill.events().is_empty());

    MessageFlowResult {
        event_id: first.event_id().clone(),
        agent_next_batch: agent_delta.next_batch().clone(),
    }
}

async fn verify_receipt_and_leave(scenario: &RoomScenario, message_flow: MessageFlowResult) {
    scenario
        .agent
        .gateway()
        .send_receipt(
            &scenario.room_id,
            &MatrixReceipt::new(MatrixReceiptKind::Read, message_flow.event_id),
        )
        .await
        .expect("已加入用户必须能发送阅读回执");

    leave_with_retry(scenario.agent.gateway(), &scenario.room_id).await;
    let agent_left = sync(
        scenario.agent.gateway(),
        Some(message_flow.agent_next_batch),
    )
    .await;
    assert_room_kind(&agent_left, &scenario.room_id, MatrixRoomSyncKind::Left);
    leave_with_retry(scenario.developer.gateway(), &scenario.room_id).await;
}

fn factory(
    base_url: &str,
    request_timeout: Duration,
    timeline_limit: u16,
) -> MatrixSdkClientFactory {
    let configuration = MatrixSdkConfiguration::new(base_url, request_timeout)
        .expect("测试 Homeserver 配置有效")
        .with_sync_timeline_limit(NonZeroU16::new(timeline_limit).expect("时间线上限非零"))
        .expect("时间线上限有效");
    MatrixSdkClientFactory::new(configuration)
}

fn login(user: &str, password: &str) -> MatrixLogin {
    MatrixLogin::new(
        user,
        SecretValue::new(password).expect("测试密码有效"),
        None,
        Some("Agent Room Matrix 验收".to_owned()),
    )
    .expect("测试登录请求有效")
}

fn room_request() -> MatrixCreateRoom {
    MatrixCreateRoom::new(
        Some(format!("Task 11 验收 {}", Uuid::now_v7().simple())),
        Some("仅用于自动化 Matrix 适配器验收".to_owned()),
        MatrixRoomVisibility::Private,
        MatrixRoomPreset::PrivateChat,
        false,
        Vec::new(),
    )
    .expect("测试房间请求有效")
}

fn message_event(transaction_id: String, body: &str) -> MatrixEvent {
    MatrixEvent::new(
        MatrixEventType::new("org.agentroom.message.preview.v1").expect("事件类型有效"),
        MatrixTransactionId::new(transaction_id).expect("事务标识有效"),
        json!({
            "schemaVersion": "1.0",
            "body": body,
        }),
    )
    .expect("测试消息有效")
}

fn status_state_event(revision: u8) -> MatrixStateEvent {
    MatrixStateEvent::new(
        MatrixEventType::new("org.agentroom.agent.status.v1").expect("事件类型有效"),
        MatrixStateKey::new("instance-test").expect("状态键有效"),
        json!({
            "schemaVersion": "1.0",
            "revision": revision,
        }),
    )
    .expect("测试状态事件有效")
}

async fn send_with_retry(
    gateway: &dyn MatrixGateway,
    room_id: &MatrixRoomId,
    event: &MatrixEvent,
) -> MatrixAcceptedEvent {
    let policy = retry_policy();
    for completed_attempts in 1..=5 {
        match gateway.send_event(room_id, event).await {
            Ok(accepted) => return accepted,
            Err(failure) => wait_for_retry(policy, failure, completed_attempts).await,
        }
    }
    panic!("发送重试次数耗尽")
}

async fn create_room_with_retry(
    gateway: &dyn MatrixGateway,
    request: &MatrixCreateRoom,
) -> MatrixRoomId {
    let policy = retry_policy();
    for completed_attempts in 1..=5 {
        match gateway.create_room(request).await {
            Ok(room_id) => return room_id,
            Err(failure) => wait_for_retry(policy, failure, completed_attempts).await,
        }
    }
    panic!("建房重试次数耗尽")
}

async fn invite_with_retry(
    gateway: &dyn MatrixGateway,
    room_id: &MatrixRoomId,
    user_id: &MatrixUserId,
) {
    let policy = retry_policy();
    for completed_attempts in 1..=5 {
        match gateway.invite(room_id, user_id).await {
            Ok(()) => return,
            Err(failure) => wait_for_retry(policy, failure, completed_attempts).await,
        }
    }
    panic!("邀请重试次数耗尽")
}

async fn attach_with_retry(
    provisioning: &dyn RoomProvisioningGateway,
    space_id: &MatrixRoomId,
    child_id: &MatrixRoomId,
) {
    let policy = retry_policy();
    for completed_attempts in 1..=5 {
        match provisioning.attach_child(space_id, child_id).await {
            Ok(_) => return,
            Err(failure) => wait_for_retry(policy, failure, completed_attempts).await,
        }
    }
    panic!("Space 挂载重试次数耗尽")
}

async fn send_state_with_retry(
    gateway: &dyn MatrixGateway,
    room_id: &MatrixRoomId,
    event: &MatrixStateEvent,
) -> MatrixEventId {
    let policy = retry_policy();
    for completed_attempts in 1..=5 {
        match gateway.send_state_event(room_id, event).await {
            Ok(event_id) => return event_id,
            Err(failure) => wait_for_retry(policy, failure, completed_attempts).await,
        }
    }
    panic!("状态发送重试次数耗尽")
}

async fn leave_with_retry(gateway: &dyn MatrixGateway, room_id: &MatrixRoomId) {
    let policy = retry_policy();
    for completed_attempts in 1..=5 {
        match gateway.leave(room_id).await {
            Ok(()) => return,
            Err(failure) => wait_for_retry(policy, failure, completed_attempts).await,
        }
    }
    panic!("离房重试次数耗尽")
}

fn retry_policy() -> MatrixRetryPolicy {
    MatrixRetryPolicy::new(
        DurationMillis::new(100).expect("初始退避有效"),
        DurationMillis::new(5_000).expect("最大退避有效"),
        5,
    )
    .expect("重试策略有效")
}

async fn wait_for_retry(
    policy: MatrixRetryPolicy,
    failure: MatrixFailure,
    completed_attempts: u16,
) {
    match policy.recovery(failure, completed_attempts) {
        MatrixRecoveryAction::RetryAfter(delay) => {
            sleep(Duration::from_millis(delay.value())).await;
        }
        action => panic!("操作失败且不可安全重试：{failure:?}，恢复动作 {action:?}"),
    }
}

async fn sync(gateway: &dyn MatrixGateway, since: Option<MatrixSyncToken>) -> MatrixSyncBatch {
    let request = MatrixSyncRequest::new(
        since,
        DurationMillis::new(TEST_SYNC_TIMEOUT_MILLIS).expect("同步超时有效"),
        false,
    )
    .expect("同步请求有效");
    gateway
        .sync_once(&request)
        .await
        .expect("真实 Synapse 同步必须成功")
}

fn assert_room_kind(batch: &MatrixSyncBatch, room_id: &MatrixRoomId, kind: MatrixRoomSyncKind) {
    let _ = room_update(batch, room_id, kind);
}

fn room_update<'a>(
    batch: &'a MatrixSyncBatch,
    room_id: &MatrixRoomId,
    kind: MatrixRoomSyncKind,
) -> &'a MatrixRoomSync {
    let available = batch
        .rooms()
        .iter()
        .map(|room| format!("{}:{:?}", room.room_id().as_str(), room.kind()))
        .collect::<Vec<_>>();
    batch
        .rooms()
        .iter()
        .find(|room| room.room_id() == room_id && room.kind() == kind)
        .unwrap_or_else(|| {
            panic!(
                "同步结果缺少房间 {} 的 {kind:?} 更新；实际房间：{available:?}",
                room_id.as_str()
            )
        })
}

fn unique_value(prefix: &str) -> String {
    format!("{prefix}_{}", Uuid::now_v7().simple())
}

fn required_environment(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("缺少真实 Matrix 测试配置 {name}"))
}

struct HangingServer {
    url: String,
    task: JoinHandle<()>,
}

struct ClosingServer(HangingServer);

impl ClosingServer {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("必须能启动受控断线端点");
        let address = listener.local_addr().expect("回环地址有效");
        let task = tokio::spawn(async move {
            if let Ok((socket, _)) = listener.accept().await {
                drop(socket);
            }
        });
        Self(HangingServer {
            url: format!("http://{address}"),
            task,
        })
    }

    fn url(&self) -> &str {
        self.0.url()
    }
}

impl HangingServer {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("必须能启动受控超时端点");
        let address = listener.local_addr().expect("回环地址有效");
        let task = tokio::spawn(async move {
            if let Ok((socket, _)) = listener.accept().await {
                let _socket = socket;
                pending::<()>().await;
            }
        });
        Self {
            url: format!("http://{address}"),
            task,
        }
    }

    fn url(&self) -> &str {
        &self.url
    }
}

impl Drop for HangingServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
