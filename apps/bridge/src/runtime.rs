use std::{
    fmt,
    io::{self, Write as _},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use agent_room_application::ports::{
    Clock, MatrixFailure, MatrixFailureKind, MatrixGateway, MatrixOperation, MatrixRoomEncryption,
    MatrixRoomId, MatrixSyncRequest, MatrixSyncToken, OidcDeviceAuthorizationPrompt,
    OidcDeviceAuthorizationPromptSink, OidcDevicePromptFailure, PortFuture, ProfileImportConsent,
};
use agent_room_bridge_core::{
    agent_runtime::{
        AgentRuntimeRequestIdFactory, AgentRuntimeSessionConfig, AgentRuntimeSessionDependencies,
        AgentRuntimeSessionFailure, AgentRuntimeSessionFailureKind, AgentRuntimeSessionService,
        RegisteredAgentRuntime,
    },
    agent_verification::{
        AgentEventAuthenticator, AgentInstanceMessageAuthenticator,
        AgentInstanceMessageAuthenticatorDependencies,
    },
    authorization::{
        AuthorizeBridgeDevice, AuthorizedBridgeDevice, BridgeAuthorizationDependencies,
        BridgeAuthorizationFailure, BridgeAuthorizationFailureKind, BridgeAuthorizationService,
    },
    handoffs::{
        EncryptedHandoffToDeviceEventSource, HANDOFF_RECEIPT_EVENT_TYPE,
        HANDOFF_REQUEST_EVENT_TYPE, HandoffAuthorizationGateway, HandoffContentGateway,
        HandoffDeliveryDependencies, HandoffDeliveryService, HandoffInstanceDirectory,
        HandoffReceiptDependencies, HandoffReceiptService, HandoffReceptionDependencies,
        HandoffReceptionService, HandoffStore, HandoffTransportFailureKind,
        ProjectedHandoffContentGateway, TargetedHandoffClaimOutcome, TargetedHandoffInbox,
        TargetedHandoffInboxDependencies, TargetedHandoffInboxService,
        TargetedHandoffInboxServiceFailure, TargetedHandoffQueueGateway, TargetedHandoffTarget,
    },
    lobby_session::{
        AgentLobbySessionConfig, AgentLobbySessionFailure, AgentLobbySessionFailureKind,
        AgentLobbySessionService, ControlPlaneLobbyEntryOutcome, JoinedAgentLobby,
    },
    messages::{
        AutomationAuthorizationGateway, MatrixMessageEventPublisher,
        MessageAuthenticationFailureKind, MessageBodyProtectionService, MessageContentCipher,
        MessageContentGateway, MessageContentReadGateway, MessageProjectionStoreFailureKind,
        MessagePublicationDependencies, MessagePublicationService, MessageStoreFailureKind,
        MessageSyncDependencies, MessageSyncFailure, MessageSyncFailureKind, MessageSyncService,
        OpenMessageContentDependencies, OpenMessageContentService,
    },
    onboarding::BridgeOnboardingService,
    ports::{
        BridgeCredentialFailure, BridgeCredentialFailureKind, DeviceRefreshAttemptIdFactory,
        DeviceSigningIdentityStore, StatusEventIdentifierFactory,
    },
    presence::{
        PresenceLeasePolicy, PresenceProjectionFailureKind, PresenceProjectionRepository,
        PresenceSyncDependencies, PresenceSyncFailure, PresenceSyncFailureKind,
        PresenceSyncService,
    },
    reconnect::{ReconnectBackoff, ReconnectPolicy, SessionRefreshPlan},
    session::{
        ActiveBridgeSession, BridgeSessionDependencies, BridgeSessionFailure,
        BridgeSessionFailureKind, BridgeSessionPolicy, BridgeSessionService,
    },
    status::{
        AgentStatusLeasePolicy, AgentStatusPublicationDependencies, AgentStatusPublicationService,
        AgentStatusRoomTarget, HostAgentState, MatrixStatusStatePublisher,
        StatusPublicationFailure, StatusPublicationFailureKind,
    },
};
use agent_room_bridge_ipc::IpcBridgeState;
use agent_room_bridge_storage_adapter::{
    InMemoryPresenceProjectionRepository, SqliteHandoffStore, SqliteTargetedHandoffInbox,
};
use agent_room_domain::{
    agent_status::AgentStatusVisibility,
    devices::DevicePlatform,
    ids::{AgentId, AgentInstanceRegistrationRequestId, DeviceRefreshAttemptId, RoomCatalogId},
    time::{DurationMillis, UtcMillis},
};
use agent_room_identity_adapter::{
    DiscoveredOidcDeviceGrant, Ed25519AgentInstanceSignatureVerifier, OidcDeviceGrantConfig,
    SecureSecretFactory,
};
use agent_room_matrix_adapter::{
    MatrixSdkClientFactory, MatrixSdkConfiguration, MatrixSdkStoreConfiguration,
};
use agent_room_message_crypto_adapter::AesGcmMessageContentCipher;
use serde::Serialize;
use tokio::{
    sync::{oneshot, watch},
    task::JoinHandle,
    time::sleep,
};

use crate::{
    agent_status::AgentStatusPublicationHandle,
    config::BridgeConfig,
    ipc::{
        BridgeAgentRuntimeReader, BridgeAgentRuntimeSnapshot, BridgeIpcFailure,
        BridgeIpcFailureKind, BridgeIpcRequestHandler, BridgeIpcServer, BridgeStatusReader,
        BridgeStatusSnapshot, FoundationBridgeIpcRequestHandler,
    },
    runtime_files::{
        BridgeExclusiveLock, BridgeRuntimeFileFailure, BridgeRuntimeFileFailureKind,
        BridgeRuntimePaths,
    },
    secure_storage::{
        BridgeRuntimeSecrets, OsAgentInstanceSigningIdentityStore, OsAgentRuntimeCredentialVault,
        OsBridgeRuntimeSecretVault, OsDeviceCredentialVault, OsDeviceSigningIdentityStore,
    },
};
use agent_room_bridge::control_plane::{
    ControlPlaneHttpConfig, ReqwestAgentInstanceVerificationGateway,
    ReqwestControlPlaneAgentRuntimeGateway, ReqwestControlPlaneAutomationAuthorizationGateway,
    ReqwestControlPlaneContentGateway, ReqwestControlPlaneDeviceGateway,
    ReqwestControlPlaneHandoffGateway, ReqwestControlPlaneLobbyEntryGateway,
    ReqwestControlPlaneMessageContentGateway, ReqwestControlPlaneOnboardingGateway,
    ReqwestTargetedHandoffQueueGateway,
};
use agent_room_bridge_storage_adapter::{
    SqliteMessageSubmissionRepository, SqliteMessageTimelineRepository,
};

const DESKTOP_RUNTIME_CAPABILITY_VERSION: &str = "1.0";
mod host_sessions;
use crate::host_sessions::{HostSessionRegistry, SessionAwareIpcHandler};
const FOUNDATION_AGENT_CAPABILITIES: [&str; 9] = [
    "matrix.security",
    "self.read",
    "previews.read",
    "presence.read",
    "content.read",
    "status.publish",
    "message.send",
    "handoff.consume",
    "handoff.decline",
];
const STATUS_LEASE_LIFETIME_MILLIS: u64 = 300_000;
const STATUS_RENEWAL_INTERVAL_MILLIS: u64 = 120_000;
const STATUS_RENEWAL_JITTER_MILLIS: u64 = 15_000;
const STATUS_ALLOWED_CLOCK_SKEW_MILLIS: u64 = 15_000;
const TARGETED_HANDOFF_STORED_DELAY: Duration = Duration::from_millis(250);
const TARGETED_HANDOFF_IDLE_DELAY: Duration = Duration::from_secs(5);
const TARGETED_HANDOFF_FAILURE_DELAY: Duration = Duration::from_secs(15);

pub(crate) async fn run() -> Result<(), BridgeRuntimeError> {
    let config = BridgeConfig::from_environment()
        .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?;
    let paths = BridgeRuntimePaths::new(config.data_root.clone());
    paths.prepare().map_err(BridgeRuntimeError::runtime_files)?;
    let _instance_lock = BridgeExclusiveLock::acquire(paths.instance_lock_path())
        .map_err(BridgeRuntimeError::instance_lock)?;
    let _matrix_store_lock = BridgeExclusiveLock::acquire(paths.matrix_store_lock_path())
        .map_err(BridgeRuntimeError::matrix_store_lock)?;
    let runtime_secrets =
        OsBridgeRuntimeSecretVault::system(config.secure_storage_service.as_str())
            .load_or_create()
            .map_err(BridgeRuntimeError::runtime_secrets)?;
    #[cfg(unix)]
    crate::ipc::recover_endpoint(&paths, runtime_secrets.installation_id())
        .await
        .map_err(BridgeRuntimeError::ipc)?;
    let matrix = initialize_matrix(&config, &paths, &runtime_secrets).await?;
    let handoff_store = initialize_handoff_store(&paths, &runtime_secrets).await?;
    let device_session = initialize_device_session(&config).await?;
    let agent_session = initialize_agent_session(
        &config,
        &paths,
        &runtime_secrets,
        device_session.service.clone(),
        matrix,
        handoff_store,
    )
    .await?;

    let status = Arc::new(BridgeRuntimeStatus::new(
        SystemClock.now().value(),
        agent_session.is_some(),
    ));
    let reception = Arc::new(
        agent_room_bridge::control_plane::reception::HttpReceptionGateway::new(
            &ControlPlaneHttpConfig {
                base_url: config.control_plane_url.clone(),
                request_timeout: config.request_timeout,
            },
            device_session.service.clone(),
        )
        .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?,
    );
    let request_handler: Arc<dyn BridgeIpcRequestHandler> = match agent_session.as_ref() {
        Some(runtime) => Arc::new(
            FoundationBridgeIpcRequestHandler::with_agent_runtime(
                crate::ipc::AgentRuntimeConsumer::Desktop,
                status.clone(),
                runtime.state.clone(),
                runtime.previews.clone(),
                runtime.content.clone(),
                Arc::new(SystemClock),
            )
            .with_attachment_directory(paths.attachment_root().to_path_buf())
            .with_reception(reception.clone()),
        ),
        None => Arc::new(
            FoundationBridgeIpcRequestHandler::with_onboarding(
                status.clone(),
                initialize_onboarding(&config, device_session.service.clone())?,
            )
            .with_reception(reception),
        ),
    };
    let host_sessions = Arc::new(HostSessionRegistry::new(Arc::new(
        host_sessions::HostAgentRuntimeFactory::new(
            config.clone(),
            paths.clone(),
            device_session.service.clone(),
        )?,
    )));
    let request_handler = Arc::new(SessionAwareIpcHandler {
        default: request_handler,
        sessions: host_sessions.clone(),
        connection_status: Arc::new(DeviceConnectionStatus(status.clone())),
    });
    let server = BridgeIpcServer::bind(
        &paths,
        runtime_secrets.installation_id().clone(),
        runtime_secrets.ipc_shared_secret().clone(),
        request_handler,
    )
    .map_err(BridgeRuntimeError::ipc)?;
    finish_starting(&status, &device_session, agent_session.as_ref());
    announce_supervisor_ready()?;
    run_until_shutdown(
        wait_for_exit_signal(config.exit_with_supervisor),
        server,
        status,
        device_session,
        agent_session,
        host_sessions,
    )
    .await
}

/// 按初始会话标记各组件是否就绪，结束启动态。
fn finish_starting(
    status: &BridgeRuntimeStatus,
    device_session: &DeviceSessionRuntime,
    agent_session: Option<&AgentSessionRuntime>,
) {
    status.set_component_ready(
        BridgeRuntimeStatus::DEVICE_COMPONENT,
        device_session.initial_session.is_some(),
    );
    if let Some(runtime) = agent_session {
        status.set_component_ready(
            BridgeRuntimeStatus::AGENT_COMPONENT,
            runtime.initial_session.is_some(),
        );
    }
    status.finish_starting();
}

fn initialize_onboarding(
    config: &BridgeConfig,
    device_session: Arc<BridgeSessionService>,
) -> Result<Arc<BridgeOnboardingService>, BridgeRuntimeError> {
    let gateway = ReqwestControlPlaneOnboardingGateway::new(
        &ControlPlaneHttpConfig {
            base_url: config.control_plane_url.clone(),
            request_timeout: config.request_timeout,
        },
        device_session,
    )
    .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?;
    Ok(Arc::new(BridgeOnboardingService::new(Arc::new(gateway))))
}

struct DeviceSessionRuntime {
    service: Arc<BridgeSessionService>,
    initial_session: Option<ActiveBridgeSession>,
    clock: Arc<dyn Clock>,
    refresh_lead_time: DurationMillis,
    reconnect_policy: ReconnectPolicy,
}

struct AgentSessionRuntime {
    service: Arc<AgentRuntimeSessionService>,
    signing_identities: Arc<dyn DeviceSigningIdentityStore>,
    config: AgentRuntimeSessionConfig,
    lobby: Arc<AgentLobbySessionService>,
    lobby_config: AgentLobbySessionConfig,
    matrix: Arc<MatrixSdkClientFactory>,
    messages: Arc<MessageSyncService>,
    presence: Arc<PresenceSyncService>,
    presence_projections: Arc<dyn PresenceProjectionRepository>,
    previews: Arc<SqliteMessageTimelineRepository>,
    content: Arc<OpenMessageContentService>,
    content_protection: Arc<MessageBodyProtectionService>,
    outbound_content: Arc<dyn MessageContentGateway>,
    submissions: Arc<SqliteMessageSubmissionRepository>,
    automation: Arc<dyn AutomationAuthorizationGateway>,
    handoffs: AgentHandoffServices,
    state: Arc<BridgeAgentRuntimeState>,
    status_policy: AgentStatusLeasePolicy,
    sync_timeout: DurationMillis,
    initial_session: Option<AgentOnlineSession>,
    report_to_desktop_supervisor: bool,
    reconnect_policy: ReconnectPolicy,
    matrix_identity_recovery: MatrixIdentityRecovery,
}

const MATRIX_IDENTITY_RECOVERY_UNTOUCHED: u8 = 0;
const MATRIX_IDENTITY_RECOVERY_STORE_QUARANTINED: u8 = 1;
const MATRIX_IDENTITY_RECOVERY_COMPLETE: u8 = 2;

struct MatrixIdentityRecovery {
    phase: AtomicU8,
}

impl MatrixIdentityRecovery {
    const fn new() -> Self {
        Self {
            phase: AtomicU8::new(MATRIX_IDENTITY_RECOVERY_UNTOUCHED),
        }
    }

    fn prepare_store(&self, matrix: &MatrixSdkClientFactory) -> Result<bool, MatrixFailure> {
        loop {
            match self.phase.load(Ordering::Acquire) {
                MATRIX_IDENTITY_RECOVERY_COMPLETE => return Ok(false),
                MATRIX_IDENTITY_RECOVERY_STORE_QUARANTINED => return Ok(true),
                MATRIX_IDENTITY_RECOVERY_UNTOUCHED => {
                    if self
                        .phase
                        .compare_exchange(
                            MATRIX_IDENTITY_RECOVERY_UNTOUCHED,
                            MATRIX_IDENTITY_RECOVERY_STORE_QUARANTINED,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_err()
                    {
                        continue;
                    }
                    if let Err(failure) = matrix.quarantine_device_session_store() {
                        self.phase
                            .store(MATRIX_IDENTITY_RECOVERY_UNTOUCHED, Ordering::Release);
                        return Err(failure);
                    }
                    return Ok(true);
                }
                _ => {
                    return Err(MatrixFailure::new(
                        MatrixOperation::InitializeStore,
                        MatrixFailureKind::InvalidResponse,
                    ));
                }
            }
        }
    }

    fn complete(&self) {
        self.phase
            .store(MATRIX_IDENTITY_RECOVERY_COMPLETE, Ordering::Release);
    }
}

#[derive(Clone)]
struct AgentSessionTarget {
    agent_id: AgentId,
    lobby_catalog_id: RoomCatalogId,
    room: Option<agent_room_domain::rooms::MatrixRoomReference>,
}

struct AgentHandoffStores {
    legacy: Arc<SqliteHandoffStore>,
    targeted: Arc<SqliteTargetedHandoffInbox>,
}

struct AgentMessageServices {
    sync: Arc<MessageSyncService>,
    projections: Arc<SqliteMessageTimelineRepository>,
    content: Arc<OpenMessageContentService>,
    content_protection: Arc<MessageBodyProtectionService>,
    outbound_content: Arc<dyn MessageContentGateway>,
    submissions: Arc<SqliteMessageSubmissionRepository>,
    automation: Arc<dyn AutomationAuthorizationGateway>,
    authenticator: Arc<dyn AgentEventAuthenticator>,
    content_reader: Arc<dyn MessageContentReadGateway>,
    handoff_content: Arc<dyn HandoffContentGateway>,
    presence: Arc<PresenceSyncService>,
    presence_projections: Arc<dyn PresenceProjectionRepository>,
}

struct AgentHandoffServices {
    authorization: Arc<dyn HandoffAuthorizationGateway>,
    directory: Arc<dyn HandoffInstanceDirectory>,
    authenticator: Arc<dyn AgentEventAuthenticator>,
    content: Arc<dyn HandoffContentGateway>,
    store: Arc<dyn HandoffStore>,
    targeted_queue: Arc<dyn TargetedHandoffQueueGateway>,
    targeted_inbox: Arc<dyn TargetedHandoffInbox>,
    targeted_content: Arc<dyn MessageContentReadGateway>,
}

struct AgentOnlineSession {
    security: Arc<dyn agent_room_bridge_core::matrix_security::MatrixSecurityGateway>,
    runtime: RegisteredAgentRuntime,
    lobby: JoinedAgentLobby,
    room_id: MatrixRoomId,
    matrix: Arc<dyn MatrixGateway>,
    room_authority: Arc<dyn agent_room_application::ports::MatrixRoomAuthorityGateway>,
    status: Arc<AgentStatusPublicationHandle>,
    publication: Arc<MessagePublicationService>,
    content_protection: Arc<MessageBodyProtectionService>,
    handoffs: Arc<HandoffReceptionService>,
    handoff_delivery: Arc<HandoffDeliveryService>,
    handoff_worker: HandoffEventWorker,
    targeted_handoffs: Arc<TargetedHandoffInboxService>,
    targeted_handoff_worker: TargetedHandoffWorker,
    presence_projections: Arc<dyn PresenceProjectionRepository>,
    next_batch: Option<MatrixSyncToken>,
}

struct HandoffEventWorker {
    task: JoinHandle<()>,
    shutdown: watch::Sender<bool>,
    terminal_failure: watch::Receiver<Option<HandoffTransportFailureKind>>,
}

impl AgentOnlineSession {
    async fn disconnect(&mut self) {
        self.stop_workers().await;
        match tokio::time::timeout(
            Duration::from_secs(5),
            self.status.publish(HostAgentState::Disconnected),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(failure)) => {
                tracing::warn!(failure_kind = ?failure.kind(), "人物退出状态发布失败，将由租约到期回收");
            }
            Err(_) => {
                tracing::warn!("人物退出状态发布超时，将由租约到期回收");
            }
        }
    }

    async fn stop_workers(&mut self) {
        self.handoff_worker.shutdown.send_replace(true);
        self.targeted_handoff_worker.shutdown.send_replace(true);
        let (handoff, targeted) = tokio::join!(
            &mut self.handoff_worker.task,
            &mut self.targeted_handoff_worker.task,
        );
        for result in [handoff, targeted] {
            if let Err(error) = result {
                tracing::warn!(%error, "人物后台任务退出异常");
            }
        }
    }
}

impl Drop for HandoffEventWorker {
    fn drop(&mut self) {
        self.task.abort();
    }
}

trait TargetedHandoffPoller: Send + Sync {
    fn claim_once(
        &self,
    ) -> PortFuture<'_, Result<TargetedHandoffClaimOutcome, TargetedHandoffInboxServiceFailure>>;
}

impl TargetedHandoffPoller for TargetedHandoffInboxService {
    fn claim_once(
        &self,
    ) -> PortFuture<'_, Result<TargetedHandoffClaimOutcome, TargetedHandoffInboxServiceFailure>>
    {
        Box::pin(TargetedHandoffInboxService::claim_once(self))
    }
}

#[derive(Clone, Copy)]
struct TargetedHandoffPollingPolicy {
    stored: Duration,
    idle: Duration,
    failure: Duration,
}

struct TargetedHandoffWorker {
    task: JoinHandle<()>,
    shutdown: watch::Sender<bool>,
}

impl TargetedHandoffWorker {
    fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
}

impl Drop for TargetedHandoffWorker {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct BridgeAgentRuntimeState {
    snapshot: watch::Sender<Option<BridgeAgentRuntimeSnapshot>>,
}

impl BridgeAgentRuntimeState {
    fn new() -> Self {
        let (snapshot, _receiver) = watch::channel(None);
        Self { snapshot }
    }

    fn publish(&self, online: &AgentOnlineSession) {
        self.snapshot.send_replace(Some(
            BridgeAgentRuntimeSnapshot::new(
                online.runtime.identity().clone(),
                online
                    .runtime
                    .matrix_session()
                    .metadata()
                    .device_id()
                    .as_str(),
                online.room_id.clone(),
                FOUNDATION_AGENT_CAPABILITIES,
            )
            .with_room_authority(online.room_authority.clone())
            .with_security(online.security.clone())
            .with_status(online.status.clone())
            .with_message_publication(online.publication.clone())
            .with_message_content_protection(online.content_protection.clone())
            .with_room_encryption(MatrixRoomEncryption::Unencrypted)
            .with_handoff_delivery(online.handoff_delivery.clone())
            .with_handoffs(online.handoffs.clone())
            .with_targeted_handoffs(online.targeted_handoffs.clone())
            .with_presence(online.presence_projections.clone()),
        ));
    }

    fn clear(&self) {
        self.snapshot.send_replace(None);
    }
}

impl BridgeAgentRuntimeReader for BridgeAgentRuntimeState {
    fn read_agent_runtime(&self) -> Option<BridgeAgentRuntimeSnapshot> {
        self.snapshot.borrow().clone()
    }
}

#[derive(Debug, Clone, Copy)]
enum AgentOnlineFailure {
    AgentRuntime(AgentRuntimeSessionFailure),
    Lobby(AgentLobbySessionFailure),
    ProvisioningBusy(UtcMillis),
    CapacityChanged,
    Matrix(MatrixFailure),
    SigningIdentity(BridgeCredentialFailure),
    Status(StatusPublicationFailure),
    PresenceSync(PresenceSyncFailure),
    MessageSync(MessageSyncFailure),
    HandoffTransport(HandoffTransportFailureKind),
    TargetedHandoffWorker,
    InvalidRoom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentOnlineFailureKind {
    AgentRuntime,
    Lobby,
    ProvisioningBusy,
    CapacityChanged,
    Matrix,
    SigningIdentity,
    Status,
    PresenceSync,
    MessageSync,
    HandoffTransport(HandoffTransportFailureKind),
    TargetedHandoffWorker,
    InvalidRoom,
}

impl AgentOnlineFailure {
    const fn kind(self) -> AgentOnlineFailureKind {
        match self {
            Self::AgentRuntime(_) => AgentOnlineFailureKind::AgentRuntime,
            Self::Lobby(_) => AgentOnlineFailureKind::Lobby,
            Self::ProvisioningBusy(_) => AgentOnlineFailureKind::ProvisioningBusy,
            Self::CapacityChanged => AgentOnlineFailureKind::CapacityChanged,
            Self::Matrix(_) => AgentOnlineFailureKind::Matrix,
            Self::SigningIdentity(_) => AgentOnlineFailureKind::SigningIdentity,
            Self::Status(_) => AgentOnlineFailureKind::Status,
            Self::PresenceSync(_) => AgentOnlineFailureKind::PresenceSync,
            Self::MessageSync(_) => AgentOnlineFailureKind::MessageSync,
            Self::HandoffTransport(failure) => AgentOnlineFailureKind::HandoffTransport(failure),
            Self::TargetedHandoffWorker => AgentOnlineFailureKind::TargetedHandoffWorker,
            Self::InvalidRoom => AgentOnlineFailureKind::InvalidRoom,
        }
    }
}

async fn initialize_device_session(
    config: &BridgeConfig,
) -> Result<DeviceSessionRuntime, BridgeRuntimeError> {
    let control_plane = Arc::new(
        ReqwestControlPlaneDeviceGateway::new(&ControlPlaneHttpConfig {
            base_url: config.control_plane_url.clone(),
            request_timeout: config.request_timeout,
        })
        .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?,
    );
    let oidc = Arc::new(
        DiscoveredOidcDeviceGrant::new(OidcDeviceGrantConfig {
            issuer_url: config.oidc_issuer_url.clone(),
            client_id: config.oidc_client_id.clone(),
            request_timeout: config.request_timeout,
            maximum_polling_duration: config.authorization_timeout,
        })
        .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?,
    );
    let signing_identities = Arc::new(OsDeviceSigningIdentityStore::system(
        config.secure_storage_service.as_str(),
    ));
    let credentials = Arc::new(OsDeviceCredentialVault::system(
        config.secure_storage_service.as_str(),
    ));
    let secrets = Arc::new(SecureSecretFactory);
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let refresh_lead_time = domain_duration(config.refresh_lead_time)?;
    let reconnect_policy = ReconnectPolicy::new(
        domain_duration(config.reconnect_initial_delay)?,
        domain_duration(config.reconnect_maximum_delay)?,
    )
    .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?;
    let session_service = Arc::new(BridgeSessionService::new(
        BridgeSessionDependencies {
            signing_identities: signing_identities.clone(),
            control_plane: control_plane.clone(),
            credentials: credentials.clone(),
            secrets: secrets.clone(),
            clock: clock.clone(),
            refresh_attempts: Arc::new(SystemDeviceRefreshAttempts),
        },
        BridgeSessionPolicy::new(refresh_lead_time),
    ));
    if config.reset_device_session {
        // 桌面端「重新授权这台电脑」：实例锁已在调用方持有，不会与另一个 Bridge 抢写凭据。
        session_service
            .forget_device_session()
            .await
            .map_err(BridgeRuntimeError::session)?;
        tracing::warn!("已按桌面端请求清除本机设备会话凭据，接下来重新授权这台设备");
    }

    let authorization_service = BridgeAuthorizationService::new(BridgeAuthorizationDependencies {
        oidc,
        signing_identities,
        control_plane,
        credentials,
        secrets,
    });
    let initial_session = establish_initial_session(
        config,
        &session_service,
        authorization_service,
        reconnect_policy,
    )
    .await?;

    Ok(DeviceSessionRuntime {
        service: session_service,
        initial_session,
        clock,
        refresh_lead_time,
        reconnect_policy,
    })
}

async fn initialize_agent_session(
    config: &BridgeConfig,
    paths: &BridgeRuntimePaths,
    runtime_secrets: &BridgeRuntimeSecrets,
    device_session: Arc<BridgeSessionService>,
    matrix: Arc<MatrixSdkClientFactory>,
    handoff_store: Arc<SqliteHandoffStore>,
) -> Result<Option<AgentSessionRuntime>, BridgeRuntimeError> {
    let (Some(agent_id), Some(lobby_catalog_id)) =
        (config.agent_id, config.public_lobby_catalog_id)
    else {
        return Ok(None);
    };
    let runtime = compose_agent_session_runtime(
        config,
        paths,
        runtime_secrets,
        device_session,
        matrix,
        AgentHandoffStores {
            legacy: handoff_store,
            targeted: initialize_targeted_handoff_inbox(paths).await?,
        },
        AgentSessionTarget {
            agent_id,
            lobby_catalog_id,
            room: None,
        },
    )
    .await?;
    // Start the local endpoint as soon as the device is available. The default
    // character joins in maintain_agent_session, with its own retry/failure
    // state, just like host task sessions. A bad old room must not block IPC.
    Ok(Some(runtime))
}

async fn compose_agent_session_runtime(
    config: &BridgeConfig,
    paths: &BridgeRuntimePaths,
    runtime_secrets: &BridgeRuntimeSecrets,
    device_session: Arc<BridgeSessionService>,
    matrix: Arc<MatrixSdkClientFactory>,
    handoff_stores: AgentHandoffStores,
    target: AgentSessionTarget,
) -> Result<AgentSessionRuntime, BridgeRuntimeError> {
    let http = ControlPlaneHttpConfig {
        base_url: config.control_plane_url.clone(),
        request_timeout: config.request_timeout,
    };
    let control_plane = Arc::new(
        ReqwestControlPlaneAgentRuntimeGateway::new(&http, device_session.clone())
            .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?,
    );
    let signing_identities: Arc<dyn DeviceSigningIdentityStore> = Arc::new(
        OsAgentInstanceSigningIdentityStore::system(config.secure_storage_service.as_str()),
    );
    let service = Arc::new(AgentRuntimeSessionService::new(
        AgentRuntimeSessionDependencies {
            signing_identities: signing_identities.clone(),
            control_plane,
            credentials: Arc::new(OsAgentRuntimeCredentialVault::system(
                config.secure_storage_service.as_str(),
            )),
            identifiers: Arc::new(SystemAgentRuntimeIdentifiers),
        },
    ));
    let agent_config = AgentRuntimeSessionConfig::new(
        target.agent_id,
        "agent-room-mcp",
        DESKTOP_RUNTIME_CAPABILITY_VERSION,
    )
    .map_err(BridgeRuntimeError::agent_runtime)?;
    let lobby = Arc::new(AgentLobbySessionService::new(Arc::new(
        ReqwestControlPlaneLobbyEntryGateway::new(&http, device_session.clone())
            .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?,
    )));
    let message_services = compose_agent_message_services(
        &http,
        paths,
        runtime_secrets,
        device_session.clone(),
        target.agent_id,
    )
    .await?;
    let handoffs =
        compose_agent_handoff_services(&http, device_session, &message_services, handoff_stores)?;
    let state = Arc::new(BridgeAgentRuntimeState::new());
    let lobby_config = AgentLobbySessionConfig::new(
        target.lobby_catalog_id,
        config.lobby_language.clone(),
        config.lobby_region.clone(),
    )
    .with_target_room(target.room);
    let sync_timeout = domain_duration(config.matrix_sync_timeout)?;
    let status_policy = AgentStatusLeasePolicy::new(
        DurationMillis::new(STATUS_LEASE_LIFETIME_MILLIS)
            .map_err(|_| BridgeRuntimeError::status_policy())?,
        DurationMillis::new(STATUS_RENEWAL_INTERVAL_MILLIS)
            .map_err(|_| BridgeRuntimeError::status_policy())?,
        DurationMillis::new(STATUS_RENEWAL_JITTER_MILLIS)
            .map_err(|_| BridgeRuntimeError::status_policy())?,
    )
    .map_err(|_| BridgeRuntimeError::status_policy())?;
    MatrixSyncRequest::new(None, sync_timeout, true)
        .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?;
    Ok(AgentSessionRuntime {
        service,
        signing_identities,
        config: agent_config,
        report_to_desktop_supervisor: true,
        lobby,
        lobby_config,
        matrix,
        messages: message_services.sync,
        presence: message_services.presence,
        presence_projections: message_services.presence_projections,
        previews: message_services.projections,
        content: message_services.content,
        content_protection: message_services.content_protection,
        outbound_content: message_services.outbound_content,
        submissions: message_services.submissions,
        automation: message_services.automation,
        handoffs,
        state,
        status_policy,
        sync_timeout,
        initial_session: None,
        reconnect_policy: ReconnectPolicy::new(
            domain_duration(config.reconnect_initial_delay)?,
            domain_duration(config.reconnect_maximum_delay)?,
        )
        .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?,
        matrix_identity_recovery: MatrixIdentityRecovery::new(),
    })
}

fn compose_agent_handoff_services(
    http: &ControlPlaneHttpConfig,
    device_session: Arc<BridgeSessionService>,
    messages: &AgentMessageServices,
    stores: AgentHandoffStores,
) -> Result<AgentHandoffServices, BridgeRuntimeError> {
    let legacy = Arc::new(
        ReqwestControlPlaneHandoffGateway::new(http, device_session.clone())
            .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?,
    );
    let targeted_queue: Arc<dyn TargetedHandoffQueueGateway> = Arc::new(
        ReqwestTargetedHandoffQueueGateway::new(http, device_session)
            .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?,
    );
    Ok(AgentHandoffServices {
        authorization: legacy.clone(),
        directory: legacy,
        authenticator: messages.authenticator.clone(),
        content: messages.handoff_content.clone(),
        store: stores.legacy,
        targeted_queue,
        targeted_inbox: stores.targeted,
        targeted_content: messages.content_reader.clone(),
    })
}

async fn compose_agent_message_services(
    http: &ControlPlaneHttpConfig,
    paths: &BridgeRuntimePaths,
    runtime_secrets: &BridgeRuntimeSecrets,
    device_session: Arc<BridgeSessionService>,
    actor_agent_id: AgentId,
) -> Result<AgentMessageServices, BridgeRuntimeError> {
    let verification = Arc::new(
        ReqwestAgentInstanceVerificationGateway::new(http, device_session.clone())
            .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?,
    );
    let projections = Arc::new(
        SqliteMessageTimelineRepository::open(
            paths.message_database(),
            runtime_secrets.message_projection_storage_key(),
        )
        .await
        .map_err(|failure| BridgeRuntimeError::message_store(&failure))?,
    );
    let submissions = Arc::new(
        SqliteMessageSubmissionRepository::open(paths.message_database())
            .await
            .map_err(|failure| BridgeRuntimeError::message_store(&failure))?,
    );
    let outbound_content: Arc<dyn MessageContentGateway> = Arc::new(
        ReqwestControlPlaneMessageContentGateway::new(http, device_session.clone(), actor_agent_id)
            .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?,
    );
    let automation: Arc<dyn AutomationAuthorizationGateway> = Arc::new(
        ReqwestControlPlaneAutomationAuthorizationGateway::new(http, device_session.clone())
            .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?,
    );
    let content_reader: Arc<dyn MessageContentReadGateway> = Arc::new(
        ReqwestControlPlaneContentGateway::new(http, device_session, actor_agent_id)
            .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?,
    );
    let content_cryptography: Arc<dyn MessageContentCipher> = Arc::new(
        AesGcmMessageContentCipher::new(runtime_secrets.message_content_root_key().clone()),
    );
    let content = Arc::new(OpenMessageContentService::new(
        OpenMessageContentDependencies {
            projections: projections.clone(),
            content: content_reader.clone(),
            cryptography: Some(content_cryptography.clone()),
        },
    ));
    let content_protection = Arc::new(MessageBodyProtectionService::new(content_cryptography));
    let authenticator: Arc<dyn AgentEventAuthenticator> = Arc::new(
        AgentInstanceMessageAuthenticator::new(AgentInstanceMessageAuthenticatorDependencies {
            verification,
            signatures: Arc::new(Ed25519AgentInstanceSignatureVerifier),
        }),
    );
    let sync = Arc::new(MessageSyncService::new(MessageSyncDependencies {
        authenticator: authenticator.clone(),
        projections: projections.clone(),
        submissions: submissions.clone(),
    }));
    let presence_projections: Arc<dyn PresenceProjectionRepository> =
        Arc::new(InMemoryPresenceProjectionRepository::default());
    let presence = Arc::new(PresenceSyncService::new(
        PresenceSyncDependencies {
            authenticator: authenticator.clone(),
            projections: presence_projections.clone(),
            clock: Arc::new(SystemClock),
        },
        PresenceLeasePolicy::new(
            DurationMillis::new(STATUS_LEASE_LIFETIME_MILLIS)
                .map_err(|_| BridgeRuntimeError::presence_policy())?,
            DurationMillis::new(STATUS_ALLOWED_CLOCK_SKEW_MILLIS)
                .map_err(|_| BridgeRuntimeError::presence_policy())?,
        )
        .map_err(|_| BridgeRuntimeError::presence_policy())?,
    ));
    let handoff_content: Arc<dyn HandoffContentGateway> = Arc::new(
        ProjectedHandoffContentGateway::new(projections.clone(), content_reader.clone()),
    );
    Ok(AgentMessageServices {
        sync,
        projections,
        content,
        content_protection,
        outbound_content,
        submissions,
        automation,
        authenticator,
        content_reader,
        handoff_content,
        presence,
        presence_projections,
    })
}

async fn establish_agent_online(
    runtime: &AgentSessionRuntime,
) -> Result<AgentOnlineSession, AgentOnlineFailure> {
    match establish_agent_online_once(runtime).await {
        Err(AgentOnlineFailure::Matrix(failure))
            if failure.kind() == MatrixFailureKind::CryptographicIdentityConflict =>
        {
            recover_matrix_identity(runtime).await?;
            establish_agent_online_once(runtime).await
        }
        result => result,
    }
}

async fn recover_matrix_identity(runtime: &AgentSessionRuntime) -> Result<(), AgentOnlineFailure> {
    if !runtime
        .matrix_identity_recovery
        .prepare_store(runtime.matrix.as_ref())
        .map_err(AgentOnlineFailure::Matrix)?
    {
        return Err(AgentOnlineFailure::Matrix(MatrixFailure::new(
            MatrixOperation::Sync,
            MatrixFailureKind::InvalidResponse,
        )));
    }
    tracing::warn!("检测到 Matrix 设备加密身份冲突，已隔离本地 Store，开始轮换设备会话");
    runtime
        .service
        .recover_matrix_session(&runtime.config)
        .await
        .map_err(AgentOnlineFailure::AgentRuntime)?;
    runtime.matrix_identity_recovery.complete();
    tracing::info!("Matrix 设备会话轮换完成，准备重新建立加密同步");
    Ok(())
}

async fn establish_agent_online_once(
    runtime: &AgentSessionRuntime,
) -> Result<AgentOnlineSession, AgentOnlineFailure> {
    let registered = runtime
        .service
        .ensure_session(&runtime.config)
        .await
        .map_err(AgentOnlineFailure::AgentRuntime)?;
    let lobby = enter_agent_lobby(runtime, &registered).await?;
    let room_id = MatrixRoomId::new(lobby.matrix_room_id().as_str().to_owned())
        .map_err(|_| AgentOnlineFailure::InvalidRoom)?;
    let connection = runtime
        .matrix
        .restore_with_handoffs(registered.matrix_session())
        .await
        .map_err(AgentOnlineFailure::Matrix)?;
    let signer = runtime
        .signing_identities
        .load_or_create()
        .map_err(AgentOnlineFailure::SigningIdentity)?;
    let matrix = connection.matrix_gateway_handle();
    let handoff_transport = connection.handoff_transport_handle();
    let handoff_events = connection.handoff_event_source_handle();
    let status = Arc::new(AgentStatusPublicationHandle::new(
        AgentStatusPublicationService::new(
            AgentStatusPublicationDependencies {
                identity: registered.identity().clone(),
                signer: signer.clone(),
                publisher: Arc::new(MatrixStatusStatePublisher::new(matrix.clone())),
                identifiers: Arc::new(SystemStatusEventIdentifiers),
                clock: Arc::new(SystemClock),
            },
            runtime.status_policy,
        ),
        AgentStatusRoomTarget::new(room_id.clone(), AgentStatusVisibility::Coarse),
        HostAgentState::Available,
    ));
    let publication = Arc::new(MessagePublicationService::new(
        MessagePublicationDependencies {
            identity: registered.identity().clone(),
            signer: signer.clone(),
            publisher: Arc::new(MatrixMessageEventPublisher::new(matrix.clone())),
            content: runtime.outbound_content.clone(),
            submissions: runtime.submissions.clone(),
            automation: runtime.automation.clone(),
            room_catalog_id: lobby.catalog_id(),
        },
    ));
    let handoffs = Arc::new(HandoffReceptionService::new(HandoffReceptionDependencies {
        identity: registered.identity().clone(),
        signer: signer.clone(),
        clock: Arc::new(SystemClock),
        authenticator: runtime.handoffs.authenticator.clone(),
        authorization: runtime.handoffs.authorization.clone(),
        directory: runtime.handoffs.directory.clone(),
        transport: handoff_transport.clone(),
        content: runtime.handoffs.content.clone(),
        store: runtime.handoffs.store.clone(),
    }));
    let handoff_delivery = Arc::new(HandoffDeliveryService::new(HandoffDeliveryDependencies {
        identity: registered.identity().clone(),
        signer,
        clock: Arc::new(SystemClock),
        authorization: runtime.handoffs.authorization.clone(),
        directory: runtime.handoffs.directory.clone(),
        transport: handoff_transport,
        store: runtime.handoffs.store.clone(),
    }));
    let handoff_receipts = compose_handoff_receipts(runtime, registered.identity().clone());
    let handoff_worker =
        spawn_handoff_event_worker(handoff_events, handoffs.clone(), handoff_receipts);
    let (targeted_handoffs, targeted_handoff_worker) = compose_targeted_handoff_runtime(
        runtime,
        TargetedHandoffTarget {
            agent_id: registered.identity().agent_id(),
            instance_id: registered.identity().agent_instance_id(),
        },
    );
    let online = AgentOnlineSession {
        security: connection.security_gateway_handle(),
        room_authority: connection.room_authority_gateway_handle(),
        runtime: registered,
        lobby,
        room_id,
        matrix,
        status,
        publication,
        content_protection: runtime.content_protection.clone(),
        handoffs,
        handoff_delivery,
        handoff_worker,
        targeted_handoffs,
        targeted_handoff_worker,
        presence_projections: runtime.presence_projections.clone(),
        next_batch: stored_sync_cursor(runtime).await,
    };
    complete_agent_online(runtime, online).await
}

/// 接着上次处理完的位置同步。以前每次上线都从头全量同步，只带每个房间最近几十条，
/// 离线期间更早的消息本地永远没有。游标读不出来不挡上线，退回全量同步。
async fn stored_sync_cursor(runtime: &AgentSessionRuntime) -> Option<MatrixSyncToken> {
    runtime
        .messages
        .stored_cursor()
        .await
        .unwrap_or_else(|failure| {
            tracing::warn!(?failure, "读取上次同步游标失败，改为全量同步");
            None
        })
}

/// 首次同步成功后才算上线；随后确保加密身份，失败的同步会停掉已启动的后台任务。
async fn complete_agent_online(
    runtime: &AgentSessionRuntime,
    mut online: AgentOnlineSession,
) -> Result<AgentOnlineSession, AgentOnlineFailure> {
    let mut first = sync_agent_online(runtime, &mut online, true).await;
    if online.next_batch.is_some() && matches!(&first, Err(failure) if stale_sync_cursor(failure)) {
        // 服务端不认上次的游标（比如服务端换过库）：只在这一处退回全量同步，
        // 否则每次重连都会重新读到同一个失效游标。
        tracing::warn!("服务端不认上次的同步游标，改为全量同步");
        online.next_batch = None;
        first = sync_agent_online(runtime, &mut online, true).await;
    }
    if let Err(failure) = first {
        online.stop_workers().await;
        return Err(failure);
    }
    ensure_agent_encryption_identity(online.security.as_ref()).await;
    Ok(online)
}

const fn stale_sync_cursor(failure: &AgentOnlineFailure) -> bool {
    matches!(
        failure,
        AgentOnlineFailure::Matrix(matrix) if matches!(matrix.kind(), MatrixFailureKind::StaleSyncToken)
    )
}

/// Agent 上线时建立自己的加密身份（只在从未建立过时）。别人只把房间密钥发给由主人签名的设备，
/// 所以要在第一条加密消息到来之前签好本机设备。失败不影响上线，发送前还会再试一次；
/// 已有身份但本机缺私钥时不覆盖，交给恢复流程。
async fn ensure_agent_encryption_identity(
    security: &dyn agent_room_bridge_core::matrix_security::MatrixSecurityGateway,
) {
    use agent_room_bridge_core::matrix_security::{MatrixSecurityCommand, MatrixSecurityFailure};
    match security
        .execute(MatrixSecurityCommand::EstablishIdentity)
        .await
    {
        Ok(_) => {}
        Err(MatrixSecurityFailure::RecoveryRequired) => {
            tracing::info!("Agent 已有加密身份但本机缺少私钥，等待恢复后才能在加密房间收发");
        }
        Err(failure) => {
            tracing::warn!(?failure, "Agent 加密身份暂未建立，发送前会再试一次");
        }
    }
}

fn compose_handoff_receipts(
    runtime: &AgentSessionRuntime,
    identity: agent_room_bridge_core::agent_identity::BridgeAgentIdentity,
) -> Arc<HandoffReceiptService> {
    Arc::new(HandoffReceiptService::new(HandoffReceiptDependencies {
        identity,
        clock: Arc::new(SystemClock),
        authenticator: runtime.handoffs.authenticator.clone(),
        store: runtime.handoffs.store.clone(),
    }))
}

fn compose_targeted_handoff_runtime(
    runtime: &AgentSessionRuntime,
    target: TargetedHandoffTarget,
) -> (Arc<TargetedHandoffInboxService>, TargetedHandoffWorker) {
    let service = Arc::new(TargetedHandoffInboxService::new(
        TargetedHandoffInboxDependencies {
            target,
            queue: runtime.handoffs.targeted_queue.clone(),
            inbox: runtime.handoffs.targeted_inbox.clone(),
            content: runtime.handoffs.targeted_content.clone(),
            clock: Arc::new(SystemClock),
        },
    ));
    let worker = spawn_targeted_handoff_worker(service.clone());
    (service, worker)
}

async fn enter_agent_lobby(
    runtime: &AgentSessionRuntime,
    registered: &RegisteredAgentRuntime,
) -> Result<JoinedAgentLobby, AgentOnlineFailure> {
    match runtime
        .lobby
        .enter(registered.identity(), &runtime.lobby_config)
        .await
        .map_err(AgentOnlineFailure::Lobby)?
    {
        ControlPlaneLobbyEntryOutcome::Joined(lobby) => Ok(lobby),
        ControlPlaneLobbyEntryOutcome::ProvisioningBusy { retry_at } => {
            Err(AgentOnlineFailure::ProvisioningBusy(retry_at))
        }
        ControlPlaneLobbyEntryOutcome::CapacityChanged { .. } => {
            Err(AgentOnlineFailure::CapacityChanged)
        }
    }
}

fn spawn_handoff_event_worker(
    events: Arc<dyn EncryptedHandoffToDeviceEventSource>,
    handoffs: Arc<HandoffReceptionService>,
    receipts: Arc<HandoffReceiptService>,
) -> HandoffEventWorker {
    let (terminal_sender, terminal_failure) = watch::channel(None);
    let (shutdown, mut stop) = watch::channel(false);
    let task = tokio::spawn(async move {
        loop {
            if *stop.borrow() {
                return;
            }
            // 只取消等待事件，已经开始验证或持久化的交接必须完成。
            let received = tokio::select! {
                _ = stop.changed() => return,
                received = events.receive() => received,
            };
            let event = match received {
                Ok(event) => event,
                Err(failure) if failure.kind() == HandoffTransportFailureKind::Rejected => {
                    tracing::warn!(
                        failure_kind = ?failure.kind(),
                        "已拒绝不满足加密 To-Device 边界的交接事件"
                    );
                    continue;
                }
                Err(failure) => {
                    let _ = terminal_sender.send(Some(failure.kind()));
                    return;
                }
            };
            match event.event_type().as_str() {
                HANDOFF_REQUEST_EVENT_TYPE => match handoffs.receive(&event).await {
                    Ok(outcome) => {
                        tracing::info!(?outcome, "一次性交接已验证并写入加密本地存储");
                    }
                    Err(failure) => {
                        tracing::warn!(
                            failure_kind = ?failure.kind(),
                            "一次性交接未通过接收验证"
                        );
                    }
                },
                HANDOFF_RECEIPT_EVENT_TYPE => match receipts.apply(&event).await {
                    Ok(outcome) => {
                        tracing::info!(?outcome, "交接回执已验证并推进本地发送状态");
                    }
                    Err(failure) => {
                        tracing::warn!(
                            failure_kind = ?failure.kind(),
                            "交接回执未通过发送侧验证"
                        );
                    }
                },
                unexpected => {
                    tracing::warn!(event_type = unexpected, "忽略未知的加密交接协议事件");
                }
            }
        }
    });
    HandoffEventWorker {
        task,
        shutdown,
        terminal_failure,
    }
}

fn spawn_targeted_handoff_worker(poller: Arc<dyn TargetedHandoffPoller>) -> TargetedHandoffWorker {
    spawn_targeted_handoff_worker_with_policy(
        poller,
        TargetedHandoffPollingPolicy {
            stored: TARGETED_HANDOFF_STORED_DELAY,
            idle: TARGETED_HANDOFF_IDLE_DELAY,
            failure: TARGETED_HANDOFF_FAILURE_DELAY,
        },
    )
}

fn spawn_targeted_handoff_worker_with_policy(
    poller: Arc<dyn TargetedHandoffPoller>,
    policy: TargetedHandoffPollingPolicy,
) -> TargetedHandoffWorker {
    let (shutdown, mut stop) = watch::channel(false);
    let task = tokio::spawn(async move {
        loop {
            if *stop.borrow() {
                return;
            }
            let delay = match poller.claim_once().await {
                Ok(TargetedHandoffClaimOutcome::Stored(handoff)) => {
                    tracing::info!(
                        handoff_id = %handoff.fields().id,
                        "云端定向交接元数据已写入本机收件箱"
                    );
                    policy.stored
                }
                Ok(TargetedHandoffClaimOutcome::Pending(handoff)) => {
                    tracing::debug!(
                        handoff_id = %handoff.fields().id,
                        "本机仍有待处理定向交接，暂停领取下一条云端任务"
                    );
                    policy.idle
                }
                Ok(TargetedHandoffClaimOutcome::Empty) => policy.idle,
                Err(failure) => {
                    tracing::warn!(
                        failure_kind = ?failure.kind(),
                        "云端定向交接轮询暂时失败，将独立重试且不终止 Matrix 会话"
                    );
                    policy.failure
                }
            };
            if *stop.borrow() {
                return;
            }
            tokio::select! {
                _ = stop.changed() => return,
                () = sleep(delay) => {},
            }
        }
    });
    TargetedHandoffWorker { task, shutdown }
}

async fn sync_agent_online(
    runtime: &AgentSessionRuntime,
    online: &mut AgentOnlineSession,
    full_state: bool,
) -> Result<(), AgentOnlineFailure> {
    let request =
        MatrixSyncRequest::new(online.next_batch.clone(), runtime.sync_timeout, full_state)
            .map_err(|_| AgentOnlineFailure::InvalidRoom)?;
    let batch = online
        .matrix
        .sync_once(&request)
        .await
        .map_err(AgentOnlineFailure::Matrix)?;
    let presence = runtime
        .presence
        .process(&batch, full_state)
        .await
        .map_err(AgentOnlineFailure::PresenceSync)?;
    tracing::debug!(
        accepted_statuses = presence.accepted_statuses(),
        membership_changes = presence.membership_changes(),
        isolated_events = presence.issues().len(),
        "Agent Matrix Presence 投影已刷新"
    );
    let outcome = runtime
        .messages
        .process(&batch)
        .await
        .map_err(AgentOnlineFailure::MessageSync)?;
    tracing::debug!(
        accepted_events = outcome.accepted_events,
        isolated_events = outcome.isolated_events,
        timeline_gaps = outcome.timeline_gaps,
        reconciled_submissions = outcome.reconciled_submissions,
        "Agent Matrix 增量同步已持久化"
    );
    // 解不开或来自不受信设备的消息只留下记录；至少在默认的 warn 级别把数量喊出来（不含内容），
    // 否则「Agent 看不到房间里的消息」在日志里也毫无痕迹。
    if outcome.isolated_events > 0 || outcome.timeline_gaps > 0 {
        tracing::warn!(
            isolated_events = outcome.isolated_events,
            timeline_gaps = outcome.timeline_gaps,
            "部分房间消息未能读取：已记为隔离事件或时间线缺口"
        );
    }
    online
        .status
        .renew()
        .await
        .map_err(AgentOnlineFailure::Status)?;
    online.next_batch = Some(batch.next_batch().clone());
    Ok(())
}

async fn establish_initial_session(
    config: &BridgeConfig,
    session_service: &BridgeSessionService,
    authorization_service: BridgeAuthorizationService,
    reconnect_policy: ReconnectPolicy,
) -> Result<Option<ActiveBridgeSession>, BridgeRuntimeError> {
    match session_service.active_session().await {
        Ok(session) => {
            announce_active_session(&session)?;
            Ok(Some(session))
        }
        Err(error) if error.kind() == BridgeSessionFailureKind::NotAuthorized => {
            let authorization = ConfiguredDeviceAuthorization {
                service: authorization_service,
                request: AuthorizeBridgeDevice {
                    label: config.device_label.clone(),
                    platform: current_platform(),
                    profile_import: ProfileImportConsent {
                        display_name: config.import_oidc_profile,
                        locale: config.import_oidc_profile,
                    },
                },
            };
            let authorized = authorize_first_device(
                &authorization,
                &TerminalAuthorizationPrompt,
                reconnect_policy,
                announce_authorization_retry,
            )
            .await?;
            announce_authorized_device(authorized)?;
            let session = session_service
                .active_session()
                .await
                .map_err(BridgeRuntimeError::session)?;
            Ok(Some(session))
        }
        Err(error) if is_reconnectable_session_failure(error) => {
            announce_reconnecting_session()?;
            Ok(None)
        }
        Err(error) => Err(BridgeRuntimeError::session(error)),
    }
}

/// 一次完整的首次设备授权：发现身份服务、申请并展示设备码、等待批准、注册设备。
trait FirstDeviceAuthorization: Send + Sync {
    fn attempt<'a>(
        &'a self,
        prompt: &'a dyn OidcDeviceAuthorizationPromptSink,
    ) -> PortFuture<'a, Result<AuthorizedBridgeDevice, BridgeAuthorizationFailure>>;
}

struct ConfiguredDeviceAuthorization {
    service: BridgeAuthorizationService,
    request: AuthorizeBridgeDevice,
}

impl FirstDeviceAuthorization for ConfiguredDeviceAuthorization {
    fn attempt<'a>(
        &'a self,
        prompt: &'a dyn OidcDeviceAuthorizationPromptSink,
    ) -> PortFuture<'a, Result<AuthorizedBridgeDevice, BridgeAuthorizationFailure>> {
        Box::pin(self.service.authorize(self.request.clone(), prompt))
    }
}

