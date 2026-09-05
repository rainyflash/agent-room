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
        BridgeCredentialFailure, BridgeCredentialFailureKind, DeviceSigningIdentityStore,
        StatusEventIdentifierFactory,
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
    ids::{AgentId, AgentInstanceRegistrationRequestId, RoomCatalogId},
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
use tokio::{sync::watch, task::JoinHandle, time::sleep};

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
const FOUNDATION_AGENT_CAPABILITIES: [&str; 8] = [
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
    let request_handler: Arc<dyn BridgeIpcRequestHandler> = match agent_session.as_ref() {
        Some(runtime) => Arc::new(FoundationBridgeIpcRequestHandler::with_agent_runtime(
            status.clone(),
            runtime.state.clone(),
            runtime.previews.clone(),
            runtime.content.clone(),
            Arc::new(SystemClock),
        )),
        None => Arc::new(FoundationBridgeIpcRequestHandler::with_onboarding(
            status.clone(),
            initialize_onboarding(&config, device_session.service.clone())?,
        )),
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
    });
    let server = BridgeIpcServer::bind(
        &paths,
        runtime_secrets.installation_id().clone(),
        runtime_secrets.ipc_shared_secret().clone(),
        request_handler,
    )
    .map_err(BridgeRuntimeError::ipc)?;
    status.set_component_ready(
        BridgeRuntimeStatus::DEVICE_COMPONENT,
        device_session.initial_session.is_some(),
    );
    if let Some(runtime) = agent_session.as_ref() {
        status.set_component_ready(
            BridgeRuntimeStatus::AGENT_COMPONENT,
            runtime.initial_session.is_some(),
        );
    }
    status.finish_starting();
    announce_supervisor_ready()?;
    run_until_shutdown(server, status, device_session, agent_session, host_sessions).await
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

