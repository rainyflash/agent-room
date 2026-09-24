//! 进过加密房间的网络 Agent（第 3 步，`specs/network-agents/design.md` 的“加密房间”）：网关为它
//! 打开本机 Bridge 用的同一套 matrix-sdk 客户端，加密存储放在持久卷里，每个 Agent 一个目录。
//!
//! 一台设备只能有一条同步流：轻量客户端不收 to-device 消息，会把发给这台设备的房间密钥跳过去。
//! 所以这类 Agent 的所有房间都改由这里同步；收件箱、确认和预览形状不变，同步位置仍记在收件箱里。
//!
//! matrix-sdk 客户端是有状态的：
//! - 同步放进单独的任务里跑完，长轮询被取代时不会停在处理一半的地方；
//! - 同一个 Agent 同时只有一次同步；从同一位置再同步（上次的结果没进收件箱）时直接给上次的结果，
//!   不让 SDK 把同一段 to-device 消息再处理一遍。
//!
//! 第一次同步后在后台建立加密身份（交叉签名自己的网络设备），并开启服务器端密钥备份：恢复凭据先
//! 封存入库再开启，所以不会出现备份开好了、凭据却没存下的情况。

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex as StdMutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use agent_room_application::{
    network_agents::{NetworkAgentSession, NetworkAgentUseCases},
    ports::{
        MatrixDeviceId, MatrixFailureKind, MatrixGateway, MatrixSession, MatrixSessionMetadata,
        MatrixSyncBatch, MatrixSyncRequest, MatrixSyncToken, MatrixUserId, NetworkAgentSyncRequest,
        PortFuture, SecretFactory,
    },
};
use agent_room_bridge_core::{
    matrix_recovery::{MatrixRecoveryCommand, MatrixRecoverySecret},
    matrix_security::{
        MatrixIdentityState, MatrixSecurityCommand, MatrixSecurityFailure, MatrixSecurityGateway,
        MatrixSecurityResult,
    },
};
use agent_room_domain::{ids::NetworkAgentId, time::DurationMillis};
use agent_room_matrix_adapter::{
    MatrixSdkClientFactory, MatrixSdkConfiguration, MatrixSdkConfigurationError,
    MatrixSdkStoreConfiguration,
};
use tokio::{sync::Mutex, task::JoinHandle, time::Instant};

use super::NetworkGatewayFailure;

/// matrix-sdk 每个请求的期限：盖得住一段 10 秒的长轮询，也给密钥上传与备份留出余量。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(40);
/// 最后一次使用后这么久没人用，就关掉客户端；打开一次要为四个库各做一次 PBKDF2。
const IDLE_LIFETIME: Duration = Duration::from_mins(10);
/// 打不开或设备与存储对不上之后，这么久之内直接回答暂时不可用，不反复打开。
const OPEN_RETRY: Duration = Duration::from_secs(30);
/// 加密身份或密钥备份没建成，隔这么久再试。
const IDENTITY_RETRY: Duration = Duration::from_mins(5);

/// 进过加密房间的网络 Agent 的同步来源；测试里用替身。
pub(crate) trait EncryptedSessions: Send + Sync {
    /// 用这个 Agent 的 matrix-sdk 客户端同步一次。事件已解密、已按信任分类；
    /// 每个房间最多带回的条数是客户端的固定上限，不看请求里的 `timeline_limit`。
    fn sync<'a>(
        &'a self,
        session: &'a NetworkAgentSession,
        request: &'a NetworkAgentSyncRequest,
    ) -> PortFuture<'a, Result<MatrixSyncBatch, NetworkGatewayFailure>>;

    /// 进加密房间之前：打开客户端，从收件箱的位置同步一次（上传设备密钥与一次性密钥；结果留给
    /// 下一次长轮询），再建好加密身份与密钥备份。没建好就报暂时不可用：这时进去，别人发的消息
    /// 它会解不开。
    fn prepare<'a>(
        &'a self,
        session: &'a NetworkAgentSession,
        since: Option<MatrixSyncToken>,
    ) -> PortFuture<'a, Result<(), NetworkGatewayFailure>>;

    /// 停用后关掉它的客户端。
    fn forget(&self, id: NetworkAgentId) -> PortFuture<'_, ()>;

    /// 关掉闲置太久的客户端，返回关了几个。
    fn evict_idle(&self) -> PortFuture<'_, usize>;
}