/// 首次授权在拿到设备码之前连不上身份服务或控制面时，进程不退出：按与设备会话重连
/// 相同的退避策略等待后重来，并把原因和下次尝试的时间报告给桌面端。
///
/// 已经展示过设备码的失败照旧返回。重来会申请新码，用户可能正在批准旧码，
/// 所以要由用户显式重试；桌面端对此停在「授权失败」。
async fn authorize_first_device(
    authorization: &impl FirstDeviceAuthorization,
    prompt: &dyn OidcDeviceAuthorizationPromptSink,
    reconnect_policy: ReconnectPolicy,
    announce_retry: impl Fn(
        BridgeAuthorizationFailure,
        DurationMillis,
    ) -> Result<(), BridgeRuntimeError>,
) -> Result<AuthorizedBridgeDevice, BridgeRuntimeError> {
    let mut backoff = ReconnectBackoff::new(reconnect_policy);
    loop {
        let attempt = PromptPresence::new(prompt);
        let failure = match authorization.attempt(&attempt).await {
            Ok(authorized) => return Ok(authorized),
            Err(failure) => failure,
        };
        if attempt.presented() || !is_unreachable_authorization_failure(failure) {
            return Err(BridgeRuntimeError::authorization(failure));
        }
        let delay = backoff.record_failure(retry_entropy());
        tracing::warn!(
            operation = failure.operation(),
            failure_kind = ?failure.kind(),
            consecutive_failures = backoff.consecutive_failures(),
            retry_after_ms = delay.value(),
            "首次设备授权连不上服务，已安排重试"
        );
        announce_retry(failure, delay)?;
        sleep(Duration::from_millis(delay.value())).await;
    }
}

