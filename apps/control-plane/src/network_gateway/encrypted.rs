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
//!
//! 发言也由这个客户端发出（3d）：加密房间里正文先用正文密钥加密，事件由客户端用房间密钥加密。
//! 客户端只能在认识的房间里发言，所以准备好时、进了房间之后都完整同步一次。
//!
//! 存储丢了或与 Matrix 设备对不上（3e）：隔离旧存储，实例换一台新的 Matrix 设备（Synapse 上旧设备
//! 连同它的密钥一起删掉），打开新存储，再凭封存的恢复凭据恢复加密身份、从服务器端备份取回
//! 房间密钥。不沿用旧设备 ID：Synapse 删设备时留着给它的交叉签名，同一 ID 的新设备签不上。

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex as StdMutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use agent_room_application::{
    network_agents::{NetworkAgentEncryptionSecrets, NetworkAgentSession, NetworkAgentUseCases},
    ports::{
        MatrixDeviceId, MatrixFailureKind, MatrixGateway, MatrixRoomAuthorityGateway,
        MatrixSession, MatrixSessionMetadata, MatrixSyncBatch, MatrixSyncRequest, MatrixSyncToken,
        MatrixUserId, NetworkAgentSyncRequest, PortFuture, SecretFactory,
    },
};
use agent_room_bridge_core::{
    matrix_recovery::{MatrixRecoveryCommand, MatrixRecoverySecret},
    matrix_security::{
        MatrixIdentityState, MatrixSecurityCommand, MatrixSecurityFailure, MatrixSecurityGateway,
        MatrixSecurityResult,
    },
    messages::MessageBodyProtectionService,
};
use agent_room_domain::{ids::NetworkAgentId, time::DurationMillis};
use agent_room_matrix_adapter::{
    MatrixSdkClientFactory, MatrixSdkConfiguration, MatrixSdkConfigurationError,
    MatrixSdkStoreConfiguration,
};
use agent_room_message_crypto_adapter::{AesGcmMessageContentCipher, MessageContentRootKey};
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
/// 同一个 Agent 这么久之内只重建一次加密存储：重建会删掉 Matrix 上的设备，不能反复来。
const REBUILD_INTERVAL: Duration = Duration::from_hours(1);

/// 进过加密房间的网络 Agent 的同步来源；测试里用替身。
pub(crate) trait EncryptedSessions: Send + Sync {
    /// 用这个 Agent 的 matrix-sdk 客户端同步一次。事件已解密、已按信任分类；
    /// 每个房间最多带回的条数是客户端的固定上限，不看请求里的 `timeline_limit`。
    fn sync<'a>(
        &'a self,
        session: &'a NetworkAgentSession,
        request: &'a NetworkAgentSyncRequest,
    ) -> PortFuture<'a, Result<MatrixSyncBatch, NetworkGatewayFailure>>;

    /// 进加密房间之前：打开客户端并完整同步一次（认识所有已加入的房间，上传设备密钥与一次性
    /// 密钥），再建好加密身份与密钥备份。没建好就报暂时不可用：这时进去，别人发的消息它会解不开。
    fn prepare<'a>(
        &'a self,
        session: &'a NetworkAgentSession,
    ) -> PortFuture<'a, Result<(), NetworkGatewayFailure>>;

    /// 完整同步一次，让客户端认识所有已加入的房间，包括刚进的。结果留给还没有同步位置的第一次
    /// 长轮询。
    fn refresh<'a>(
        &'a self,
        session: &'a NetworkAgentSession,
    ) -> PortFuture<'a, Result<(), NetworkGatewayFailure>>;

    /// 以这个 Agent 的加密客户端发言要用的几样。
    fn speaker<'a>(
        &'a self,
        session: &'a NetworkAgentSession,
    ) -> PortFuture<'a, Result<EncryptedSpeaker, NetworkGatewayFailure>>;

    /// 停用后关掉它的客户端。
    fn forget(&self, id: NetworkAgentId) -> PortFuture<'_, ()>;

    /// 关掉闲置太久的客户端，返回关了几个。
    fn evict_idle(&self) -> PortFuture<'_, usize>;
}