pub(crate) struct EncryptedClients {
    agents: Arc<dyn NetworkAgentUseCases>,
    secrets: Arc<dyn SecretFactory>,
    configuration: MatrixSdkConfiguration,
    root: PathBuf,
    /// 每个 Agent 一个槽：同一个 Agent 的打开串行，不会有两个客户端同时打开同一个存储。
    slots: Mutex<HashMap<NetworkAgentId, Slot>>,
}

type Slot = Arc<Mutex<SlotState>>;

#[derive(Default)]
enum SlotState {
    #[default]
    Closed,
    Open(Arc<OpenClient>),
    /// 打不开，或设备与存储对不上：到点之前直接回答暂时不可用。
    Failed {
        retry_at: Instant,
    },
}

/// 一次同步的起点与结果。
type ProcessedSync = (Option<MatrixSyncToken>, MatrixSyncBatch);

struct OpenClient {
    id: NetworkAgentId,
    agents: Arc<dyn NetworkAgentUseCases>,
    secrets: Arc<dyn SecretFactory>,
    gateway: Arc<dyn MatrixGateway>,
    security: Arc<dyn MatrixSecurityGateway>,
    /// 上一次同步；拿着这把锁同步，所以同一时刻只有一次。
    last_sync: Mutex<Option<ProcessedSync>>,
    /// 拿着这把锁建身份；里面是没建成时下一次可以再试的时刻。
    identity: Mutex<Option<Instant>>,
    identity_ready: AtomicBool,
    /// 设备与存储对不上（例如存储丢了又新建）：这个客户端不能再用，要换设备重建。
    conflicted: AtomicBool,
    last_used: StdMutex<Instant>,
    /// 排空交接队列：网络 Agent 不处理交接，队列满了会卡住同步。
    drain: JoinHandle<()>,
}

impl Drop for OpenClient {
    fn drop(&mut self) {
        // 排空任务握着客户端；不停掉它，客户端就关不掉。
        self.drain.abort();
    }
}

impl EncryptedClients {
    /// `root` 下每个网络 Agent 一个目录。
    ///
    /// # Errors
    ///
    /// Matrix 地址不合 matrix-sdk 的要求时返回配置错误。
    pub(crate) fn new(
        agents: Arc<dyn NetworkAgentUseCases>,
        secrets: Arc<dyn SecretFactory>,
        matrix_base_url: &str,
        root: PathBuf,
    ) -> Result<Self, MatrixSdkConfigurationError> {
        Ok(Self {
            agents,
            secrets,
            configuration: MatrixSdkConfiguration::new(matrix_base_url, REQUEST_TIMEOUT)?,
            root,
            slots: Mutex::new(HashMap::new()),
        })
    }

    async fn sync_internal(
        &self,
        session: &NetworkAgentSession,
        request: &NetworkAgentSyncRequest,
    ) -> Result<MatrixSyncBatch, NetworkGatewayFailure> {
        // matrix-sdk 的超时不接受 0：“只看一眼”按 1 毫秒算。
        let timeout = DurationMillis::new(request.timeout_millis.max(1))
            .map_err(|_| NetworkGatewayFailure::Internal)?;
        let request = MatrixSyncRequest::new(request.since.clone(), timeout, false)
            .map_err(|_| NetworkGatewayFailure::Internal)?;
        let client = self.client(session).await?;
        tokio::spawn(client.sync(request))
            .await
            .map_err(|_| NetworkGatewayFailure::Internal)?
    }

