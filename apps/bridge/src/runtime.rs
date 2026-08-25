use std::{
    fmt,
    io::{self, Write as _},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use agent_room_application::ports::{
    Clock, MatrixFailure, MatrixFailureKind, MatrixGateway, MatrixRoomId, MatrixSyncRequest,
    MatrixSyncToken, OidcDeviceAuthorizationPrompt, OidcDeviceAuthorizationPromptSink,
    OidcDevicePromptFailure, ProfileImportConsent,
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
        ProjectedHandoffContentGateway,
    },
    lobby_session::{
        AgentLobbySessionConfig, AgentLobbySessionFailure, AgentLobbySessionFailureKind,
        AgentLobbySessionService, ControlPlaneLobbyEntryOutcome, JoinedAgentLobby,
    },
    messages::{
        MatrixMessageEventPublisher, MessageAuthenticationFailureKind, MessageContentGateway,
        MessageContentReadGateway, MessageProjectionStoreFailureKind,
        MessagePublicationDependencies, MessagePublicationService, MessageStoreFailureKind,
        MessageSyncDependencies, MessageSyncFailure, MessageSyncFailureKind, MessageSyncService,
        OpenMessageContentDependencies, OpenMessageContentService,
    },
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
use agent_room_bridge_storage_adapter::{InMemoryPresenceProjectionRepository, SqliteHandoffStore};
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
    ReqwestControlPlaneAgentRuntimeGateway, ReqwestControlPlaneContentGateway,
    ReqwestControlPlaneDeviceGateway, ReqwestControlPlaneHandoffGateway,
    ReqwestControlPlaneLobbyEntryGateway, ReqwestControlPlaneMessageContentGateway,
};
use agent_room_bridge_storage_adapter::{
    SqliteMessageSubmissionRepository, SqliteMessageTimelineRepository,
};

const CODEX_ADAPTER_CAPABILITY_VERSION: &str = "1.0";
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
        None => Arc::new(FoundationBridgeIpcRequestHandler::new(status.clone())),
    };
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
    run_until_shutdown(server, status, device_session, agent_session).await
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
    outbound_content: Arc<dyn MessageContentGateway>,
    submissions: Arc<SqliteMessageSubmissionRepository>,
    handoffs: AgentHandoffServices,
    state: Arc<BridgeAgentRuntimeState>,
    status_policy: AgentStatusLeasePolicy,
    sync_timeout: DurationMillis,
    initial_session: Option<AgentOnlineSession>,
    reconnect_policy: ReconnectPolicy,
}

struct AgentMessageServices {
    sync: Arc<MessageSyncService>,
    projections: Arc<SqliteMessageTimelineRepository>,
    content: Arc<OpenMessageContentService>,
    outbound_content: Arc<dyn MessageContentGateway>,
    submissions: Arc<SqliteMessageSubmissionRepository>,
    authenticator: Arc<dyn AgentEventAuthenticator>,
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
}

struct AgentOnlineSession {
    runtime: RegisteredAgentRuntime,
    lobby: JoinedAgentLobby,
    room_id: MatrixRoomId,
    matrix: Arc<dyn MatrixGateway>,
    status: Arc<AgentStatusPublicationHandle>,
    publication: Arc<MessagePublicationService>,
    handoffs: Arc<HandoffReceptionService>,
    handoff_delivery: Arc<HandoffDeliveryService>,
    handoff_worker: HandoffEventWorker,
    presence_projections: Arc<dyn PresenceProjectionRepository>,
    next_batch: Option<MatrixSyncToken>,
}

struct HandoffEventWorker {
    task: JoinHandle<()>,
    terminal_failure: watch::Receiver<Option<HandoffTransportFailureKind>>,
}

impl Drop for HandoffEventWorker {
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
            .with_status(online.status.clone())
            .with_message_publication(online.publication.clone())
            .with_handoff_delivery(online.handoff_delivery.clone())
            .with_handoffs(online.handoffs.clone())
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
        device_session,
        matrix,
        handoff_store,
        agent_id,
        lobby_catalog_id,
    )
    .await?;
    runtime.initial_session = match establish_agent_online(&runtime).await {
        Ok(online) => {
            announce_agent_online(&online)?;
            runtime.state.publish(&online);
            Some(online)
        }
        Err(failure) if is_reconnectable_agent_online_failure(failure) => {
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
    device_session: Arc<BridgeSessionService>,
    matrix: Arc<MatrixSdkClientFactory>,
    handoff_store: Arc<SqliteHandoffStore>,
    agent_id: AgentId,
    lobby_catalog_id: RoomCatalogId,
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
    let agent_config =
        AgentRuntimeSessionConfig::new(agent_id, "codex-desktop", CODEX_ADAPTER_CAPABILITY_VERSION)
            .map_err(BridgeRuntimeError::agent_runtime)?;
    let lobby = Arc::new(AgentLobbySessionService::new(Arc::new(
        ReqwestControlPlaneLobbyEntryGateway::new(&http, device_session.clone())
            .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?,
    )));
    let message_services =
        compose_agent_message_services(&http, paths, device_session.clone(), agent_id).await?;
    let handoff_gateway = Arc::new(
        ReqwestControlPlaneHandoffGateway::new(&http, device_session)
            .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?,
    );
    let handoffs = AgentHandoffServices {
        authorization: handoff_gateway.clone(),
        directory: handoff_gateway,
        authenticator: message_services.authenticator.clone(),
        content: message_services.handoff_content.clone(),
        store: handoff_store,
    };
    let state = Arc::new(BridgeAgentRuntimeState::new());
    let lobby_config = AgentLobbySessionConfig::new(
        lobby_catalog_id,
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
        lobby,
        lobby_config,
        matrix,
        messages: message_services.sync,
        presence: message_services.presence,
        presence_projections: message_services.presence_projections,
        previews: message_services.projections,
        content: message_services.content,
        outbound_content: message_services.outbound_content,
        submissions: message_services.submissions,
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
    })
}