const fn is_unreachable_authorization_failure(failure: BridgeAuthorizationFailure) -> bool {
    matches!(
        failure.kind(),
        BridgeAuthorizationFailureKind::IdentityProviderUnavailable
            | BridgeAuthorizationFailureKind::ControlPlaneUnavailable
    )
}

/// 记录这次尝试有没有把设备码交给用户。
struct PromptPresence<'a> {
    prompt: &'a dyn OidcDeviceAuthorizationPromptSink,
    presented: AtomicBool,
}

impl<'a> PromptPresence<'a> {
    const fn new(prompt: &'a dyn OidcDeviceAuthorizationPromptSink) -> Self {
        Self {
            prompt,
            presented: AtomicBool::new(false),
        }
    }

    fn presented(&self) -> bool {
        self.presented.load(Ordering::Acquire)
    }
}

impl OidcDeviceAuthorizationPromptSink for PromptPresence<'_> {
    fn present(
        &self,
        prompt: &OidcDeviceAuthorizationPrompt,
    ) -> Result<(), OidcDevicePromptFailure> {
        self.presented.store(true, Ordering::Release);
        self.prompt.present(prompt)
    }
}

async fn initialize_matrix(
    config: &BridgeConfig,
    paths: &BridgeRuntimePaths,
    runtime_secrets: &BridgeRuntimeSecrets,
) -> Result<Arc<MatrixSdkClientFactory>, BridgeRuntimeError> {
    let sdk = MatrixSdkConfiguration::new(&config.matrix_homeserver_url, config.request_timeout)
        .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?;
    let store = MatrixSdkStoreConfiguration::encrypted_sqlite(
        paths.matrix_store_root().to_path_buf(),
        runtime_secrets.matrix_store_passphrase().clone(),
    )
    .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?;
    let factory = Arc::new(MatrixSdkClientFactory::with_encrypted_sqlite(sdk, store));
    factory
        .initialize_store()
        .await
        .map_err(BridgeRuntimeError::matrix_store)?;
    Ok(factory)
}