/// 以加密客户端发言：发事件、看房间加不加密、确认身份就绪并刷新成员身份、加密正文。
#[derive(Clone)]
pub(crate) struct EncryptedSpeaker {
    pub(crate) matrix: Arc<dyn MatrixGateway>,
    pub(crate) authority: Arc<dyn MatrixRoomAuthorityGateway>,
    pub(crate) security: Arc<dyn MatrixSecurityGateway>,
    pub(crate) protection: Arc<MessageBodyProtectionService>,
}

pub(crate) struct EncryptedClients {
    agents: Arc<dyn NetworkAgentUseCases>,
    secrets: Arc<dyn SecretFactory>,
    configuration: MatrixSdkConfiguration,
    root: PathBuf,
    /// 每个 Agent 一个槽：同一个 Agent 的打开串行，不会有两个客户端同时打开同一个存储。
    slots: Mutex<HashMap<NetworkAgentId, Slot>>,
    /// 最近一次重建加密存储的时刻。
    rebuilt: StdMutex<HashMap<NetworkAgentId, Instant>>,
}

/// 打开失败：设备与存储对不上（包括存储丢了）要换设备重建，其余的如实报。
enum OpenFailure {
    Conflict,
    Failed(NetworkGatewayFailure),
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
    authority: Arc<dyn MatrixRoomAuthorityGateway>,
    security: Arc<dyn MatrixSecurityGateway>,
    protection: Arc<MessageBodyProtectionService>,
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
            rebuilt: StdMutex::new(HashMap::new()),
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
        tokio::spawn(client.sync(request, true))
            .await
            .map_err(|_| NetworkGatewayFailure::Internal)?
    }

    async fn prepare_internal(
        &self,
        session: &NetworkAgentSession,
    ) -> Result<(), NetworkGatewayFailure> {
        let client = self.client(session).await?;
        tokio::spawn(client.clone().sync(full_sync()?, false))
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

    async fn refresh_internal(
        &self,
        session: &NetworkAgentSession,
    ) -> Result<(), NetworkGatewayFailure> {
        let client = self.client(session).await?;
        tokio::spawn(client.sync(full_sync()?, false))
            .await
            .map_err(|_| NetworkGatewayFailure::Internal)?
            .map(|_| ())
    }

    async fn speaker_internal(
        &self,
        session: &NetworkAgentSession,
    ) -> Result<EncryptedSpeaker, NetworkGatewayFailure> {
        let client = self.client(session).await?;
        Ok(EncryptedSpeaker {
            matrix: client.gateway.clone(),
            authority: client.authority.clone(),
            security: client.security.clone(),
            protection: client.protection.clone(),
        })
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
            SlotState::Failed { retry_at } if Instant::now() < *retry_at => {
                return Err(NetworkGatewayFailure::Unavailable);
            }
            // 同步时发现设备与存储对不上的，先关掉：重新打开时会再认出冲突，接着重建。
            SlotState::Open(_) | SlotState::Failed { .. } | SlotState::Closed => {
                *state = SlotState::Closed;
            }
        }
        let opened = match self.open(session).await {
            Ok(client) => Ok(Arc::new(client)),
            Err(OpenFailure::Conflict) => self.rebuild(session).await,
            Err(OpenFailure::Failed(failure)) => Err(failure),
        };
        match opened {
            Ok(client) => {
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

    /// 存储丢了或与 Matrix 设备对不上：隔离旧存储（原文件留在恢复目录，不删），实例换一台新的
    /// Matrix 设备（旧设备在 Matrix 上删掉），再打开新存储并完整同步一次。加密身份随后凭封存的
    /// 恢复凭据恢复，房间密钥从服务器端备份取回；其他成员仍认得这个身份。
    async fn rebuild(
        &self,
        session: &NetworkAgentSession,
    ) -> Result<Arc<OpenClient>, NetworkGatewayFailure> {
        let id = session.network_agent_id;
        {
            let mut rebuilt = self.rebuilt.lock().unwrap_or_else(PoisonError::into_inner);
            if rebuilt
                .get(&id)
                .is_some_and(|at| at.elapsed() < REBUILD_INTERVAL)
            {
                tracing::error!(
                    network_agent.id = %id,
                    "网络 Agent 刚重建过加密存储又对不上，这一小时内不再重建"
                );
                return Err(NetworkGatewayFailure::Unavailable);
            }
            rebuilt.insert(id, Instant::now());
        }
        tracing::warn!(
            network_agent.id = %id,
            "网络 Agent 的加密存储丢了或与 Matrix 设备对不上，换一台设备重建"
        );
        let (factory, store_dir, _) = self.factory(id).await?;
        factory
            .quarantine_device_session_store()
            .map_err(|_| NetworkGatewayFailure::Unavailable)?;
        std::fs::create_dir_all(&store_dir).map_err(|_| NetworkGatewayFailure::Unavailable)?;
        let device = self
            .agents
            .replace_matrix_device(id)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        let replaced = NetworkAgentSession {
            matrix_access_token: device.access_token,
            matrix_device_id: device.device_id,
            ..session.clone()
        };
        let client = match self.open(&replaced).await {
            Ok(client) => Arc::new(client),
            Err(OpenFailure::Conflict) => {
                tracing::error!(network_agent.id = %id, "换了设备还是对不上");
                return Err(NetworkGatewayFailure::Unavailable);
            }
            Err(OpenFailure::Failed(failure)) => return Err(failure),
        };
        let synced = tokio::spawn(client.clone().sync(full_sync()?, false))
            .await
            .map_err(|_| NetworkGatewayFailure::Internal)?;
        if let Err(failure) = synced {
            tracing::warn!(
                network_agent.id = %id,
                failure = ?failure,
                "重建后的第一次完整同步没成，下次同步再来"
            );
        }
        Ok(client)
    }

    /// 这个 Agent 的 matrix-sdk 工厂（加密存储在 `root/<ID>/matrix-store`）与它的加密秘密。
    async fn factory(
        &self,
        id: NetworkAgentId,
    ) -> Result<
        (
            MatrixSdkClientFactory,
            PathBuf,
            NetworkAgentEncryptionSecrets,
        ),
        NetworkGatewayFailure,
    > {
        let secrets = self
            .agents
            .encryption_secrets(id)
            .await
            .map_err(NetworkGatewayFailure::Agent)?;
        let store_dir = store_dir(&self.root, id);
        let store = MatrixSdkStoreConfiguration::encrypted_sqlite(
            store_dir.clone(),
            secrets.store_passphrase.clone(),
        )
        .map_err(|_| NetworkGatewayFailure::Internal)?;
        Ok((
            MatrixSdkClientFactory::with_encrypted_sqlite(self.configuration.clone(), store),
            store_dir,
            secrets,
        ))
    }

    async fn open(&self, session: &NetworkAgentSession) -> Result<OpenClient, OpenFailure> {
        let id = session.network_agent_id;
        let (factory, store_dir, secrets) = self.factory(id).await.map_err(OpenFailure::Failed)?;
        // 恢复凭据在（身份与备份建好过），存储目录却没了：存储丢了。同一台设备配新存储会与
        // Matrix 上留着的密钥冲突，按对不上处理，换一台设备重建。
        if secrets.recovery_credential.is_some() && !store_dir.exists() {
            return Err(OpenFailure::Conflict);
        }
        // 与本机 Bridge 一样用 32 字节的正文根密钥；封存的是一段随机文本，取它的 SHA-256。
        let root_key = MessageContentRootKey::from_bytes(
            *self
                .secrets
                .digest(secrets.content_root_key.expose())
                .as_bytes(),
        );
        let protection = Arc::new(MessageBodyProtectionService::new(Arc::new(
            AesGcmMessageContentCipher::new(root_key),
        )));
        let metadata = MatrixSessionMetadata::new(
            MatrixUserId::new(session.agent_matrix_user_id.clone())
                .map_err(|_| OpenFailure::Failed(NetworkGatewayFailure::Internal))?,
            MatrixDeviceId::new(session.matrix_device_id.clone())
                .map_err(|_| OpenFailure::Failed(NetworkGatewayFailure::Internal))?,
        );
        let connection = factory
            .restore_with_handoffs(&MatrixSession::new(
                metadata,
                session.matrix_access_token.clone(),
                None,
            ))
            .await
            .map_err(|failure| {
                if failure.kind() == MatrixFailureKind::CryptographicIdentityConflict {
                    OpenFailure::Conflict
                } else {
                    tracing::warn!(
                        network_agent.id = %id,
                        failure = ?failure.kind(),
                        "网络 Agent 的加密客户端打不开"
                    );
                    OpenFailure::Failed(NetworkGatewayFailure::Unavailable)
                }
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
            authority: connection.room_authority_gateway_handle(),
            security: connection.security_gateway_handle(),
            protection,
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

    /// 同步一次。`replay` 时从上次的起点再来就直接给上次的结果；完整同步要真的同步，让客户端认识
    /// 刚进的房间。
    async fn sync(
        self: Arc<Self>,
        request: MatrixSyncRequest,
        replay: bool,
    ) -> Result<MatrixSyncBatch, NetworkGatewayFailure> {
        let mut last = self.last_sync.lock().await;
        if self.conflicted.load(Ordering::Acquire) {
            return Err(NetworkGatewayFailure::Unavailable);
        }
        if replay && let Some(batch) = replayable(last.as_ref(), request.since()) {
            return Ok(batch);
        }
        let batch = self.gateway.sync_once(&request).await.map_err(|failure| {
            if failure.kind() == MatrixFailureKind::CryptographicIdentityConflict {
                self.conflicted.store(true, Ordering::Release);
                tracing::warn!(
                    network_agent.id = %self.id,
                    "网络 Agent 的 Matrix 设备与加密存储对不上，下次取用时换一台设备重建"
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
    ) -> PortFuture<'a, Result<(), NetworkGatewayFailure>> {
        Box::pin(self.prepare_internal(session))
    }

    fn refresh<'a>(
        &'a self,
        session: &'a NetworkAgentSession,
    ) -> PortFuture<'a, Result<(), NetworkGatewayFailure>> {
        Box::pin(self.refresh_internal(session))
    }

    fn speaker<'a>(
        &'a self,
        session: &'a NetworkAgentSession,
    ) -> PortFuture<'a, Result<EncryptedSpeaker, NetworkGatewayFailure>> {
        Box::pin(self.speaker_internal(session))
    }

    fn forget(&self, id: NetworkAgentId) -> PortFuture<'_, ()> {
        Box::pin(self.forget_internal(id))
    }

    fn evict_idle(&self) -> PortFuture<'_, usize> {
        Box::pin(self.evict_idle_internal())
    }
}

/// 这个 Agent 的加密存储目录。
fn store_dir(root: &Path, id: NetworkAgentId) -> PathBuf {
    root.join(id.to_string()).join("matrix-store")
}

/// 从同一位置再同步时直接给上次的结果，免得 SDK 把同一段 to-device 消息再处理一遍。
/// 上次没往前走（超时了、什么都没有，服务器原样给回起点）就不给：那一段什么也没处理，
/// 真的再同步一次才能收到之后的消息和房间密钥；否则会一直拿同一个空结果，长轮询空转。
fn replayable(
    last: Option<&ProcessedSync>,
    since: Option<&MatrixSyncToken>,
) -> Option<MatrixSyncBatch> {
    let (last_since, batch) = last?;
    (last_since.as_ref() == since && last_since.as_ref() != Some(batch.next_batch()))
        .then(|| batch.clone())
}

/// 完整同步：不带起点，只看一眼。
fn full_sync() -> Result<MatrixSyncRequest, NetworkGatewayFailure> {
    let timeout = DurationMillis::new(1).map_err(|_| NetworkGatewayFailure::Internal)?;
    MatrixSyncRequest::new(None, timeout, false).map_err(|_| NetworkGatewayFailure::Internal)
}

#[cfg(test)]
mod replay_tests {
    use agent_room_application::ports::{MatrixSyncBatch, MatrixSyncToken};

    use super::replayable;

    fn token(value: &str) -> MatrixSyncToken {
        MatrixSyncToken::new(value).expect("同步位置有效")
    }

    fn batch(next: &str) -> MatrixSyncBatch {
        MatrixSyncBatch::new(token(next), Vec::new())
    }

    #[test]
    fn 从同一位置再同步时给上次往前走过的结果() {
        let last = (Some(token("s1")), batch("s2"));
        assert_eq!(
            replayable(Some(&last), Some(&token("s1"))).map(|b| b.next_batch().clone()),
            Some(token("s2"))
        );
        // 第一次同步（没有起点）同样可以重放。
        let first = (None, batch("s1"));
        assert!(replayable(Some(&first), None).is_some());
    }

    #[test]
    fn 上次原地不动或起点不同都要真的同步() {
        // 超时、什么都没有：服务器原样给回起点。重放它会一直拿同一个空结果，长轮询空转。
        let stalled = (Some(token("s1")), batch("s1"));
        assert!(replayable(Some(&stalled), Some(&token("s1"))).is_none());
        let moved_on = (Some(token("s1")), batch("s2"));
        assert!(replayable(Some(&moved_on), Some(&token("s2"))).is_none());
        assert!(replayable(None, Some(&token("s1"))).is_none());
    }
}

#[cfg(test)]
mod real_dependency_tests {
    use std::sync::{Arc, Mutex};

    use agent_room_application::{
        network_agents::{
            CreateNetworkAgent, CreatedNetworkAgent, NetworkAgentAdmission,
            NetworkAgentEncryptionSecrets, NetworkAgentFailure, NetworkAgentFailureKind,
            NetworkAgentLobby, NetworkAgentMatrixDevice, NetworkAgentPendingExit,
            NetworkAgentResult, NetworkAgentRoom, NetworkAgentRoomRequest, NetworkAgentSession,
            NetworkAgentTarget, NetworkAgentUseCases, NetworkAgentView,
        },
        ports::{
            MatrixAgentDeviceSessionRequest, MatrixAgentDeviceSessionRotator,
            MatrixAgentDeviceSessionTarget, MatrixAgentIdentityProvisioner, MatrixAgentLocalpart,
            MatrixAgentUserRegistration, MatrixCreateRoom, MatrixDeviceId, MatrixEvent,
            MatrixEventType, MatrixRoomEncryption, MatrixRoomPreset, MatrixRoomVisibility,
            MatrixTransactionId, MatrixUserId, NetworkAgentSyncRequest, PortFuture, SecretFactory,
            SecretValue,
        },
    };
    use agent_room_bridge_core::messages::ProtectMessageBodyRequest;
    use agent_room_domain::{
        content::{ContentEncryptionMode, ContentMediaType},
        ids::{AgentId, AgentInstanceId, MessageSubmissionId, NetworkAgentId, PrincipalId},
        time::UtcMillis,
    };
    use agent_room_identity_adapter::SecureSecretFactory;
    use serde_json::json;
    use uuid::Uuid;

    use super::{EncryptedClients, EncryptedSessions};
    use crate::config::ControlPlaneConfig;

    /// 只管加密秘密的网络 Agent 用例：存储口令与正文根密钥第一次要用时生成，恢复凭据照存；
    /// 换设备直接找 Application Service，换过的设备按先后记下。
    struct Vault {
        passphrase: Mutex<Option<SecretValue>>,
        root_key: Mutex<Option<SecretValue>>,
        recovery: Mutex<Option<SecretValue>>,
        rotator: Arc<dyn MatrixAgentDeviceSessionRotator>,
        devices: Mutex<Vec<MatrixAgentDeviceSessionRequest>>,
    }

    impl Vault {
        fn new(
            rotator: Arc<dyn MatrixAgentDeviceSessionRotator>,
            device: MatrixAgentDeviceSessionRequest,
        ) -> Self {
            Self {
                passphrase: Mutex::new(None),
                root_key: Mutex::new(None),
                recovery: Mutex::new(None),
                rotator,
                devices: Mutex::new(vec![device]),
            }
        }

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
            let content_root_key = self
                .root_key
                .lock()
                .unwrap()
                .get_or_insert_with(|| SecureSecretFactory.generate().unwrap())
                .clone();
            let recovery_credential = self.recovery();
            Box::pin(async move {
                Ok(NetworkAgentEncryptionSecrets {
                    store_passphrase,
                    content_root_key,
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

        fn replace_matrix_device(
            &self,
            _id: NetworkAgentId,
        ) -> PortFuture<'_, NetworkAgentResult<NetworkAgentMatrixDevice>> {
            Box::pin(async move {
                let current = self.devices.lock().unwrap().last().cloned().unwrap();
                let previous = MatrixAgentDeviceSessionTarget::new(
                    current.user_id().clone(),
                    current.device_id().clone(),
                );
                let next = MatrixAgentDeviceSessionRequest::new(
                    current.user_id().clone(),
                    MatrixDeviceId::new(format!("{}_{}", current.device_id().as_str(), 1)).unwrap(),
                    "网络 Agent 加密客户端验收".to_owned(),
                )
                .unwrap();
                let session = self
                    .rotator
                    .replace_device_session(&previous, &next)
                    .await
                    .map_err(|_| {
                        NetworkAgentFailure::new(NetworkAgentFailureKind::DependencyUnavailable)
                    })?;
                let device_id = next.device_id().as_str().to_owned();
                self.devices.lock().unwrap().push(next);
                Ok(NetworkAgentMatrixDevice {
                    device_id,
                    access_token: session.access_token().clone(),
                })
            })
        }
    }

    /// 与网络 Agent 一样：一个新的 Agent Matrix 用户，一台 AR_<实例> 设备。还交回签发设备会话的
    /// Application Service 与请求，换设备时用。
    async fn network_session(
        config: &ControlPlaneConfig,
    ) -> (
        NetworkAgentSession,
        Arc<dyn MatrixAgentDeviceSessionRotator>,
        MatrixAgentDeviceSessionRequest,
    ) {
        let identities =
            crate::build_matrix_identity_provisioner(config, config.dependencies.timeout)
                .expect("Application Service 配置有效");
        let agent_id = AgentId::from_uuid(Uuid::now_v7());
        let instance_id = AgentInstanceId::from_uuid(Uuid::now_v7());
        let user_id = identities
            .ensure_user(&MatrixAgentUserRegistration::new(
                MatrixAgentLocalpart::from_agent_id(agent_id),
            ))
            .await
            .expect("注册 Agent 的 Matrix 用户");
        let device_id = format!("AR_{}", instance_id.as_uuid().simple());
        let device = MatrixAgentDeviceSessionRequest::new(
            user_id.clone(),
            MatrixDeviceId::new(device_id.clone()).unwrap(),
            "网络 Agent 加密客户端验收".to_owned(),
        )
        .unwrap();
        let matrix = identities
            .issue_device_session(&device)
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
        (session, identities, device)
    }

    /// 在自己建的加密房间里发一条：完整同步后认识这个房间，身份就绪、成员身份刷新过，
    /// 正文先加密，事件由客户端用房间密钥加密后发出。
    async fn send_in_new_encrypted_room(clients: &EncryptedClients, session: &NetworkAgentSession) {
        let speaker = clients.speaker(session).await.expect("发言要用的几样");
        let room = speaker
            .matrix
            .create_room(
                &MatrixCreateRoom::new(
                    Some("网络 Agent 加密发言验收".to_owned()),
                    None,
                    MatrixRoomVisibility::Private,
                    MatrixRoomPreset::PrivateChat,
                    false,
                    Vec::new(),
                )
                .unwrap()
                .with_end_to_end_encryption(),
            )
            .await
            .expect("建加密房间");
        clients.refresh(session).await.expect("完整同步");
        let user_id = MatrixUserId::new(session.agent_matrix_user_id.clone()).unwrap();
        let authority = speaker
            .authority
            .inspect_room_authority(&room, &user_id)
            .await
            .expect("房间状态");
        assert_eq!(authority.encryption(), MatrixRoomEncryption::EndToEnd);
        speaker
            .security
            .ensure_room_ready(&room)
            .await
            .expect("加密房间就绪");
        let body = speaker
            .protection
            .protect(&ProtectMessageBodyRequest {
                submission_id: MessageSubmissionId::from_uuid(Uuid::now_v7()),
                room_id: &room,
                room_encryption: MatrixRoomEncryption::EndToEnd,
                media_type: &ContentMediaType::new("text/plain".to_owned()).unwrap(),
                plaintext: b"only for the room",
                expires_at: None,
            })
            .expect("正文加密");
        assert_eq!(body.encryption_mode(), ContentEncryptionMode::ClientE2ee);
        let event = MatrixEvent::new(
            MatrixEventType::new("io.github.rainyflash.agentroom.message.preview.v1").unwrap(),
            MatrixTransactionId::new(Uuid::now_v7().to_string()).unwrap(),
            json!({"content": {"encryption": {"probe": true}}}),
        )
        .unwrap();
        speaker
            .matrix
            .send_event(&room, &event)
            .await
            .expect("由客户端加密发出");
    }

    #[tokio::test]
    #[ignore = "需要先运行 just dev-up，再由自动化脚本注入本地配置"]
    async fn 真实_synapse_上加密客户端建好身份与密钥备份_重开沿用_存储丢了换设备恢复_都能在加密房间发言()
     {
        // 失败时能看到网关与 matrix-sdk 说了什么。
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new(
                "agent_room_control_plane=debug,agent_room_matrix_adapter=debug,matrix_sdk=info,matrix_sdk_crypto=info",
            ))
            .with_test_writer()
            .try_init();
        let config = ControlPlaneConfig::from_environment().expect("本地运行配置有效");
        let (session, rotator, device) = network_session(&config).await;
        let vault = Arc::new(Vault::new(rotator, device));
        let store = tempfile::tempdir().expect("加密存储目录");
        let clients = EncryptedClients::new(
            vault.clone(),
            Arc::new(SecureSecretFactory),
            config.dependencies.matrix_base_url.as_str(),
            store.path().to_path_buf(),
        )
        .expect("matrix-sdk 配置有效");

        clients
            .prepare(&session)
            .await
            .expect("加密身份与密钥备份就绪");
        let credential = vault.recovery().expect("开备份之前先封存了恢复凭据");

        // 关掉再打开同一个存储：身份与备份都在，不再生成新的恢复凭据；收消息照常。
        clients.forget(session.network_agent_id).await;
        clients.prepare(&session).await.expect("重开后仍然就绪");
        assert_eq!(vault.recovery(), Some(credential.clone()));
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

        send_in_new_encrypted_room(&clients, &session).await;

        // 存储丢了：同一台设备重新签发会话（旧设备连同密钥在 Synapse 上删掉），凭封存的恢复凭据
        // 恢复加密身份。恢复凭据不变，说明身份是恢复回来的而不是新建的；发言照常。
        clients.forget(session.network_agent_id).await;
        let lost = store
            .path()
            .join(session.network_agent_id.to_string())
            .join("matrix-store");
        std::fs::rename(&lost, store.path().join("lost-matrix-store")).expect("模拟存储丢失");
        clients.prepare(&session).await.expect("重建后就绪");
        assert_eq!(vault.devices.lock().unwrap().len(), 2, "换了一次设备");
        assert_eq!(vault.recovery(), Some(credential));
        send_in_new_encrypted_room(&clients, &session).await;
        clients.forget(session.network_agent_id).await;
    }
}