    async fn prepare_internal(
        &self,
        session: &NetworkAgentSession,
        since: Option<MatrixSyncToken>,
    ) -> Result<(), NetworkGatewayFailure> {
        let timeout = DurationMillis::new(1).map_err(|_| NetworkGatewayFailure::Internal)?;
        let request = MatrixSyncRequest::new(since, timeout, false)
            .map_err(|_| NetworkGatewayFailure::Internal)?;
        let client = self.client(session).await?;
        tokio::spawn(client.clone().sync(request))
            .await
            .map_err(|_| NetworkGatewayFailure::Internal)??;
        let ready = tokio::spawn(client.establish_now())
            .await
            .map_err(|_| NetworkGatewayFailure::Internal)?;
        if ready {
            Ok(())
        } else {
            Err(NetworkGatewayFailure::Unavailable)
        }
    }

    async fn client(
        &self,
        session: &NetworkAgentSession,
    ) -> Result<Arc<OpenClient>, NetworkGatewayFailure> {
        let slot = self
            .slots
            .lock()
            .await
            .entry(session.network_agent_id)
            .or_default()
            .clone();
        let mut state = slot.lock().await;
        match &*state {
            SlotState::Open(client) if !client.conflicted.load(Ordering::Acquire) => {
                client.touch();
                return Ok(client.clone());
            }
            SlotState::Open(_) => {
                *state = SlotState::Failed {
                    retry_at: Instant::now() + OPEN_RETRY,
                };
                return Err(NetworkGatewayFailure::Unavailable);
            }
            SlotState::Failed { retry_at } if Instant::now() < *retry_at => {
                return Err(NetworkGatewayFailure::Unavailable);
            }
            SlotState::Failed { .. } | SlotState::Closed => {}
        }
        match self.open(session).await {
            Ok(client) => {
                let client = Arc::new(client);
                *state = SlotState::Open(client.clone());
                Ok(client)
            }
            Err(failure) => {
                *state = SlotState::Failed {
                    retry_at: Instant::now() + OPEN_RETRY,
                };
                Err(failure)
            }
        }
    }