async fn initialize_handoff_store(
    paths: &BridgeRuntimePaths,
    runtime_secrets: &BridgeRuntimeSecrets,
) -> Result<Arc<SqliteHandoffStore>, BridgeRuntimeError> {
    let store = SqliteHandoffStore::open(
        paths.handoff_database(),
        runtime_secrets.handoff_storage_key().clone(),
    )
    .await
    .map_err(|failure| BridgeRuntimeError::handoff_store(&failure))?;
    Ok(Arc::new(store))
}

async fn initialize_targeted_handoff_inbox(
    paths: &BridgeRuntimePaths,
) -> Result<Arc<SqliteTargetedHandoffInbox>, BridgeRuntimeError> {
    let inbox = SqliteTargetedHandoffInbox::open(paths.handoff_database())
        .await
        .map_err(|failure| BridgeRuntimeError::handoff_store(&failure))?;
    Ok(Arc::new(inbox))
}

async fn run_until_shutdown(
    exit: impl Future<Output = io::Result<()>>,
    server: BridgeIpcServer,
    status: Arc<BridgeRuntimeStatus>,
    device_session: DeviceSessionRuntime,
    agent_session: Option<AgentSessionRuntime>,
    host_sessions: Arc<HostSessionRegistry>,
) -> Result<(), BridgeRuntimeError> {
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    let (authorization_lost_sender, mut authorization_lost) = oneshot::channel();
    let mut server_task = tokio::spawn(server.run(shutdown_receiver));
    let mut session_task = tokio::spawn(maintain_sessions(
        device_session,
        agent_session,
        status.clone(),
        authorization_lost_sender,
        shutdown_sender.subscribe(),
    ));

    let janitor_sessions = host_sessions.clone();
    let mut janitor_shutdown = shutdown_sender.subscribe();
    let janitor = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_mins(1));
        loop {
            tokio::select! {
                _ = janitor_shutdown.changed() => break,
                _ = interval.tick() => janitor_sessions.expire_idle().await,
            }
        }
    });
    let result = async { tokio::select! {
        signal = exit => {
            signal.map_err(|_| BridgeRuntimeError::shutdown_signal())?;
            status.mark_shutting_down();
            shutdown_sender
                .send(true)
                .map_err(|_| BridgeRuntimeError::ipc_stopped_early())?;
            let server_result = server_task
                .await
                .map_err(|_| BridgeRuntimeError::ipc_task())?;
            session_task
                .await
                .map_err(|_| BridgeRuntimeError::session_task())?;
            server_result.map_err(BridgeRuntimeError::ipc)
        }
        completed = &mut server_task => {
            status.mark_shutting_down();
            shutdown_sender
                .send(true)
                .map_err(|_| BridgeRuntimeError::session_stopped_early())?;
            session_task
                .await
                .map_err(|_| BridgeRuntimeError::session_task())?;
            completed.map_err(|_| BridgeRuntimeError::ipc_task())?.map_err(BridgeRuntimeError::ipc)
        }
        completed = &mut session_task => {
            status.mark_shutting_down();
            completed.map_err(|_| BridgeRuntimeError::session_task())?;
            shutdown_sender
                .send(true)
                .map_err(|_| BridgeRuntimeError::ipc_stopped_early())?;
            server_task
                .await
                .map_err(|_| BridgeRuntimeError::ipc_task())?
                .map_err(BridgeRuntimeError::ipc)?;
            Err(BridgeRuntimeError::session_stopped_early())
        }
        // 服务端已确认设备凭据不可用、本机凭据也已清除。停在离线态只会让桌面端停机；
        // 整体退出后由监督进程重启，新进程直接进入设备授权。
        Ok(failure) = &mut authorization_lost => {
            status.mark_shutting_down();
            shutdown_sender
                .send(true)
                .map_err(|_| BridgeRuntimeError::ipc_stopped_early())?;
            server_task
                .await
                .map_err(|_| BridgeRuntimeError::ipc_task())?
                .map_err(BridgeRuntimeError::ipc)?;
            session_task
                .await
                .map_err(|_| BridgeRuntimeError::session_task())?;
            Err(BridgeRuntimeError::session(failure))
        }
    } }.await;
    shutdown_sender.send_replace(true);
    host_sessions.shutdown().await;
    janitor
        .await
        .map_err(|_| BridgeRuntimeError::session_task())?;
    result
}

/// 等操作系统关闭信号；桌面托管启动时，监督它的桌面退出也算。
///
/// 桌面被强制结束时来不及结束子进程，Windows 上也没有信号可收。留下的 Bridge 继续占着实例锁和
/// 本机 IPC，下一次启动的桌面既起不了自己的 Bridge，又会把它当成外部 Bridge 接管，而它随时可能退出。
async fn wait_for_exit_signal(exit_with_supervisor: bool) -> io::Result<()> {
    let supervisor_exited = async {
        if exit_with_supervisor {
            input_closed(io::stdin()).await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    tokio::select! {
        result = operating_system_exit_signal() => result,
        () = supervisor_exited => {
            tracing::warn!("监督 Bridge 的桌面已经退出，Bridge 随之有序退出");
            Ok(())
        }
    }
}

async fn operating_system_exit_signal() -> io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            signal = terminate.recv() => signal.ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe)),
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await
}