#[derive(Clone, Copy)]
struct AgentSessionTarget {
    agent_id: AgentId,
    lobby_catalog_id: RoomCatalogId,
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
        },
        BridgeSessionPolicy::new(refresh_lead_time),
    ));

    let authorization_service = BridgeAuthorizationService::new(BridgeAuthorizationDependencies {
        oidc,
        signing_identities,
        control_plane,
        credentials,
        secrets,
    });
    let initial_session =
        establish_initial_session(config, &session_service, authorization_service).await?;

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
    let mut runtime = compose_agent_session_runtime(
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
        },
    )
    .await?;
    runtime.initial_session = match establish_agent_online(&runtime).await {
        Ok(online) => {
            announce_agent_online(&online)?;
            runtime.state.publish(&online);
            Some(online)
        }
        Err(failure) if is_reconnectable_agent_online_failure(failure) => {
            if let Err(error) = announce_supervisor_diagnostic(failure) {
                tracing::warn!(
                    error_code = error.code(),
                    "Agent 暂时失败诊断无法写入监督通道"
                );
            }
            tracing::warn!(
                failure_kind = ?failure.kind(),
                "Agent 上线流程暂时不可用，Bridge 将在后台重试"
            );
            None
        }
        Err(failure) => return Err(BridgeRuntimeError::agent_online(failure)),
    };
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
    );
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
    let mut online = AgentOnlineSession {
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
        next_batch: None,
    };
    if let Err(failure) = sync_agent_online(runtime, &mut online, true).await {
        online.stop_workers().await;
        return Err(failure);
    }
    Ok(online)
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
) -> Result<Option<ActiveBridgeSession>, BridgeRuntimeError> {
    match session_service.active_session().await {
        Ok(session) => {
            announce_active_session(&session)?;
            Ok(Some(session))
        }
        Err(error) if error.kind() == BridgeSessionFailureKind::NotAuthorized => {
            let authorized = authorization_service
                .authorize(
                    AuthorizeBridgeDevice {
                        label: config.device_label.clone(),
                        platform: current_platform(),
                        profile_import: ProfileImportConsent {
                            display_name: config.import_oidc_profile,
                            locale: config.import_oidc_profile,
                        },
                    },
                    &TerminalAuthorizationPrompt,
                )
                .await
                .map_err(BridgeRuntimeError::authorization)?;
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
    server: BridgeIpcServer,
    status: Arc<BridgeRuntimeStatus>,
    device_session: DeviceSessionRuntime,
    agent_session: Option<AgentSessionRuntime>,
    host_sessions: Arc<HostSessionRegistry>,
) -> Result<(), BridgeRuntimeError> {
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    let mut server_task = tokio::spawn(server.run(shutdown_receiver));
    let mut session_task = tokio::spawn(maintain_sessions(
        device_session,
        agent_session,
        status.clone(),
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
        signal = tokio::signal::ctrl_c() => {
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
    } }.await;
    shutdown_sender.send_replace(true);
    host_sessions.shutdown().await;
    janitor
        .await
        .map_err(|_| BridgeRuntimeError::session_task())?;
    result
}

async fn maintain_sessions(
    device: DeviceSessionRuntime,
    agent: Option<AgentSessionRuntime>,
    status: Arc<BridgeRuntimeStatus>,
    shutdown: watch::Receiver<bool>,
) {
    let device_task = maintain_device_session(
        device.service,
        device.initial_session,
        status.clone(),
        device.clock,
        device.refresh_lead_time,
        device.reconnect_policy,
        shutdown.clone(),
    );
    let agent_task = maintain_agent_session(agent, status, shutdown);
    tokio::join!(device_task, agent_task);
}

async fn maintain_device_session(
    session_service: Arc<BridgeSessionService>,
    mut session: Option<ActiveBridgeSession>,
    status: Arc<BridgeRuntimeStatus>,
    clock: Arc<dyn Clock>,
    refresh_lead_time: DurationMillis,
    reconnect_policy: ReconnectPolicy,
    mut shutdown: watch::Receiver<bool>,
) {
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
            Err(failure) => {
                status.mark_fatal();
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
        self.mark_fatal();
    }

    fn mark_shutting_down(&self) {
        self.shutting_down.store(true, Ordering::Release);
    }

    fn state(&self) -> IpcBridgeState {
        if self.shutting_down.load(Ordering::Acquire) {
            IpcBridgeState::ShuttingDown
        } else if self.fatal.load(Ordering::Acquire) {
            IpcBridgeState::Offline
        } else if self.starting.load(Ordering::Acquire) {
            IpcBridgeState::Starting
        } else if self.ready_components.load(Ordering::Acquire) & self.required_components
            == self.required_components
        {
            IpcBridgeState::Ready
        } else {
            IpcBridgeState::Reconnecting
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

impl OidcDeviceAuthorizationPromptSink for TerminalAuthorizationPrompt {
    fn present(
        &self,
        prompt: &OidcDeviceAuthorizationPrompt,
    ) -> Result<(), OidcDevicePromptFailure> {
        let destination = prompt
            .verification_uri_complete
            .as_deref()
            .unwrap_or(&prompt.verification_uri);
        if supervisor_events_enabled() {
            return write_supervisor_event(&BridgeSupervisorEvent::AuthorizationRequired {
                channel: "agent_room_desktop",
                verification_uri: destination,
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
    Ready {
        channel: &'static str,
    },
    TransientFailure {
        channel: &'static str,
        code: &'static str,
    },
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
            BridgeSessionFailureKind::NotAuthorized => ("bridge.not_authorized", "设备尚未授权"),
            BridgeSessionFailureKind::RefreshOutcomeUnknown => (
                "bridge.refresh_outcome_unknown",
                "刷新结果未知；旧刷新令牌不会被再次使用，请在设备管理页撤销后重新授权",
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
        TargetedHandoffPoller, TargetedHandoffPollingPolicy, is_reconnectable_agent_online_failure,
        spawn_targeted_handoff_worker_with_policy,
    };

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