    async fn open(
        &self,
        session: &NetworkAgentSession,
    ) -> Result<OpenClient, NetworkGatewayFailure> {
        let id = session.network_agent_id;
        let secrets = self
            .agents
            .encryption_secrets(id)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        let store = MatrixSdkStoreConfiguration::encrypted_sqlite(
            self.root.join(id.to_string()).join("matrix-store"),
            secrets.store_passphrase,
        )
        .map_err(|_| NetworkGatewayFailure::Internal)?;
        let metadata = MatrixSessionMetadata::new(
            MatrixUserId::new(session.agent_matrix_user_id.clone())
                .map_err(|_| NetworkGatewayFailure::Internal)?,
            MatrixDeviceId::new(session.matrix_device_id.clone())
                .map_err(|_| NetworkGatewayFailure::Internal)?,
        );
        let connection =
            MatrixSdkClientFactory::with_encrypted_sqlite(self.configuration.clone(), store)
                .restore_with_handoffs(&MatrixSession::new(
                    metadata,
                    session.matrix_access_token.clone(),
                    None,
                ))
                .await
                .map_err(|failure| {
                    if failure.kind() == MatrixFailureKind::CryptographicIdentityConflict {
                        tracing::error!(
                            network_agent.id = %id,
                            "网络 Agent 的 Matrix 设备与加密存储对不上，需要换设备重建"
                        );
                    } else {
                        tracing::warn!(
                            network_agent.id = %id,
                            failure = ?failure.kind(),
                            "网络 Agent 的加密客户端打不开"
                        );
                    }
                    NetworkGatewayFailure::Unavailable
                })?;
        let handoffs = connection.handoff_event_source_handle();
        let drain = tokio::spawn(async move {
            loop {
                if handoffs.receive().await.is_err() {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        });
        Ok(OpenClient {
            id,
            agents: self.agents.clone(),
            secrets: self.secrets.clone(),
            gateway: connection.matrix_gateway_handle(),
            security: connection.security_gateway_handle(),
            last_sync: Mutex::new(None),
            identity: Mutex::new(None),
            identity_ready: AtomicBool::new(false),
            conflicted: AtomicBool::new(false),
            last_used: StdMutex::new(Instant::now()),
            drain,
        })
    }

    async fn forget_internal(&self, id: NetworkAgentId) {
        let slot = self.slots.lock().await.remove(&id);
        if let Some(slot) = slot {
            *slot.lock().await = SlotState::Closed;
        }
    }

    async fn evict_idle_internal(&self) -> usize {
        let slots: Vec<Slot> = self.slots.lock().await.values().cloned().collect();
        let mut closed = 0;
        for slot in slots {
            // 正在打开的不算闲置。
            let Ok(mut state) = slot.try_lock() else {
                continue;
            };
            if let SlotState::Open(client) = &*state
                && client.idle_for() >= IDLE_LIFETIME
            {
                tracing::debug!(network_agent.id = %client.id, "关掉闲置的加密客户端");
                *state = SlotState::Closed;
                closed += 1;
            }
        }
        closed
    }
}

impl OpenClient {
    fn touch(&self) {
        *self
            .last_used
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }

    fn idle_for(&self) -> Duration {
        self.last_used
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .elapsed()
    }

    async fn sync(
        self: Arc<Self>,
        request: MatrixSyncRequest,
    ) -> Result<MatrixSyncBatch, NetworkGatewayFailure> {
        let mut last = self.last_sync.lock().await;
        if self.conflicted.load(Ordering::Acquire) {
            return Err(NetworkGatewayFailure::Unavailable);
        }
        if let Some((since, batch)) = last.as_ref()
            && since.as_ref() == request.since()
        {
            return Ok(batch.clone());
        }
        let batch = self.gateway.sync_once(&request).await.map_err(|failure| {
            if failure.kind() == MatrixFailureKind::CryptographicIdentityConflict {
                self.conflicted.store(true, Ordering::Release);
                tracing::error!(
                    network_agent.id = %self.id,
                    "网络 Agent 的 Matrix 设备与加密存储对不上，需要换设备重建"
                );
            } else {
                tracing::warn!(
                    network_agent.id = %self.id,
                    failure = ?failure.kind(),
                    "网络 Agent 的加密客户端同步失败"
                );
            }
            NetworkGatewayFailure::Unavailable
        })?;
        *last = Some((request.since().cloned(), batch.clone()));
        drop(last);
        self.touch();
        if !self.identity_ready.load(Ordering::Acquire) {
            tokio::spawn(self.clone().ensure_identity());
        }
        Ok(batch)
    }

    /// 同步之后在后台建立加密身份并开启服务器端密钥备份；没建成只记日志，隔一会儿再试，不挡收消息。
    async fn ensure_identity(self: Arc<Self>) {
        // 拿不到锁说明另一次正在建。
        let Ok(mut retry_at) = self.identity.try_lock() else {
            return;
        };
        if retry_at.is_some_and(|at| Instant::now() < at) {
            return;
        }
        self.set_up_identity(&mut retry_at).await;
    }

    /// 进加密房间之前要身份立刻就绪：不看退避；另一次正在建时等它建完。
    async fn establish_now(self: Arc<Self>) -> bool {
        let mut retry_at = self.identity.lock().await;
        self.set_up_identity(&mut retry_at).await
    }

    /// 拿着身份锁调用：建好就记下，没建好就记下次可以再试的时刻。
    async fn set_up_identity(&self, retry_at: &mut Option<Instant>) -> bool {
        if self.identity_ready.load(Ordering::Acquire) {
            return true;
        }
        let ready = self.establish_identity().await && self.ensure_backup().await;
        if ready {
            self.identity_ready.store(true, Ordering::Release);
            *retry_at = None;
        } else {
            *retry_at = Some(Instant::now() + IDENTITY_RETRY);
        }
        ready
    }

    async fn establish_identity(&self) -> bool {
        match self
            .security
            .execute(MatrixSecurityCommand::EstablishIdentity)
            .await
        {
            Ok(MatrixSecurityResult::Identity {
                state: MatrixIdentityState::Ready,
                ..
            }) => true,
            // 身份在服务器上、本地却缺私钥：凭封存的恢复凭据恢复，不另建身份去覆盖它。
            Err(MatrixSecurityFailure::RecoveryRequired) => self.restore_identity().await,
            outcome => {
                tracing::warn!(
                    network_agent.id = %self.id,
                    outcome = ?outcome,
                    "网络 Agent 的加密身份还没建好，稍后再试"
                );
                false
            }
        }
    }

    async fn restore_identity(&self) -> bool {
        let credential = match self.agents.encryption_secrets(self.id).await {
            Ok(secrets) => secrets.recovery_credential,
            Err(failure) => {
                tracing::warn!(
                    network_agent.id = %self.id,
                    failure = ?failure.kind(),
                    "读不到网络 Agent 的恢复凭据"
                );
                return false;
            }
        };
        let Some(credential) = credential else {
            tracing::warn!(
                network_agent.id = %self.id,
                "网络 Agent 的加密身份要恢复，但没有封存的恢复凭据"
            );
            return false;
        };
        let command = MatrixRecoveryCommand::Restore {
            credential: MatrixRecoverySecret::new(credential.expose().to_owned()),
        };
        match self.security.recover(command).await {
            Ok(_) => true,
            Err(failure) => {
                tracing::warn!(
                    network_agent.id = %self.id,
                    failure = ?failure,
                    "网络 Agent 的加密身份恢复失败"
                );
                false
            }
        }
    }

    /// 服务器端密钥备份已经开着就不动；没开就先封存新的恢复凭据，再用它开启。
    async fn ensure_backup(&self) -> bool {
        match self.security.recover(MatrixRecoveryCommand::Inspect).await {
            Ok(inspected) if inspected.state.recovery_available => return true,
            Ok(_) => {}
            Err(failure) => {
                tracing::warn!(
                    network_agent.id = %self.id,
                    failure = ?failure,
                    "查不了网络 Agent 的密钥备份"
                );
                return false;
            }
        }
        let Ok(credential) = self.secrets.generate() else {
            tracing::warn!(network_agent.id = %self.id, "生成不了恢复凭据");
            return false;
        };
        if let Err(failure) = self
            .agents
            .store_recovery_credential(self.id, &credential)
            .await
        {
            tracing::warn!(
                network_agent.id = %self.id,
                failure = ?failure.kind(),
                "恢复凭据存不下，先不开密钥备份"
            );
            return false;
        }
        let command = MatrixRecoveryCommand::Enable {
            passphrase: MatrixRecoverySecret::new(credential.expose().to_owned()),
        };
        match self.security.recover(command).await {
            Ok(_) => true,
            // 服务器上已有备份、却不是用刚封存的凭据开的，也算没建成：宁可再试，不假装能恢复。
            Err(failure) => {
                tracing::warn!(
                    network_agent.id = %self.id,
                    failure = ?failure,
                    "开不了网络 Agent 的服务器端密钥备份"
                );
                false
            }
        }
    }
}

impl EncryptedSessions for EncryptedClients {
    fn sync<'a>(
        &'a self,
        session: &'a NetworkAgentSession,
        request: &'a NetworkAgentSyncRequest,
    ) -> PortFuture<'a, Result<MatrixSyncBatch, NetworkGatewayFailure>> {
        Box::pin(self.sync_internal(session, request))
    }