/// 输入读到结束或读取失败时完成。桌面把标准输入接成自己持有的管道、从不写入，
/// 桌面进程一退出（包括被强制结束）管道就关闭。
///
/// 阻塞读放在独立线程：运行时关闭时会等 `spawn_blocking` 的任务返回，而桌面还在时这次读取不会返回。
fn input_closed(mut input: impl io::Read + Send + 'static) -> impl Future<Output = ()> {
    let (closed, receiver) = oneshot::channel();
    let watcher = std::thread::Builder::new()
        .name("agent-room-supervisor-watch".to_owned())
        .spawn(move || {
            let _ = io::copy(&mut input, &mut io::sink());
            let _ = closed.send(());
        });
    async move {
        // 看不了输入时不因此退出，照旧只响应操作系统关闭信号。
        if watcher.is_err() || receiver.await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

async fn maintain_sessions(
    device: DeviceSessionRuntime,
    agent: Option<AgentSessionRuntime>,
    status: Arc<BridgeRuntimeStatus>,
    authorization_lost: oneshot::Sender<BridgeSessionFailure>,
    shutdown: watch::Receiver<bool>,
) {
    let device_task =
        maintain_device_session(device, status.clone(), authorization_lost, shutdown.clone());
    let agent_task = maintain_agent_session(agent, status, shutdown);
    tokio::join!(device_task, agent_task);
}

async fn maintain_device_session(
    device: DeviceSessionRuntime,
    status: Arc<BridgeRuntimeStatus>,
    authorization_lost: oneshot::Sender<BridgeSessionFailure>,
    mut shutdown: watch::Receiver<bool>,
) {
    let DeviceSessionRuntime {
        service: session_service,
        initial_session: mut session,
        clock,
        refresh_lead_time,
        reconnect_policy,
    } = device;
    let mut backoff = ReconnectBackoff::new(reconnect_policy);
    let mut retry_delay = session
        .is_none()
        .then(|| backoff.record_failure(retry_entropy()));

    loop {
        let wait = retry_delay.take().map_or_else(
            || {
                session
                    .as_ref()
                    .map_or(SessionRefreshPlan::DueNow, |active| {
                        SessionRefreshPlan::calculate(
                            active.access_token_expires_at,
                            clock.now(),
                            refresh_lead_time,
                        )
                    })
            },
            SessionRefreshPlan::After,
        );
        if wait_for_refresh(wait, &mut shutdown).await {
            return;
        }

        status.set_component_ready(BridgeRuntimeStatus::DEVICE_COMPONENT, false);
        match session_service.active_session().await {
            Ok(active) => {
                backoff.record_connected();
                status.set_component_ready(BridgeRuntimeStatus::DEVICE_COMPONENT, true);
                session = Some(active);
            }
            Err(failure) if is_reconnectable_session_failure(failure) => {
                let delay = backoff.record_failure(retry_entropy());
                tracing::warn!(
                    operation = failure.operation(),
                    failure_kind = ?failure.kind(),
                    consecutive_failures = backoff.consecutive_failures(),
                    retry_after_ms = delay.value(),
                    "Bridge 设备会话暂时不可用，已安排重连"
                );
                session = None;
                retry_delay = Some(delay);
            }
            Err(failure) if failure.kind() == BridgeSessionFailureKind::NotAuthorized => {
                tracing::error!(
                    operation = failure.operation(),
                    "Bridge 设备授权已失效，退出后重新授权"
                );
                let _ = authorization_lost.send(failure);
                wait_for_shutdown(&mut shutdown).await;
                return;
            }
            Err(failure) => {
                status.mark_session_failure(failure);
                tracing::error!(
                    operation = failure.operation(),
                    failure_kind = ?failure.kind(),
                    "Bridge 设备会话进入离线态，禁止不安全重试"
                );
                wait_for_shutdown(&mut shutdown).await;
                return;
            }
        }
    }
}

async fn maintain_agent_session(
    runtime: Option<AgentSessionRuntime>,
    status: Arc<BridgeRuntimeStatus>,
    mut shutdown: watch::Receiver<bool>,
) {
    let Some(mut runtime) = runtime else {
        wait_for_shutdown(&mut shutdown).await;
        return;
    };
    let mut backoff = ReconnectBackoff::new(runtime.reconnect_policy);
    let mut online = runtime.initial_session.take();
    let mut retry_delay = online
        .is_none()
        .then(|| backoff.record_failure(retry_entropy()));

    loop {
        if let Some(active) = online.as_mut() {
            let Some(sync) = poll_agent_online(&runtime, active, &mut shutdown).await else {
                active.disconnect().await;
                runtime.state.clear();
                return;
            };
            match sync {
                Ok(()) => backoff.record_connected(),
                Err(failure) if is_reconnectable_agent_online_failure(failure) => {
                    status.set_component_ready(BridgeRuntimeStatus::AGENT_COMPONENT, false);
                    runtime.state.clear();
                    let delay = retry_delay_for_agent_failure(failure, &mut backoff);
                    tracing::warn!(
                        failure_kind = ?failure.kind(),
                        consecutive_failures = backoff.consecutive_failures(),
                        retry_after_ms = delay.value(),
                        "Agent Matrix 会话暂时不可用，已安排完整重连"
                    );
                    active.stop_workers().await;
                    online = None;
                    retry_delay = Some(delay);
                }
                Err(failure) => {
                    status.mark_agent_failure(failure);
                    runtime.state.clear();
                    tracing::error!(
                        failure_kind = ?failure.kind(),
                        "Agent Matrix 会话进入离线态，禁止不安全重试"
                    );
                    active.stop_workers().await;
                    drop(online.take());
                    wait_for_shutdown(&mut shutdown).await;
                    return;
                }
            }
            continue;
        }

        let delay = retry_delay
            .take()
            .unwrap_or_else(|| backoff.record_failure(retry_entropy()));
        if wait_for_refresh(SessionRefreshPlan::After(delay), &mut shutdown).await {
            return;
        }
        match establish_agent_online(&runtime).await {
            Ok(agent_online) => {
                backoff.record_connected();
                status.set_component_ready(BridgeRuntimeStatus::AGENT_COMPONENT, true);
                runtime.state.publish(&agent_online);
                if runtime.report_to_desktop_supervisor
                    && let Err(error) = announce_agent_online(&agent_online)
                {
                    tracing::warn!(error_code = error.code(), "Agent 已上线但终端不可写");
                }
                online = Some(agent_online);
            }
            Err(failure) if is_reconnectable_agent_online_failure(failure) => {
                status.set_component_ready(BridgeRuntimeStatus::AGENT_COMPONENT, false);
                runtime.state.clear();
                let delay = retry_delay_for_agent_failure(failure, &mut backoff);
                if runtime.report_to_desktop_supervisor
                    && let Err(error) = announce_supervisor_diagnostic(failure)
                {
                    tracing::warn!(
                        error_code = error.code(),
                        "Agent 暂时失败诊断无法写入监督通道"
                    );
                }
                tracing::warn!(
                    failure_kind = ?failure.kind(),
                    consecutive_failures = backoff.consecutive_failures(),
                    retry_after_ms = delay.value(),
                    "Agent 上线流程暂时不可用，已安排重连"
                );
                retry_delay = Some(delay);
            }
            Err(failure) => {
                status.mark_agent_failure(failure);
                runtime.state.clear();
                tracing::error!(
                    failure_kind = ?failure.kind(),
                    "Agent 上线流程进入离线态，禁止不安全重试"
                );
                wait_for_shutdown(&mut shutdown).await;
                return;
            }
        }
    }
}

async fn poll_agent_online(
    runtime: &AgentSessionRuntime,
    active: &mut AgentOnlineSession,
    shutdown: &mut watch::Receiver<bool>,
) -> Option<Result<(), AgentOnlineFailure>> {
    let mut handoff_failure = active.handoff_worker.terminal_failure.clone();
    loop {
        if *shutdown.borrow() {
            return None;
        }
        if active.targeted_handoff_worker.is_finished() {
            return Some(Err(AgentOnlineFailure::TargetedHandoffWorker));
        }
        if let Some(failure) = *handoff_failure.borrow() {
            return Some(Err(AgentOnlineFailure::HandoffTransport(failure)));
        }
        let sync = sync_agent_online(runtime, active, false);
        tokio::pin!(sync);
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow_and_update() {
                    // 同步包含设备签名与令牌刷新，关闭不能把共享认证取消在半途。
                    let _ = sync.await;
                    return None;
                }
            }
            result = &mut sync => return Some(result),
            changed = handoff_failure.changed() => {
                let _ = sync.await;
                let failure = if changed.is_ok() {
                    (*handoff_failure.borrow())
                        .unwrap_or(HandoffTransportFailureKind::Internal)
                } else {
                    HandoffTransportFailureKind::Internal
                };
                return Some(Err(AgentOnlineFailure::HandoffTransport(failure)));
            }
        }
    }
}

async fn wait_for_refresh(plan: SessionRefreshPlan, shutdown: &mut watch::Receiver<bool>) -> bool {
    if *shutdown.borrow() {
        return true;
    }
    let SessionRefreshPlan::After(delay) = plan else {
        return false;
    };
    tokio::select! {
        () = sleep(Duration::from_millis(delay.value())) => false,
        changed = shutdown.changed() => {
            changed.is_err() || *shutdown.borrow_and_update()
        }
    }
}

async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    while !*shutdown.borrow_and_update() && shutdown.changed().await.is_ok() {}
}

fn is_reconnectable_session_failure(failure: BridgeSessionFailure) -> bool {
    matches!(
        failure.kind(),
        BridgeSessionFailureKind::ControlPlaneUnavailable
            | BridgeSessionFailureKind::InvalidControlPlaneResponse
            | BridgeSessionFailureKind::SecureStorageUnavailable
            | BridgeSessionFailureKind::RefreshOutcomeUnknown
    )
}

fn is_reconnectable_agent_runtime_failure(failure: AgentRuntimeSessionFailure) -> bool {
    matches!(
        failure.kind(),
        AgentRuntimeSessionFailureKind::NotAuthorized
            | AgentRuntimeSessionFailureKind::ControlPlaneUnavailable
            | AgentRuntimeSessionFailureKind::RegistrationOutcomeUnknown
            | AgentRuntimeSessionFailureKind::SecureStorageUnavailable
    )
}

fn is_reconnectable_agent_online_failure(failure: AgentOnlineFailure) -> bool {
    match failure {
        AgentOnlineFailure::AgentRuntime(failure) => {
            is_reconnectable_agent_runtime_failure(failure)
        }
        AgentOnlineFailure::Lobby(failure) => matches!(
            failure.kind(),
            AgentLobbySessionFailureKind::NotAuthorized
                | AgentLobbySessionFailureKind::Conflict
                | AgentLobbySessionFailureKind::ControlPlaneUnavailable
                | AgentLobbySessionFailureKind::EntryOutcomeUnknown
        ),
        AgentOnlineFailure::ProvisioningBusy(_)
        | AgentOnlineFailure::CapacityChanged
        | AgentOnlineFailure::TargetedHandoffWorker => true,
        AgentOnlineFailure::Matrix(failure) => matches!(
            failure.kind(),
            MatrixFailureKind::RateLimited
                | MatrixFailureKind::Timeout
                | MatrixFailureKind::DependencyUnavailable
                | MatrixFailureKind::CryptographicIdentityConflict
                | MatrixFailureKind::StaleSyncToken
        ),
        AgentOnlineFailure::SigningIdentity(failure) => {
            failure.kind() == BridgeCredentialFailureKind::Unavailable
        }
        AgentOnlineFailure::Status(failure) => reconnectable_status_publication(failure),
        AgentOnlineFailure::PresenceSync(failure) => match failure.kind() {
            PresenceSyncFailureKind::Authentication => failure
                .authentication_failure()
                .is_some_and(|failure| {
                    failure.kind()
                        == agent_room_bridge_core::agent_verification::AgentEventAuthenticationFailureKind::Unavailable
                }),
            PresenceSyncFailureKind::Projection => failure
                .projection_failure()
                .is_some_and(|failure| failure.kind() == PresenceProjectionFailureKind::Unavailable),
        },
        AgentOnlineFailure::MessageSync(failure) => reconnectable_message_sync(failure),
        AgentOnlineFailure::HandoffTransport(failure) => matches!(
            failure,
            HandoffTransportFailureKind::Unavailable
                | HandoffTransportFailureKind::UnknownCommit
                | HandoffTransportFailureKind::Internal
                | HandoffTransportFailureKind::Rejected
        ),
        AgentOnlineFailure::InvalidRoom => false,
    }
}

fn reconnectable_status_publication(failure: StatusPublicationFailure) -> bool {
    match failure.kind() {
        StatusPublicationFailureKind::SigningUnavailable => true,
        StatusPublicationFailureKind::Matrix => failure.matrix_failure().is_some_and(|failure| {
            matches!(
                failure.kind(),
                MatrixFailureKind::Conflict
                    | MatrixFailureKind::RateLimited
                    | MatrixFailureKind::Timeout
                    | MatrixFailureKind::DependencyUnavailable
                    | MatrixFailureKind::UnknownCommit
            )
        }),
        StatusPublicationFailureKind::InvalidConfiguration
        | StatusPublicationFailureKind::InvalidIdentity
        | StatusPublicationFailureKind::InvalidIntent
        | StatusPublicationFailureKind::InvalidIdentifier
        | StatusPublicationFailureKind::Serialization => false,
    }
}

fn reconnectable_message_sync(failure: MessageSyncFailure) -> bool {
    match failure.kind() {
        MessageSyncFailureKind::SubmissionStore => failure
            .submission_store_failure()
            .is_some_and(|failure| failure.kind() == MessageStoreFailureKind::Unavailable),
        MessageSyncFailureKind::Authentication => failure
            .authentication_failure()
            .is_some_and(|failure| failure.kind() == MessageAuthenticationFailureKind::Unavailable),
        MessageSyncFailureKind::ProjectionStore => {
            failure.projection_store_failure().is_some_and(|failure| {
                matches!(
                    failure.kind(),
                    MessageProjectionStoreFailureKind::Unavailable
                        | MessageProjectionStoreFailureKind::Conflict
                )
            })
        }
    }
}

fn retry_delay_for_agent_failure(
    failure: AgentOnlineFailure,
    backoff: &mut ReconnectBackoff,
) -> DurationMillis {
    if let AgentOnlineFailure::ProvisioningBusy(retry_at) = failure {
        let remaining = retry_at.value().saturating_sub(SystemClock.now().value());
        if let Ok(remaining) = u64::try_from(remaining)
            && let Ok(delay) = DurationMillis::new(remaining)
        {
            return delay;
        }
    }
    // Matrix 限流时服务端会给 Retry-After；照它说的等，而不是盲目退避到上限。
    if let AgentOnlineFailure::Matrix(matrix) = failure
        && matrix.kind() == MatrixFailureKind::RateLimited
        && let Some(retry_after) = matrix.retry_after()
    {
        return backoff.record_failure_after(retry_after);
    }
    backoff.record_failure(retry_entropy())
}

fn retry_entropy() -> u64 {
    let mut entropy = [0_u8; size_of::<u64>()];
    if getrandom::fill(&mut entropy).is_ok() {
        return u64::from_le_bytes(entropy);
    }
    tracing::warn!(
        error_code = "bridge.reconnect_entropy_unavailable",
        "重连抖动无法读取系统熵，改用进程内非安全回退值"
    );
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX) ^ u64::from(std::process::id())
}

struct BridgeRuntimeStatus {
    required_components: u8,
    ready_components: AtomicU8,
    starting: AtomicBool,
    fatal: AtomicBool,
    fatal_code: std::sync::OnceLock<&'static str>,
    shutting_down: AtomicBool,
    started_at_unix_ms: i64,
}

impl BridgeRuntimeStatus {
    const DEVICE_COMPONENT: u8 = 1 << 0;
    const AGENT_COMPONENT: u8 = 1 << 1;

    const fn new(started_at_unix_ms: i64, agent_required: bool) -> Self {
        Self {
            required_components: if agent_required {
                Self::DEVICE_COMPONENT | Self::AGENT_COMPONENT
            } else {
                Self::DEVICE_COMPONENT
            },
            ready_components: AtomicU8::new(0),
            starting: AtomicBool::new(true),
            fatal: AtomicBool::new(false),
            fatal_code: std::sync::OnceLock::new(),
            shutting_down: AtomicBool::new(false),
            started_at_unix_ms,
        }
    }

    fn set_component_ready(&self, component: u8, ready: bool) {
        if ready {
            self.ready_components.fetch_or(component, Ordering::AcqRel);
        } else {
            self.ready_components
                .fetch_and(!component, Ordering::AcqRel);
        }
    }

    fn finish_starting(&self) {
        self.starting.store(false, Ordering::Release);
    }

    fn mark_fatal(&self) {
        self.fatal.store(true, Ordering::Release);
    }

    fn mark_agent_failure(&self, failure: AgentOnlineFailure) {
        self.fatal_code
            .get_or_init(|| BridgeRuntimeError::agent_online(failure).code());
    }

    /// 设备会话本身失败：整个 Bridge 离线，并记下原因码让桌面端能说明。
    fn mark_session_failure(&self, failure: BridgeSessionFailure) {
        self.fatal_code
            .get_or_init(|| BridgeRuntimeError::session(failure).code());
        self.mark_fatal();
    }

    fn mark_shutting_down(&self) {
        self.shutting_down.store(true, Ordering::Release);
    }

    fn state(&self) -> IpcBridgeState {
        self.state_for(self.required_components)
    }

    fn state_for(&self, components: u8) -> IpcBridgeState {
        if self.shutting_down.load(Ordering::Acquire) {
            IpcBridgeState::ShuttingDown
        } else if self.fatal.load(Ordering::Acquire)
            || (components & Self::AGENT_COMPONENT != 0 && self.fatal_code.get().is_some())
        {
            IpcBridgeState::Offline
        } else if self.starting.load(Ordering::Acquire) {
            IpcBridgeState::Starting
        } else if self.ready_components.load(Ordering::Acquire) & components == components {
            IpcBridgeState::Ready
        } else {
            IpcBridgeState::Reconnecting
        }
    }
}

// Device connectivity is shared by all host sessions. A legacy/default Agent's
// Matrix failure must not make a healthy device look unable to admit new Agents.
struct DeviceConnectionStatus(Arc<BridgeRuntimeStatus>);

