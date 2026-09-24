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

    /// 建立加密身份并开启服务器端密钥备份；没建成只记日志，隔一会儿再试，不挡收消息。
    async fn ensure_identity(self: Arc<Self>) {
        // 拿不到锁说明另一次正在建。
        let Ok(mut retry_at) = self.identity.try_lock() else {
            return;
        };
        if self.identity_ready.load(Ordering::Acquire)
            || retry_at.is_some_and(|at| Instant::now() < at)
        {
            return;
        }
        if self.establish_identity().await && self.ensure_backup().await {
            self.identity_ready.store(true, Ordering::Release);
            *retry_at = None;
        } else {
            *retry_at = Some(Instant::now() + IDENTITY_RETRY);
        }
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

    fn forget(&self, id: NetworkAgentId) -> PortFuture<'_, ()> {
        Box::pin(self.forget_internal(id))
    }

    fn evict_idle(&self) -> PortFuture<'_, usize> {
        Box::pin(self.evict_idle_internal())
    }
}