    fn prepare<'a>(
        &'a self,
        session: &'a NetworkAgentSession,
        since: Option<MatrixSyncToken>,
    ) -> PortFuture<'a, Result<(), NetworkGatewayFailure>> {
        Box::pin(self.prepare_internal(session, since))
    }

    fn forget(&self, id: NetworkAgentId) -> PortFuture<'_, ()> {
        Box::pin(self.forget_internal(id))
    }

    fn evict_idle(&self) -> PortFuture<'_, usize> {
        Box::pin(self.evict_idle_internal())
    }
}

#[cfg(test)]
mod real_dependency_tests {
    use std::sync::{Arc, Mutex};

    use agent_room_application::{
        network_agents::{
            CreateNetworkAgent, CreatedNetworkAgent, NetworkAgentAdmission,
            NetworkAgentEncryptionSecrets, NetworkAgentLobby, NetworkAgentPendingExit,
            NetworkAgentResult, NetworkAgentRoom, NetworkAgentRoomRequest, NetworkAgentSession,
            NetworkAgentTarget, NetworkAgentUseCases, NetworkAgentView,
        },
        ports::{
            MatrixAgentDeviceSessionRequest, MatrixAgentIdentityProvisioner, MatrixAgentLocalpart,
            MatrixAgentUserRegistration, MatrixDeviceId, NetworkAgentSyncRequest, PortFuture,
            SecretFactory, SecretValue,
        },
    };
    use agent_room_domain::{
        ids::{AgentId, AgentInstanceId, NetworkAgentId, PrincipalId},
        time::UtcMillis,
    };
    use agent_room_identity_adapter::SecureSecretFactory;
    use uuid::Uuid;