impl BridgeStatusReader for DeviceConnectionStatus {
    fn read_status(&self) -> BridgeStatusSnapshot {
        BridgeStatusSnapshot {
            state: self.0.state_for(BridgeRuntimeStatus::DEVICE_COMPONENT),
            started_at_unix_ms: self.0.started_at_unix_ms,
        }
    }
}

impl BridgeStatusReader for BridgeRuntimeStatus {
    fn failure_code(&self) -> Option<&'static str> {
        self.fatal_code.get().copied()
    }

    fn read_status(&self) -> BridgeStatusSnapshot {
        BridgeStatusSnapshot {
            state: self.state(),
            started_at_unix_ms: self.started_at_unix_ms,
        }
    }
}

fn announce_active_session(session: &ActiveBridgeSession) -> Result<(), BridgeRuntimeError> {
    write_stdout(&format!(
        "Agent Room Bridge 已就绪。\n设备：{}\n访问会话到期时间：{}\n",
        session.device_id,
        session.access_token_expires_at.value()
    ))
}

fn announce_authorized_device(device: AuthorizedBridgeDevice) -> Result<(), BridgeRuntimeError> {
    if supervisor_events_enabled() {
        return write_supervisor_event(&BridgeSupervisorEvent::DeviceAuthorized {
            channel: "agent_room_desktop",
        });
    }
    write_stdout(&format!(
        "设备授权完成，Agent Room Bridge 已就绪。\n设备：{}\n访问会话到期时间：{}\n刷新会话到期时间：{}\n",
        device.device_id,
        device.access_token_expires_at.value(),
        device.refresh_token_expires_at.value()
    ))
}

fn announce_reconnecting_session() -> Result<(), BridgeRuntimeError> {
    write_stdout("Agent Room Bridge 已启动，正在重新连接控制平面。\n")
}

fn announce_agent_online(online: &AgentOnlineSession) -> Result<(), BridgeRuntimeError> {
    write_stdout(&format!(
        "Agent 已进入公共大厅并开始同步。\nAgent：{}\n实例：{}\n大厅分片：{}\n房间：{}\nMatrix 设备：{}\n",
        online.runtime.identity().display_name(),
        online.runtime.identity().agent_instance_id(),
        online.lobby.room_instance_id(),
        online.room_id.as_str(),
        online
            .runtime
            .matrix_session()
            .metadata()
            .device_id()
            .as_str()
    ))
}

struct SystemDeviceRefreshAttempts;

impl DeviceRefreshAttemptIdFactory for SystemDeviceRefreshAttempts {
    fn refresh_attempt_id(&self) -> DeviceRefreshAttemptId {
        DeviceRefreshAttemptId::from_uuid(uuid::Uuid::now_v7())
    }
}

struct SystemAgentRuntimeIdentifiers;

impl AgentRuntimeRequestIdFactory for SystemAgentRuntimeIdentifiers {
    fn registration_request_id(&self) -> AgentInstanceRegistrationRequestId {
        AgentInstanceRegistrationRequestId::from_uuid(uuid::Uuid::now_v7())
    }
}

struct SystemStatusEventIdentifiers;

impl StatusEventIdentifierFactory for SystemStatusEventIdentifiers {
    fn event_id(&self) -> uuid::Uuid {
        uuid::Uuid::now_v7()
    }

    fn correlation_id(&self) -> uuid::Uuid {
        uuid::Uuid::now_v7()
    }
}

fn write_stdout(message: &str) -> Result<(), BridgeRuntimeError> {
    let mut output = io::stdout().lock();
    output
        .write_all(message.as_bytes())
        .and_then(|()| output.flush())
        .map_err(|_| BridgeRuntimeError::terminal())
}

struct TerminalAuthorizationPrompt;

/// 身份服务先重定向验证链接再渲染页面，查询参数里的码到不了批准页；片段会跟着重定向保留下来，
/// 登录主题读它，请用户在批准前与这台电脑上显示的码核对。片段不会发给服务器。
fn verification_destination(prompt: &OidcDeviceAuthorizationPrompt) -> String {
    let base = prompt
        .verification_uri_complete
        .as_deref()
        .unwrap_or(&prompt.verification_uri);
    match url::Url::parse(base) {
        Ok(mut destination) if destination.fragment().is_none() => {
            destination.set_fragment(Some(&format!("user_code={}", prompt.user_code.expose())));
            destination.into()
        }
        _ => base.to_owned(),
    }
}

impl OidcDeviceAuthorizationPromptSink for TerminalAuthorizationPrompt {
    fn present(
        &self,
        prompt: &OidcDeviceAuthorizationPrompt,
    ) -> Result<(), OidcDevicePromptFailure> {
        let destination = verification_destination(prompt);
        if supervisor_events_enabled() {
            return write_supervisor_event(&BridgeSupervisorEvent::AuthorizationRequired {
                channel: "agent_room_desktop",
                verification_uri: &destination,
                user_code: prompt.user_code.expose(),
                expires_in_seconds: prompt.expires_in.value() / 1_000,
            })
            .map_err(|_| OidcDevicePromptFailure);
        }
        let message = format!(
            "请在浏览器打开以下地址完成设备授权：\n{destination}\n设备验证码：{}\n验证码将在 {} 秒后失效。\n",
            prompt.user_code.expose(),
            prompt.expires_in.value() / 1_000
        );
        let mut output = io::stdout().lock();
        output
            .write_all(message.as_bytes())
            .and_then(|()| output.flush())
            .map_err(|_| OidcDevicePromptFailure)
    }
}

#[derive(Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum BridgeSupervisorEvent<'a> {
    AuthorizationRequired {
        channel: &'static str,
        #[serde(rename = "verificationUri")]
        verification_uri: &'a str,
        #[serde(rename = "userCode")]
        user_code: &'a str,
        #[serde(rename = "expiresInSeconds")]
        expires_in_seconds: u64,
    },
    DeviceAuthorized {
        channel: &'static str,
    },
    Ready {
        channel: &'static str,
    },
    TransientFailure {
        channel: &'static str,
        code: &'static str,
    },
    /// 首次授权连不上服务，Bridge 保持运行并将在 `retryAfterMs` 后重试。
    ServerUnreachable {
        channel: &'static str,
        code: &'static str,
        #[serde(rename = "retryAfterMs")]
        retry_after_ms: u64,
    },
}

fn announce_authorization_retry(
    failure: BridgeAuthorizationFailure,
    delay: DurationMillis,
) -> Result<(), BridgeRuntimeError> {
    let reason = BridgeRuntimeError::authorization(failure);
    if supervisor_events_enabled() {
        return write_supervisor_event(&BridgeSupervisorEvent::ServerUnreachable {
            channel: "agent_room_desktop",
            code: reason.code(),
            retry_after_ms: delay.value(),
        });
    }
    write_stdout(&format!(
        "{reason}，{} 秒后重试设备授权。\n",
        delay.value().div_ceil(1_000)
    ))
}

fn announce_supervisor_ready() -> Result<(), BridgeRuntimeError> {
    if !supervisor_events_enabled() {
        return Ok(());
    }
    write_supervisor_event(&BridgeSupervisorEvent::Ready {
        channel: "agent_room_desktop",
    })
}

fn announce_supervisor_diagnostic(failure: AgentOnlineFailure) -> Result<(), BridgeRuntimeError> {
    if !supervisor_events_enabled() {
        return Ok(());
    }
    let mapped = BridgeRuntimeError::agent_online(failure);
    write_supervisor_event(&BridgeSupervisorEvent::TransientFailure {
        channel: "agent_room_desktop",
        code: mapped.code(),
    })
}

fn supervisor_events_enabled() -> bool {
    std::env::var("AGENT_ROOM_BRIDGE_SUPERVISED").is_ok_and(|value| value == "true")
}

fn write_supervisor_event(event: &BridgeSupervisorEvent<'_>) -> Result<(), BridgeRuntimeError> {
    let mut serialized = serde_json::to_string(event)
        .map_err(|_| BridgeRuntimeError::configuration("桌面监督事件编码失败".to_owned()))?;
    serialized.push('\n');
    write_stdout(&serialized)
}

struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> UtcMillis {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时钟不得早于 Unix epoch");
        let milliseconds = i64::try_from(elapsed.as_millis()).expect("系统时间不得超出 i64");
        UtcMillis::new(milliseconds).expect("当前系统时间必须有效")
    }
}

fn current_platform() -> DevicePlatform {
    #[cfg(target_os = "windows")]
    return DevicePlatform::Windows;
    #[cfg(target_os = "macos")]
    return DevicePlatform::MacOs;
    #[cfg(target_os = "linux")]
    return DevicePlatform::Linux;
    #[allow(unreachable_code)]
    DevicePlatform::Web
}

fn domain_duration(duration: Duration) -> Result<DurationMillis, BridgeRuntimeError> {
    let milliseconds = u64::try_from(duration.as_millis())
        .map_err(|_| BridgeRuntimeError::configuration("Bridge 时限超出可表示范围".to_owned()))?;
    DurationMillis::new(milliseconds)
        .map_err(|_| BridgeRuntimeError::configuration("Bridge 时限必须大于零".to_owned()))
}

#[derive(Debug)]
pub(crate) struct BridgeRuntimeError {
    code: &'static str,
    message: String,
}