async fn compose_agent_message_services(
    http: &ControlPlaneHttpConfig,
    paths: &BridgeRuntimePaths,
    device_session: Arc<BridgeSessionService>,
    actor_agent_id: AgentId,
) -> Result<AgentMessageServices, BridgeRuntimeError> {
    let verification = Arc::new(
        ReqwestAgentInstanceVerificationGateway::new(http, device_session.clone())
            .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?,
    );
    let projections = Arc::new(
        SqliteMessageTimelineRepository::open(paths.message_database())
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
    let content_reader: Arc<dyn MessageContentReadGateway> = Arc::new(
        ReqwestControlPlaneContentGateway::new(http, device_session, actor_agent_id)
            .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?,
    );
    let content = Arc::new(OpenMessageContentService::new(
        OpenMessageContentDependencies {
            projections: projections.clone(),
            content: content_reader.clone(),
        },
    ));
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
        ProjectedHandoffContentGateway::new(projections.clone(), content_reader),
    );
    Ok(AgentMessageServices {
        sync,
        projections,
        content,
        outbound_content,
        submissions,
        authenticator,
        handoff_content,
        presence,
        presence_projections,
    })
}

async fn establish_agent_online(
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
    let handoff_receipts = Arc::new(HandoffReceiptService::new(HandoffReceiptDependencies {
        identity: registered.identity().clone(),
        clock: Arc::new(SystemClock),
        authenticator: runtime.handoffs.authenticator.clone(),
        store: runtime.handoffs.store.clone(),
    }));
    let handoff_worker =
        spawn_handoff_event_worker(handoff_events, handoffs.clone(), handoff_receipts);
    let mut online = AgentOnlineSession {
        runtime: registered,
        lobby,
        room_id,
        matrix,
        status,
        publication,
        handoffs,
        handoff_delivery,
        handoff_worker,
        presence_projections: runtime.presence_projections.clone(),
        next_batch: None,
    };
    sync_agent_online(runtime, &mut online, true).await?;
    Ok(online)
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
    let task = tokio::spawn(async move {
        loop {
            let event = match events.receive().await {
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
        terminal_failure,
    }
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

async fn run_until_shutdown(
    server: BridgeIpcServer,
    status: Arc<BridgeRuntimeStatus>,
    device_session: DeviceSessionRuntime,
    agent_session: Option<AgentSessionRuntime>,
) -> Result<(), BridgeRuntimeError> {
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    let mut server_task = tokio::spawn(server.run(shutdown_receiver));
    let mut session_task = tokio::spawn(maintain_sessions(
        device_session,
        agent_session,
        status.clone(),
        shutdown_sender.subscribe(),
    ));

    tokio::select! {
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
    }
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
                return;
            };
            match sync {
                Ok(()) => {
                    backoff.record_connected();
                }
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
                    online = None;
                    retry_delay = Some(delay);
                }
                Err(failure) => {
                    status.mark_fatal();
                    runtime.state.clear();
                    tracing::error!(
                        failure_kind = ?failure.kind(),
                        "Agent Matrix 会话进入离线态，禁止不安全重试"
                    );
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
                if let Err(error) = announce_agent_online(&agent_online) {
                    tracing::warn!(error_code = error.code(), "Agent 已上线但终端不可写");
                }
                online = Some(agent_online);
            }
            Err(failure) if is_reconnectable_agent_online_failure(failure) => {
                status.set_component_ready(BridgeRuntimeStatus::AGENT_COMPONENT, false);
                runtime.state.clear();
                let delay = retry_delay_for_agent_failure(failure, &mut backoff);
                tracing::warn!(
                    failure_kind = ?failure.kind(),
                    consecutive_failures = backoff.consecutive_failures(),
                    retry_after_ms = delay.value(),
                    "Agent 上线流程暂时不可用，已安排重连"
                );
                retry_delay = Some(delay);
            }
            Err(failure) => {
                status.mark_fatal();
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
        if let Some(failure) = *handoff_failure.borrow() {
            return Some(Err(AgentOnlineFailure::HandoffTransport(failure)));
        }
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow_and_update() {
                    return None;
                }
            }
            result = sync_agent_online(runtime, active, false) => return Some(result),
            changed = handoff_failure.changed() => {
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
        () = sleep(std::time::Duration::from_millis(delay.value())) => false,
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
        AgentOnlineFailure::ProvisioningBusy(_) | AgentOnlineFailure::CapacityChanged => true,
        AgentOnlineFailure::Matrix(failure) => matches!(
            failure.kind(),
            MatrixFailureKind::RateLimited
                | MatrixFailureKind::Timeout
                | MatrixFailureKind::DependencyUnavailable
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

fn domain_duration(duration: std::time::Duration) -> Result<DurationMillis, BridgeRuntimeError> {
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
            MatrixFailureKind::Conflict
            | MatrixFailureKind::RateLimited
            | MatrixFailureKind::Timeout
            | MatrixFailureKind::DependencyUnavailable
            | MatrixFailureKind::StaleSyncToken => (
                "bridge.matrix_temporarily_unavailable",
                "Agent Matrix 同步暂时不可用",
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
    use agent_room_bridge_ipc::IpcBridgeState;

    use super::{BridgeRuntimeStatus, BridgeStatusReader};

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
}