    use super::{EncryptedClients, EncryptedSessions};
    use crate::config::ControlPlaneConfig;

    /// 只管加密秘密的网络 Agent 用例：存储口令第一次要用时生成，恢复凭据照存。
    #[derive(Default)]
    struct Vault {
        passphrase: Mutex<Option<SecretValue>>,
        recovery: Mutex<Option<SecretValue>>,
    }

    impl Vault {
        fn recovery(&self) -> Option<SecretValue> {
            self.recovery.lock().unwrap().clone()
        }
    }

    impl NetworkAgentUseCases for Vault {
        fn create(
            &self,
            _request: CreateNetworkAgent,
        ) -> PortFuture<'_, NetworkAgentResult<CreatedNetworkAgent>> {
            unreachable!("只测加密客户端")
        }

        fn me<'a>(
            &'a self,
            _token: &'a str,
        ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentView>> {
            unreachable!("只测加密客户端")
        }

        fn disable<'a>(&'a self, _token: &'a str) -> PortFuture<'a, NetworkAgentResult<()>> {
            unreachable!("只测加密客户端")
        }

        fn session<'a>(
            &'a self,
            _token: &'a str,
        ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentSession>> {
            unreachable!("只测加密客户端")
        }

        fn take_message_quota(
            &self,
            _id: NetworkAgentId,
        ) -> PortFuture<'_, NetworkAgentResult<()>> {
            unreachable!("只测加密客户端")
        }

        fn disable_stale(&self) -> PortFuture<'_, NetworkAgentResult<usize>> {
            unreachable!("只测加密客户端")
        }

        fn pending_exits(
            &self,
            _limit: u32,
        ) -> PortFuture<'_, NetworkAgentResult<Vec<NetworkAgentPendingExit>>> {
            unreachable!("只测加密客户端")
        }

        fn mark_rooms_left(&self, _id: NetworkAgentId) -> PortFuture<'_, NetworkAgentResult<()>> {
            unreachable!("只测加密客户端")
        }

        fn public_lobbies(&self) -> PortFuture<'_, NetworkAgentResult<Vec<NetworkAgentLobby>>> {
            unreachable!("只测加密客户端")
        }

        fn admit<'a>(
            &'a self,
            _token: &'a str,
            _room: NetworkAgentRoomRequest,
            _source_digest: [u8; 32],
        ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentAdmission>> {
            unreachable!("只测加密客户端")
        }

        fn enter<'a>(
            &'a self,
            _token: &'a str,
            _target: NetworkAgentTarget,
        ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentRoom>> {
            unreachable!("只测加密客户端")
        }

        fn mark_encrypted(
            &self,
            _id: NetworkAgentId,
        ) -> PortFuture<'_, NetworkAgentResult<UtcMillis>> {
            unreachable!("只测加密客户端")
        }

        fn encryption_secrets(
            &self,
            _id: NetworkAgentId,
        ) -> PortFuture<'_, NetworkAgentResult<NetworkAgentEncryptionSecrets>> {
            let store_passphrase = self
                .passphrase
                .lock()
                .unwrap()
                .get_or_insert_with(|| SecureSecretFactory.generate().unwrap())
                .clone();
            let recovery_credential = self.recovery();
            Box::pin(async move {
                Ok(NetworkAgentEncryptionSecrets {
                    store_passphrase,
                    recovery_credential,
                })
            })
        }

        fn store_recovery_credential<'a>(
            &'a self,
            _id: NetworkAgentId,
            credential: &'a SecretValue,
        ) -> PortFuture<'a, NetworkAgentResult<()>> {
            *self.recovery.lock().unwrap() = Some(credential.clone());
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    #[ignore = "需要先运行 just dev-up，再由自动化脚本注入本地配置"]
    async fn 真实_synapse_上加密客户端建好身份与密钥备份_关掉重开后沿用同一套() {
        let config = ControlPlaneConfig::from_environment().expect("本地运行配置有效");
        let identities =
            crate::build_matrix_identity_provisioner(&config, config.dependencies.timeout)
                .expect("Application Service 配置有效");
        // 与网络 Agent 一样：一个新的 Agent Matrix 用户，一台 AR_<实例> 设备。
        let agent_id = AgentId::from_uuid(Uuid::now_v7());
        let instance_id = AgentInstanceId::from_uuid(Uuid::now_v7());
        let user_id = identities
            .ensure_user(&MatrixAgentUserRegistration::new(
                MatrixAgentLocalpart::from_agent_id(agent_id),
            ))
            .await
            .expect("注册 Agent 的 Matrix 用户");
        let device_id = format!("AR_{}", instance_id.as_uuid().simple());
        let matrix = identities
            .issue_device_session(
                &MatrixAgentDeviceSessionRequest::new(
                    user_id.clone(),
                    MatrixDeviceId::new(device_id.clone()).unwrap(),
                    "网络 Agent 加密客户端验收".to_owned(),
                )
                .unwrap(),
            )
            .await
            .expect("签发设备会话");
        let session = NetworkAgentSession {
            network_agent_id: NetworkAgentId::from_uuid(Uuid::now_v7()),
            principal_id: PrincipalId::from_uuid(Uuid::now_v7()),
            agent_id,
            agent_instance_id: instance_id,
            display_name: "Cipher".to_owned(),
            agent_matrix_user_id: user_id.as_str().to_owned(),
            matrix_access_token: matrix.access_token().clone(),
            instance_signing_seed: SecretValue::new("unused-in-this-test").unwrap(),
            rooms: Vec::new(),
            matrix_device_id: device_id,
            encrypted_since: Some(UtcMillis::new(1).unwrap()),
        };
        let vault = Arc::new(Vault::default());
        let store = tempfile::tempdir().expect("加密存储目录");
        let clients = EncryptedClients::new(
            vault.clone(),
            Arc::new(SecureSecretFactory),
            config.dependencies.matrix_base_url.as_str(),
            store.path().to_path_buf(),
        )
        .expect("matrix-sdk 配置有效");

        clients
            .prepare(&session, None)
            .await
            .expect("加密身份与密钥备份就绪");
        let credential = vault.recovery().expect("开备份之前先封存了恢复凭据");

        // 关掉再打开同一个存储：身份与备份都在，不再生成新的恢复凭据；收消息照常。
        clients.forget(session.network_agent_id).await;
        clients
            .prepare(&session, None)
            .await
            .expect("重开后仍然就绪");
        assert_eq!(vault.recovery(), Some(credential));
        clients
            .sync(
                &session,
                &NetworkAgentSyncRequest {
                    since: None,
                    timeout_millis: 0,
                    timeline_limit: 20,
                },
            )
            .await
            .expect("加密客户端同步");
        clients.forget(session.network_agent_id).await;
    }
}