impl BridgeRuntimeError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn configuration(message: String) -> Self {
        Self::new("bridge.invalid_configuration", message)
    }

    fn status_policy() -> Self {
        Self::new("bridge.status_policy_invalid", "Agent 状态租约策略无效")
    }

    fn presence_policy() -> Self {
        Self::new(
            "bridge.presence_policy_invalid",
            "Agent Presence 接收租约策略无效",
        )
    }

    fn terminal() -> Self {
        Self::new("bridge.terminal_unavailable", "无法写入当前终端")
    }

    fn shutdown_signal() -> Self {
        Self::new("bridge.shutdown_signal_failed", "无法监听操作系统关闭信号")
    }

    fn ipc_stopped_early() -> Self {
        Self::new("bridge.ipc_stopped_early", "本地 IPC 服务已提前停止")
    }

    fn ipc_task() -> Self {
        Self::new("bridge.ipc_task_failed", "本地 IPC 服务任务异常终止")
    }

    fn session_task() -> Self {
        Self::new(
            "bridge.session_task_failed",
            "Bridge 设备或 Agent 在线会话维护任务异常终止",
        )
    }

    fn session_stopped_early() -> Self {
        Self::new(
            "bridge.session_stopped_early",
            "Bridge 设备会话维护任务已提前停止",
        )
    }

    fn runtime_secrets(failure: BridgeCredentialFailure) -> Self {
        match failure.kind() {
            BridgeCredentialFailureKind::Unavailable => Self::new(
                "bridge.runtime_secrets_unavailable",
                "Bridge 运行时秘密无法从操作系统安全存储读取",
            ),
            BridgeCredentialFailureKind::Corrupt => Self::new(
                "bridge.runtime_secrets_corrupt",
                "Bridge 运行时秘密已损坏，拒绝静默替换",
            ),
        }
    }

    fn ipc(failure: BridgeIpcFailure) -> Self {
        let (code, message) = match failure.kind() {
            BridgeIpcFailureKind::InvalidEndpoint => {
                ("bridge.ipc_endpoint_invalid", "本地 IPC 端点名称无效")
            }
            #[cfg(windows)]
            BridgeIpcFailureKind::AccessControl => (
                "bridge.ipc_access_control_failed",
                "无法把本地 IPC 限制到当前登录会话",
            ),
            BridgeIpcFailureKind::Bind => ("bridge.ipc_bind_failed", "无法创建本地 IPC 监听器"),
            BridgeIpcFailureKind::Accept => ("bridge.ipc_accept_failed", "本地 IPC 监听器失效"),
            BridgeIpcFailureKind::Protocol
            | BridgeIpcFailureKind::Handshake
            | BridgeIpcFailureKind::Authentication
            | BridgeIpcFailureKind::Timeout => ("bridge.ipc_session_failed", "本地 IPC 会话失败"),
            BridgeIpcFailureKind::Entropy => (
                "bridge.ipc_entropy_unavailable",
                "操作系统随机数生成器不可用",
            ),
            BridgeIpcFailureKind::Internal => ("bridge.ipc_internal", "本地 IPC 服务发生内部错误"),
        };
        Self::new(code, message)
    }

    fn runtime_files(failure: BridgeRuntimeFileFailure) -> Self {
        match failure.kind() {
            BridgeRuntimeFileFailureKind::InvalidPath => {
                Self::new("bridge.runtime_path_invalid", "Bridge 运行目录无效")
            }
            #[cfg(unix)]
            BridgeRuntimeFileFailureKind::InsecurePermissions => Self::new(
                "bridge.runtime_permissions_insecure",
                "Bridge 运行目录或锁文件权限过宽",
            ),
            BridgeRuntimeFileFailureKind::AlreadyHeld => {
                Self::new("bridge.runtime_lock_held", "Bridge 运行锁已被占用")
            }
            BridgeRuntimeFileFailureKind::Io => {
                Self::new("bridge.runtime_io_failed", "Bridge 运行目录或锁文件不可用")
            }
        }
    }

    fn instance_lock(failure: BridgeRuntimeFileFailure) -> Self {
        if failure.kind() == BridgeRuntimeFileFailureKind::AlreadyHeld {
            return Self::new(
                "bridge.already_running",
                "另一个 Agent Room Bridge 进程已经运行",
            );
        }
        Self::runtime_files(failure)
    }

    fn matrix_store_lock(failure: BridgeRuntimeFileFailure) -> Self {
        if failure.kind() == BridgeRuntimeFileFailureKind::AlreadyHeld {
            return Self::new(
                "bridge.matrix_store_locked",
                "Matrix Store 已由另一个进程占用",
            );
        }
        Self::runtime_files(failure)
    }

    fn matrix_store(failure: MatrixFailure) -> Self {
        if failure.kind() == MatrixFailureKind::InvalidConfiguration {
            return Self::new(
                "bridge.matrix_store_configuration_invalid",
                "Matrix Store 配置无效",
            );
        }
        Self::new(
            "bridge.matrix_store_unavailable",
            "无法创建、打开或解密 Matrix Store；拒绝使用临时内存存储继续运行",
        )
    }

    fn handoff_store(
        failure: &agent_room_bridge_storage_adapter::SqliteBridgeStorageOpenFailure,
    ) -> Self {
        match failure {
            agent_room_bridge_storage_adapter::SqliteBridgeStorageOpenFailure::CreateDirectory(
                _,
            ) => Self::new(
                "bridge.handoff_store_directory_unavailable",
                "无法创建加密的一次性上下文存储目录",
            ),
            agent_room_bridge_storage_adapter::SqliteBridgeStorageOpenFailure::Connect(_) => {
                Self::new(
                    "bridge.handoff_store_unavailable",
                    "无法打开加密的一次性上下文存储",
                )
            }
            agent_room_bridge_storage_adapter::SqliteBridgeStorageOpenFailure::Migrate(_) => {
                Self::new(
                    "bridge.handoff_store_migration_failed",
                    "无法迁移加密的一次性上下文存储",
                )
            }
        }
    }

    fn message_store(
        failure: &agent_room_bridge_storage_adapter::SqliteBridgeStorageOpenFailure,
    ) -> Self {
        match failure {
            agent_room_bridge_storage_adapter::SqliteBridgeStorageOpenFailure::CreateDirectory(
                _,
            ) => Self::new(
                "bridge.message_store_directory_unavailable",
                "无法创建消息投影存储目录",
            ),
            agent_room_bridge_storage_adapter::SqliteBridgeStorageOpenFailure::Connect(_) => {
                Self::new("bridge.message_store_unavailable", "无法打开消息投影存储")
            }
            agent_room_bridge_storage_adapter::SqliteBridgeStorageOpenFailure::Migrate(_) => {
                Self::new(
                    "bridge.message_store_migration_failed",
                    "无法迁移消息投影存储",
                )
            }
        }
    }

    fn authorization(failure: BridgeAuthorizationFailure) -> Self {
        let (code, message) = match failure.kind() {
            BridgeAuthorizationFailureKind::InvalidRequest => {
                ("bridge.authorization_invalid", "设备授权请求无效")
            }
            BridgeAuthorizationFailureKind::AuthorizationDenied => {
                ("bridge.authorization_denied", "身份提供方拒绝了设备授权")
            }
            BridgeAuthorizationFailureKind::AuthorizationExpired => (
                "bridge.authorization_expired",
                "设备验证码已过期；请重新启动 Bridge 获取新的验证码",
            ),
            // 授权指引只经标准输出交给用户或桌面监督进程，展示失败就是终端不可写。
            BridgeAuthorizationFailureKind::AuthorizationPromptUnavailable => {
                ("bridge.terminal_unavailable", "无法写入当前终端")
            }
            BridgeAuthorizationFailureKind::IdentityProviderUnavailable => (
                "bridge.identity_provider_unavailable",
                "身份提供方暂时不可用",
            ),
            BridgeAuthorizationFailureKind::InvalidIdentityAssertion => (
                "bridge.identity_assertion_invalid",
                "身份声明无法通过安全校验",
            ),
            BridgeAuthorizationFailureKind::SecureStorageUnavailable => (
                "bridge.secure_storage_unavailable",
                "操作系统安全存储不可用",
            ),
            BridgeAuthorizationFailureKind::CorruptSecureStorage => {
                ("bridge.secure_storage_corrupt", "操作系统安全存储内容损坏")
            }
            BridgeAuthorizationFailureKind::ControlPlaneConflict => {
                ("bridge.control_plane_conflict", "设备注册状态发生冲突")
            }
            BridgeAuthorizationFailureKind::ControlPlaneUnavailable => {
                ("bridge.control_plane_unavailable", "控制平面暂时不可用")
            }
            BridgeAuthorizationFailureKind::UnknownCommit => (
                "bridge.registration_outcome_unknown",
                "设备注册结果未知；请先在设备管理页确认状态，避免重复授权",
            ),
            BridgeAuthorizationFailureKind::Internal => {
                ("bridge.authorization_internal", "设备授权发生内部错误")
            }
        };
        Self::new(code, message)
    }

    fn session(failure: BridgeSessionFailure) -> Self {
        let (code, message) = match failure.kind() {
            BridgeSessionFailureKind::NotAuthorized => (
                "bridge.not_authorized",
                "设备尚未授权或授权已失效；重新启动 Bridge 后会进入设备授权",
            ),
            BridgeSessionFailureKind::RefreshOutcomeUnknown => (
                "bridge.refresh_outcome_unknown",
                "设备会话刷新结果未知；Bridge 会用同一刷新尝试自动重试，无需重新授权",
            ),
            BridgeSessionFailureKind::SecureStorageUnavailable => (
                "bridge.secure_storage_unavailable",
                "操作系统安全存储不可用",
            ),
            BridgeSessionFailureKind::CorruptSecureStorage => {
                ("bridge.secure_storage_corrupt", "操作系统安全存储内容损坏")
            }
            BridgeSessionFailureKind::ControlPlaneUnavailable => {
                ("bridge.control_plane_unavailable", "控制平面暂时不可用")
            }
            BridgeSessionFailureKind::InvalidControlPlaneResponse => (
                "bridge.control_plane_response_invalid",
                "控制平面返回了无法通过安全校验的响应",
            ),
            BridgeSessionFailureKind::Internal => {
                ("bridge.session_internal", "Bridge 会话初始化失败")
            }
        };
        Self::new(code, message)
    }

    fn agent_runtime(failure: AgentRuntimeSessionFailure) -> Self {
        let (code, message) = match failure.kind() {
            AgentRuntimeSessionFailureKind::InvalidConfiguration => (
                "bridge.agent_runtime_configuration_invalid",
                "Agent 运行时配置无效",
            ),
            AgentRuntimeSessionFailureKind::ConfigurationConflict => (
                "bridge.agent_runtime_configuration_conflict",
                "Agent 运行时配置与已持久化身份冲突；拒绝静默切换身份",
            ),
            AgentRuntimeSessionFailureKind::NotAuthorized => (
                "bridge.agent_runtime_not_authorized",
                "当前 Bridge 设备无权登记 Agent 实例",
            ),
            AgentRuntimeSessionFailureKind::Forbidden => (
                "bridge.agent_runtime_forbidden",
                "当前账户不是该 Agent 的 Owner 或 Operator",
            ),
            AgentRuntimeSessionFailureKind::NotFound => (
                "bridge.agent_runtime_agent_not_found",
                "配置的 Agent 不存在",
            ),
            AgentRuntimeSessionFailureKind::Conflict => (
                "bridge.agent_runtime_registration_conflict",
                "Agent 实例登记幂等键与既有请求冲突",
            ),
            AgentRuntimeSessionFailureKind::ControlPlaneUnavailable => (
                "bridge.agent_runtime_control_plane_unavailable",
                "Agent 实例登记控制面暂时不可用",
            ),
            AgentRuntimeSessionFailureKind::RegistrationOutcomeUnknown => (
                "bridge.agent_runtime_registration_unknown",
                "Agent 实例登记结果未知；已保留原幂等键等待安全重试",
            ),
            AgentRuntimeSessionFailureKind::InvalidControlPlaneResponse => (
                "bridge.agent_runtime_response_invalid",
                "控制面返回的 Agent 身份或 Matrix 会话无效",
            ),
            AgentRuntimeSessionFailureKind::SecureStorageUnavailable => (
                "bridge.agent_runtime_storage_unavailable",
                "Agent 实例凭据无法从操作系统安全存储读取",
            ),
            AgentRuntimeSessionFailureKind::CorruptSecureStorage => (
                "bridge.agent_runtime_storage_corrupt",
                "Agent 实例凭据已损坏，拒绝静默重建身份",
            ),
            AgentRuntimeSessionFailureKind::Internal => {
                ("bridge.agent_runtime_internal", "Agent 运行时初始化失败")
            }
        };
        Self::new(code, message)
    }

    fn agent_online(failure: AgentOnlineFailure) -> Self {
        match failure {
            AgentOnlineFailure::AgentRuntime(failure) => Self::agent_runtime(failure),
            AgentOnlineFailure::Lobby(failure) => Self::agent_lobby(failure),
            AgentOnlineFailure::ProvisioningBusy(_) => {
                Self::new("bridge.lobby_provisioning_busy", "公共大厅正在创建新分片")
            }
            AgentOnlineFailure::CapacityChanged => Self::new(
                "bridge.lobby_capacity_changed",
                "公共大厅容量在分配期间发生变化",
            ),
            AgentOnlineFailure::Matrix(failure) => Self::agent_matrix(failure),
            AgentOnlineFailure::SigningIdentity(failure) => match failure.kind() {
                BridgeCredentialFailureKind::Unavailable => Self::new(
                    "bridge.agent_signing_identity_unavailable",
                    "Agent 实例签名密钥暂时不可用",
                ),
                BridgeCredentialFailureKind::Corrupt => Self::new(
                    "bridge.agent_signing_identity_corrupt",
                    "Agent 实例签名密钥已损坏，拒绝静默替换",
                ),
            },
            AgentOnlineFailure::Status(failure) => Self::new(
                "bridge.agent_status_publication_failed",
                format!("Agent 状态无法安全发布：{:?}", failure.kind()),
            ),
            AgentOnlineFailure::PresenceSync(failure) => Self::new(
                "bridge.presence_sync_failed",
                format!("Agent Presence 无法安全同步：{:?}", failure.kind()),
            ),
            AgentOnlineFailure::MessageSync(failure) => Self::new(
                "bridge.message_sync_failed",
                format!("消息增量同步无法安全持久化：{:?}", failure.kind()),
            ),
            AgentOnlineFailure::HandoffTransport(failure) => Self::new(
                "bridge.handoff_transport_failed",
                format!("加密交接收件通道已经终止：{failure:?}"),
            ),
            AgentOnlineFailure::TargetedHandoffWorker => Self::new(
                "bridge.targeted_handoff_worker_stopped",
                "云端定向交接轮询任务意外终止",
            ),
            AgentOnlineFailure::InvalidRoom => Self::new(
                "bridge.lobby_room_invalid",
                "控制面返回的大厅 Matrix 房间标识无效",
            ),
        }
    }

    fn agent_lobby(failure: AgentLobbySessionFailure) -> Self {
        let (code, message) = match failure.kind() {
            AgentLobbySessionFailureKind::InvalidRequest => {
                ("bridge.lobby_request_invalid", "自动大厅配置或请求无效")
            }
            AgentLobbySessionFailureKind::NotAuthorized => (
                "bridge.lobby_not_authorized",
                "当前 Bridge 设备尚未获准让 Agent 进入大厅",
            ),
            AgentLobbySessionFailureKind::Forbidden => (
                "bridge.lobby_forbidden",
                "当前设备与 Agent 实例的权威绑定不匹配",
            ),
            AgentLobbySessionFailureKind::NotFound => (
                "bridge.lobby_not_found",
                "配置的公共大厅或 Agent 实例不存在",
            ),
            AgentLobbySessionFailureKind::Conflict => {
                ("bridge.lobby_conflict", "公共大厅分配状态发生冲突")
            }
            AgentLobbySessionFailureKind::ControlPlaneUnavailable => (
                "bridge.lobby_control_plane_unavailable",
                "公共大厅控制面暂时不可用",
            ),
            AgentLobbySessionFailureKind::EntryOutcomeUnknown => (
                "bridge.lobby_entry_unknown",
                "公共大厅加入结果未知，必须先重新对账",
            ),
            AgentLobbySessionFailureKind::InvalidControlPlaneResponse => (
                "bridge.lobby_response_invalid",
                "公共大厅控制面返回了错配或畸形响应",
            ),
            AgentLobbySessionFailureKind::Internal => {
                ("bridge.lobby_internal", "公共大厅加入流程发生内部错误")
            }
        };
        Self::new(code, message)
    }

    fn agent_matrix(failure: MatrixFailure) -> Self {
        let (code, message) = match failure.kind() {
            MatrixFailureKind::Unauthenticated | MatrixFailureKind::AuthenticationRejected => (
                "bridge.matrix_session_rejected",
                "Agent Matrix 设备会话已被拒绝",
            ),
            MatrixFailureKind::Forbidden => (
                "bridge.matrix_room_forbidden",
                "Agent Matrix 身份无权同步已分配大厅",
            ),
            MatrixFailureKind::InvalidConfiguration
            | MatrixFailureKind::InvalidResponse
            | MatrixFailureKind::UnsupportedVersion => (
                "bridge.matrix_response_invalid",
                "Matrix 配置或响应无法通过安全校验",
            ),
            MatrixFailureKind::NotFound => (
                "bridge.matrix_room_not_found",
                "控制面分配的 Matrix 房间不存在",
            ),
            MatrixFailureKind::Conflict => {
                ("bridge.matrix_conflict", "Agent Matrix 房间状态发生冲突")
            }
            MatrixFailureKind::RateLimited => {
                ("bridge.matrix_rate_limited", "Agent Matrix 请求已被限流")
            }
            MatrixFailureKind::Timeout => ("bridge.matrix_timeout", "Agent Matrix 请求超时"),
            MatrixFailureKind::DependencyUnavailable => match failure.operation() {
                MatrixOperation::RestoreSession => (
                    "bridge.matrix_restore_dependency_unavailable",
                    "Agent Matrix 会话恢复依赖暂时不可用",
                ),
                MatrixOperation::Sync => (
                    "bridge.matrix_sync_dependency_unavailable",
                    "Agent Matrix 同步依赖暂时不可用",
                ),
                _ => (
                    "bridge.matrix_dependency_unavailable",
                    "Agent Matrix 服务或本地依赖暂时不可用",
                ),
            },
            MatrixFailureKind::CryptographicIdentityConflict => (
                "bridge.matrix_crypto_identity_conflict",
                "Agent Matrix 设备加密身份发生冲突",
            ),
            MatrixFailureKind::StaleSyncToken => (
                "bridge.matrix_sync_token_stale",
                "Agent Matrix 同步游标已失效",
            ),
            MatrixFailureKind::UnknownCommit => (
                "bridge.matrix_outcome_unknown",
                "Matrix 操作结果未知，必须先对账",
            ),
        };
        Self::new(code, message)
    }

    pub(crate) const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for BridgeRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for BridgeRuntimeError {}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        },
        time::Duration,
    };

    use agent_room_application::ports::{
        MatrixFailure, MatrixFailureKind, MatrixOperation, PortFuture,
    };
    use agent_room_bridge_core::handoffs::{
        TargetedHandoffClaimOutcome, TargetedHandoffInboxServiceFailure,
    };
    use agent_room_bridge_ipc::IpcBridgeState;
    use tokio::sync::Notify;

    use super::{
        AgentOnlineFailure, BridgeRuntimeError, BridgeRuntimeStatus, BridgeStatusReader,
        TargetedHandoffPoller, TargetedHandoffPollingPolicy, input_closed,
        is_reconnectable_agent_online_failure, spawn_targeted_handoff_worker_with_policy,
        verification_destination,
    };

    #[test]
    fn verification_link_carries_the_code_in_a_fragment_that_survives_the_redirect() {
        use agent_room_application::ports::OidcDeviceAuthorizationPrompt;
        use agent_room_domain::time::DurationMillis;

        let prompt = |complete: Option<&str>| OidcDeviceAuthorizationPrompt {
            user_code: agent_room_application::ports::SecretValue::new("ZUNG-KBGI".to_owned())
                .expect("验证码有效"),
            verification_uri: "https://id.example/realms/agent-room/device".to_owned(),
            verification_uri_complete: complete.map(str::to_owned),
            expires_in: DurationMillis::new(600_000).expect("时长有效"),
            polling_interval: DurationMillis::new(5_000).expect("时长有效"),
        };
        assert_eq!(
            verification_destination(&prompt(Some(
                "https://id.example/realms/agent-room/device?user_code=ZUNG-KBGI"
            ))),
            "https://id.example/realms/agent-room/device?user_code=ZUNG-KBGI#user_code=ZUNG-KBGI"
        );
        assert_eq!(
            verification_destination(&prompt(None)),
            "https://id.example/realms/agent-room/device#user_code=ZUNG-KBGI"
        );
        assert_eq!(
            verification_destination(&prompt(Some("https://id.example/device#kept"))),
            "https://id.example/device#kept"
        );
    }

    #[test]
    fn device_status_remains_ready_when_default_character_reconnects_or_fails() {
        let runtime = Arc::new(BridgeRuntimeStatus::new(1_000, true));
        let device = super::DeviceConnectionStatus(runtime.clone());
        runtime.set_component_ready(BridgeRuntimeStatus::DEVICE_COMPONENT, true);
        runtime.finish_starting();
        assert_eq!(runtime.read_status().state, IpcBridgeState::Reconnecting);
        assert_eq!(device.read_status().state, IpcBridgeState::Ready);
        runtime.mark_agent_failure(AgentOnlineFailure::InvalidRoom);
        assert_eq!(runtime.read_status().state, IpcBridgeState::Offline);
        assert_eq!(device.read_status().state, IpcBridgeState::Ready);
        runtime.set_component_ready(BridgeRuntimeStatus::DEVICE_COMPONENT, false);
        assert_eq!(device.read_status().state, IpcBridgeState::Reconnecting);
        runtime.mark_fatal();
        assert_eq!(device.read_status().state, IpcBridgeState::Offline);
    }

    #[test]
    fn matrix_依赖故障保留恢复与同步的操作维度() {
        let restore = BridgeRuntimeError::agent_matrix(MatrixFailure::new(
            MatrixOperation::RestoreSession,
            MatrixFailureKind::DependencyUnavailable,
        ));
        let sync = BridgeRuntimeError::agent_matrix(MatrixFailure::new(
            MatrixOperation::Sync,
            MatrixFailureKind::DependencyUnavailable,
        ));

        assert_eq!(
            restore.code(),
            "bridge.matrix_restore_dependency_unavailable"
        );
        assert_eq!(sync.code(), "bridge.matrix_sync_dependency_unavailable");
    }

    #[test]
    fn 在线阶段发现加密身份冲突后允许释放连接并进入一次性恢复() {
        let failure = AgentOnlineFailure::Matrix(MatrixFailure::new(
            MatrixOperation::Sync,
            MatrixFailureKind::CryptographicIdentityConflict,
        ));

        assert!(is_reconnectable_agent_online_failure(failure));
    }

    #[test]
    fn bridge_只有所有必需组件就绪时才报告_ready() {
        let status = BridgeRuntimeStatus::new(1_000, true);
        assert_eq!(
            status.read_status().state,
            IpcBridgeState::Starting,
            "组合根完成前不得提前报就绪"
        );

        status.finish_starting();
        status.set_component_ready(BridgeRuntimeStatus::DEVICE_COMPONENT, true);
        assert_eq!(status.read_status().state, IpcBridgeState::Reconnecting);

        status.set_component_ready(BridgeRuntimeStatus::AGENT_COMPONENT, true);
        assert_eq!(status.read_status().state, IpcBridgeState::Ready);

        status.set_component_ready(BridgeRuntimeStatus::DEVICE_COMPONENT, false);
        assert_eq!(status.read_status().state, IpcBridgeState::Reconnecting);
    }

    #[test]
    fn bridge_致命失败与关闭状态覆盖组件就绪() {
        let status = BridgeRuntimeStatus::new(1_000, false);
        status.set_component_ready(BridgeRuntimeStatus::DEVICE_COMPONENT, true);
        status.finish_starting();
        assert_eq!(status.read_status().state, IpcBridgeState::Ready);

        status.mark_fatal();
        assert_eq!(status.read_status().state, IpcBridgeState::Offline);

        status.mark_shutting_down();
        assert_eq!(status.read_status().state, IpcBridgeState::ShuttingDown);
    }

    #[test]
    fn 人物永久失败保留原始错误码且不覆盖其他运行时() {
        let desktop = BridgeRuntimeStatus::new(1_000, false);
        desktop.set_component_ready(BridgeRuntimeStatus::DEVICE_COMPONENT, true);
        desktop.finish_starting();
        let character = BridgeRuntimeStatus::new(1_000, true);
        character.mark_agent_failure(AgentOnlineFailure::InvalidRoom);
        assert_eq!(character.read_status().state, IpcBridgeState::Offline);
        assert_eq!(
            character.failure_code(),
            Some(BridgeRuntimeError::agent_online(AgentOnlineFailure::InvalidRoom).code())
        );
        assert_eq!(desktop.read_status().state, IpcBridgeState::Ready);
        assert_eq!(desktop.failure_code(), None);
    }

    #[tokio::test]
    async fn 云端交接轮询器随在线会话销毁而终止() {
        let poller = Arc::new(计数交接轮询器::default());
        let worker = spawn_targeted_handoff_worker_with_policy(
            poller.clone(),
            TargetedHandoffPollingPolicy {
                stored: Duration::from_millis(5),
                idle: Duration::from_millis(5),
                failure: Duration::from_millis(5),
            },
        );
        tokio::time::timeout(Duration::from_secs(1), poller.first_claim.notified())
            .await
            .expect("轮询器应及时启动");
        tokio::time::sleep(Duration::from_millis(20)).await;

        drop(worker);
        tokio::task::yield_now().await;
        let stopped_at = poller.claims.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert_eq!(
            poller.claims.load(Ordering::SeqCst),
            stopped_at,
            "在线会话销毁后不得继续轮询云端"
        );
    }

    #[derive(Default)]
    struct 计数交接轮询器 {
        claims: AtomicU32,
        first_claim: Notify,
    }

    #[derive(Default)]
    struct 在途交接轮询器 {
        entered: Notify,
        release: Notify,
        completed: AtomicU32,
    }

    impl TargetedHandoffPoller for 在途交接轮询器 {
        fn claim_once(
            &self,
        ) -> PortFuture<'_, Result<TargetedHandoffClaimOutcome, TargetedHandoffInboxServiceFailure>>
        {
            Box::pin(async move {
                self.entered.notify_one();
                self.release.notified().await;
                self.completed.fetch_add(1, Ordering::SeqCst);
                Ok(TargetedHandoffClaimOutcome::Empty)
            })
        }
    }

    #[tokio::test]
    async fn 正常关闭等待已发出的交接请求完成且不再领取下一条() {
        let poller = Arc::new(在途交接轮询器::default());
        let mut worker = spawn_targeted_handoff_worker_with_policy(
            poller.clone(),
            TargetedHandoffPollingPolicy {
                stored: Duration::from_millis(1),
                idle: Duration::from_millis(1),
                failure: Duration::from_millis(1),
            },
        );
        poller.entered.notified().await;
        worker.shutdown.send_replace(true);
        assert!(!worker.task.is_finished());
        assert_eq!(poller.completed.load(Ordering::SeqCst), 0);
        poller.release.notify_one();
        tokio::time::timeout(Duration::from_secs(1), &mut worker.task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(poller.completed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn 监督管道关闭后才触发随桌面退出() {
        let (reader, writer) = std::io::pipe().expect("可创建管道");
        let closed = input_closed(reader);
        tokio::pin!(closed);

        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut closed)
                .await
                .is_err(),
            "桌面还持有管道时 Bridge 不能退出"
        );
        drop(writer);
        tokio::time::timeout(Duration::from_secs(5), closed)
            .await
            .expect("桌面一退出，管道关闭就应触发退出");
    }

    impl TargetedHandoffPoller for 计数交接轮询器 {
        fn claim_once(
            &self,
        ) -> PortFuture<'_, Result<TargetedHandoffClaimOutcome, TargetedHandoffInboxServiceFailure>>
        {
            Box::pin(async move {
                if self.claims.fetch_add(1, Ordering::SeqCst) == 0 {
                    self.first_claim.notify_one();
                }
                Ok(TargetedHandoffClaimOutcome::Empty)
            })
        }
    }
}

#[cfg(test)]
mod first_authorization_tests {
    use std::{
        collections::VecDeque,
        sync::{
            Arc, Mutex,
            atomic::{AtomicU32, Ordering},
        },
    };

    use agent_room_application::{
        devices::{AuthenticatedDevice, DeviceCredentials},
        ports::{
            DeviceSignature, OidcDeviceAuthorizationPrompt, OidcDeviceAuthorizationPromptSink,
            OidcDeviceGrantGateway, OidcDevicePromptFailure, OidcFailure, OidcFailureKind,
            OidcResult, PortFuture, PrincipalAccount, ProfileImportConsent, SecretValue,
        },
    };
    use agent_room_bridge_core::{
        authorization::{
            AuthorizeBridgeDevice, BridgeAuthorizationDependencies, BridgeAuthorizationFailure,
            BridgeAuthorizationService,
        },
        ports::{
            BridgeCredentialResult, ControlPlaneDeviceGateway, ControlPlaneDeviceResult,
            DeviceCredentialVault, DeviceSigningIdentity, DeviceSigningIdentityStore,
            RefreshBridgeDevice, RegisterBridgeDevice, StoredBridgeDeviceCredentials,
        },
        reconnect::ReconnectPolicy,
    };
    use agent_room_domain::{
        devices::{DevicePlatform, DevicePublicSigningKey},
        identity::Principal,
        ids::{DeviceId, PrincipalId},
        time::{DurationMillis, UtcMillis},
    };
    use agent_room_identity_adapter::SecureSecretFactory;
    use serde_json::json;

    use super::{
        BridgeRuntimeError, BridgeSupervisorEvent, ConfiguredDeviceAuthorization,
        authorize_first_device,
    };

    #[derive(Debug, Clone, Copy)]
    enum 身份服务表现 {
        展示设备码前失败(OidcFailureKind),
        展示设备码后失败(OidcFailureKind),
        批准,
    }

    struct 脚本身份服务 {
        script: Mutex<VecDeque<身份服务表现>>,
        attempts: AtomicU32,
    }

    impl OidcDeviceGrantGateway for 脚本身份服务 {
        fn authorize<'a>(
            &'a self,
            prompt_sink: &'a dyn OidcDeviceAuthorizationPromptSink,
        ) -> PortFuture<'a, OidcResult<SecretValue>> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            let step = self
                .script
                .lock()
                .expect("脚本锁未中毒")
                .pop_front()
                .expect("授权尝试次数超出脚本");
            Box::pin(async move {
                if let 身份服务表现::展示设备码前失败(kind) = step {
                    return Err(OidcFailure::new(kind));
                }
                prompt_sink
                    .present(&OidcDeviceAuthorizationPrompt {
                        user_code: SecretValue::new("ABCD-EFGH").expect("测试验证码有效"),
                        verification_uri: "https://identity.example/device".to_owned(),
                        verification_uri_complete: None,
                        expires_in: DurationMillis::new(600_000).expect("时长有效"),
                        polling_interval: DurationMillis::new(5_000).expect("时长有效"),
                    })
                    .map_err(|_| OidcFailure::new(OidcFailureKind::PromptUnavailable))?;
                match step {
                    身份服务表现::展示设备码后失败(kind) => {
                        Err(OidcFailure::new(kind))
                    }
                    _ => Ok(SecretValue::new("header.payload.signature").expect("测试断言有效")),
                }
            })
        }
    }

    #[derive(Default)]
    struct 计数提示(AtomicU32);

    impl OidcDeviceAuthorizationPromptSink for 计数提示 {
        fn present(
            &self,
            _prompt: &OidcDeviceAuthorizationPrompt,
        ) -> Result<(), OidcDevicePromptFailure> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct 测试签名身份;

    impl DeviceSigningIdentity for 测试签名身份 {
        fn public_key(&self) -> BridgeCredentialResult<DevicePublicSigningKey> {
            Ok(DevicePublicSigningKey::new(vec![9; 32]).expect("测试公钥有效"))
        }

        fn sign(&self, _message: &[u8]) -> BridgeCredentialResult<DeviceSignature> {
            Ok(DeviceSignature::new(vec![7; 64]).expect("测试签名有效"))
        }
    }

    struct 测试签名存储;

    impl DeviceSigningIdentityStore for 测试签名存储 {
        fn load_or_create(&self) -> BridgeCredentialResult<Arc<dyn DeviceSigningIdentity>> {
            Ok(Arc::new(测试签名身份))
        }
    }

    struct 测试控制面;

    impl ControlPlaneDeviceGateway for 测试控制面 {
        fn register(
            &self,
            _request: RegisterBridgeDevice,
        ) -> PortFuture<'_, ControlPlaneDeviceResult<DeviceCredentials>> {
            Box::pin(async { Ok(设备凭据()) })
        }

        fn refresh(
            &self,
            _request: RefreshBridgeDevice,
        ) -> PortFuture<'_, ControlPlaneDeviceResult<DeviceCredentials>> {
            Box::pin(async { Ok(设备凭据()) })
        }
    }

    #[derive(Default)]
    struct 内存凭据库(Mutex<Option<StoredBridgeDeviceCredentials>>);

    impl DeviceCredentialVault for 内存凭据库 {
        fn load(&self) -> BridgeCredentialResult<Option<StoredBridgeDeviceCredentials>> {
            Ok(self.0.lock().expect("凭据锁未中毒").clone())
        }

        fn replace(
            &self,
            credentials: &StoredBridgeDeviceCredentials,
        ) -> BridgeCredentialResult<()> {
            self.0
                .lock()
                .expect("凭据锁未中毒")
                .replace(credentials.clone());
            Ok(())
        }

        fn clear(&self) -> BridgeCredentialResult<()> {
            self.0.lock().expect("凭据锁未中毒").take();
            Ok(())
        }
    }

    fn 设备凭据() -> DeviceCredentials {
        DeviceCredentials {
            device: AuthenticatedDevice {
                account: PrincipalAccount {
                    principal: Principal::new(PrincipalId::from_uuid(uuid::Uuid::from_u128(1))),
                    matrix_user_id: "@device-user:matrix.example".to_owned(),
                    display_name: "设备用户".to_owned(),
                    avatar_content_id: None,
                    locale: "zh-CN".to_owned(),
                },
                device_id: DeviceId::from_uuid(uuid::Uuid::from_u128(2)),
                access_token_expires_at: UtcMillis::new(301_000).expect("测试时间有效"),
            },
            access_token: SecretValue::new("bridge-access-token").expect("测试 Token 有效"),
            refresh_token: SecretValue::new("bridge-refresh-token").expect("测试 Token 有效"),
            refresh_token_expires_at: UtcMillis::new(86_401_000).expect("测试时间有效"),
        }
    }

    fn 首次授权(
        script: impl IntoIterator<Item = 身份服务表现>,
    ) -> (ConfiguredDeviceAuthorization, Arc<脚本身份服务>) {
        let oidc = Arc::new(脚本身份服务 {
            script: Mutex::new(script.into_iter().collect()),
            attempts: AtomicU32::new(0),
        });
        let service = BridgeAuthorizationService::new(BridgeAuthorizationDependencies {
            oidc: oidc.clone(),
            signing_identities: Arc::new(测试签名存储),
            control_plane: Arc::new(测试控制面),
            credentials: Arc::new(内存凭据库::default()),
            secrets: Arc::new(SecureSecretFactory),
        });
        let authorization = ConfiguredDeviceAuthorization {
            service,
            request: AuthorizeBridgeDevice {
                label: "Windows 验收设备".to_owned(),
                platform: DevicePlatform::Windows,
                profile_import: ProfileImportConsent {
                    display_name: false,
                    locale: false,
                },
            },
        };
        (authorization, oidc)
    }

    fn 毫秒级退避() -> ReconnectPolicy {
        ReconnectPolicy::new(
            DurationMillis::new(1).expect("时长有效"),
            DurationMillis::new(4).expect("时长有效"),
        )
        .expect("退避策略有效")
    }

    fn 不得重试(
        _failure: BridgeAuthorizationFailure,
        _delay: DurationMillis,
    ) -> Result<(), BridgeRuntimeError> {
        panic!("这类失败不得自动重试");
    }

    #[tokio::test]
    async fn 首次授权在拿到设备码前连不上身份服务时不退出而是退避重试() {
        let (authorization, oidc) = 首次授权([
            身份服务表现::展示设备码前失败(OidcFailureKind::DependencyUnavailable),
            身份服务表现::展示设备码前失败(OidcFailureKind::DependencyUnavailable),
            身份服务表现::批准,
        ]);
        let prompts = 计数提示::default();
        let retries = Mutex::new(Vec::new());

        let authorized = authorize_first_device(
            &authorization,
            &prompts,
            毫秒级退避(),
            |failure, delay| {
                retries.lock().expect("重试记录锁未中毒").push((
                    BridgeRuntimeError::authorization(failure).code(),
                    delay.value(),
                ));
                Ok(())
            },
        )
        .await
        .expect("身份服务恢复后应完成授权");

        assert_eq!(oidc.attempts.load(Ordering::SeqCst), 3);
        assert_eq!(prompts.0.load(Ordering::SeqCst), 1);
        assert_eq!(authorized.device_id, 设备凭据().device.device_id);
        let retries = retries.into_inner().expect("重试记录锁未中毒");
        assert_eq!(retries.len(), 2);
        for (code, delay) in retries {
            assert_eq!(code, "bridge.identity_provider_unavailable");
            assert!((1..=4).contains(&delay), "退避必须来自重连策略：{delay}");
        }
    }

    #[tokio::test]
    async fn 已经展示设备码的授权失败不会自动换新码() {
        let (authorization, oidc) = 首次授权([身份服务表现::展示设备码后失败(
            OidcFailureKind::DependencyUnavailable,
        )]);
        let prompts = 计数提示::default();

        let failure = authorize_first_device(&authorization, &prompts, 毫秒级退避(), 不得重试)
            .await
            .expect_err("展示设备码后的失败必须交给用户显式重试");

        assert_eq!(failure.code(), "bridge.identity_provider_unavailable");
        assert_eq!(oidc.attempts.load(Ordering::SeqCst), 1);
        assert_eq!(prompts.0.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn 连得上但被拒绝或响应无效时照旧失败() {
        for (kind, code) in [
            (
                OidcFailureKind::ProviderRejected,
                "bridge.authorization_denied",
            ),
            (
                OidcFailureKind::InvalidConfiguration,
                "bridge.authorization_internal",
            ),
            (
                OidcFailureKind::InvalidIdentityToken,
                "bridge.identity_assertion_invalid",
            ),
        ] {
            let (authorization, oidc) =
                首次授权([身份服务表现::展示设备码前失败(kind)]);

            let failure = authorize_first_device(
                &authorization,
                &计数提示::default(),
                毫秒级退避(),
                不得重试,
            )
            .await
            .expect_err("非连通性失败不得重试");

            assert_eq!(failure.code(), code, "{kind:?}");
            assert_eq!(oidc.attempts.load(Ordering::SeqCst), 1, "{kind:?}");
        }
    }

    #[test]
    fn 连不上服务的监督事件携带稳定代码和重试间隔() {
        let event = serde_json::to_value(BridgeSupervisorEvent::ServerUnreachable {
            channel: "agent_room_desktop",
            code: "bridge.identity_provider_unavailable",
            retry_after_ms: 1_500,
        })
        .expect("监督事件可序列化");

        assert_eq!(
            event,
            json!({
                "event": "server_unreachable",
                "channel": "agent_room_desktop",
                "code": "bridge.identity_provider_unavailable",
                "retryAfterMs": 1_500
            })
        );
    }
}
