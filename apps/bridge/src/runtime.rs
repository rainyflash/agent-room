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
    Clock, MatrixFailure, MatrixFailureKind, OidcDeviceAuthorizationPrompt,
    OidcDeviceAuthorizationPromptSink, OidcDevicePromptFailure, ProfileImportConsent,
};
use agent_room_bridge_core::{
    agent_runtime::{
        AgentRuntimeRequestIdFactory, AgentRuntimeSessionConfig, AgentRuntimeSessionDependencies,
        AgentRuntimeSessionFailure, AgentRuntimeSessionFailureKind, AgentRuntimeSessionService,
        RegisteredAgentRuntime,
    },
    authorization::{
        AuthorizeBridgeDevice, AuthorizedBridgeDevice, BridgeAuthorizationDependencies,
        BridgeAuthorizationFailure, BridgeAuthorizationFailureKind, BridgeAuthorizationService,
    },
    reconnect::{ReconnectBackoff, ReconnectPolicy, SessionRefreshPlan},
    session::{
        ActiveBridgeSession, BridgeSessionDependencies, BridgeSessionFailure,
        BridgeSessionFailureKind, BridgeSessionPolicy, BridgeSessionService,
    },
};
use agent_room_bridge_ipc::IpcBridgeState;
use agent_room_bridge_local_adapter::DEFAULT_SECURE_STORAGE_SERVICE;
use agent_room_bridge_storage_adapter::SqliteHandoffStore;
use agent_room_domain::{
    devices::DevicePlatform,
    ids::AgentInstanceRegistrationRequestId,
    time::{DurationMillis, UtcMillis},
};
use agent_room_identity_adapter::{
    DiscoveredOidcDeviceGrant, OidcDeviceGrantConfig, SecureSecretFactory,
};
use agent_room_matrix_adapter::{
    MatrixSdkClientFactory, MatrixSdkConfiguration, MatrixSdkStoreConfiguration,
};
use tokio::{sync::watch, time::sleep};

use crate::{
    config::BridgeConfig,
    ipc::{
        BridgeIpcFailure, BridgeIpcFailureKind, BridgeIpcServer, BridgeStatusReader,
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
    ControlPlaneHttpConfig, ReqwestControlPlaneAgentRuntimeGateway,
    ReqwestControlPlaneDeviceGateway,
};

const CODEX_ADAPTER_CAPABILITY_VERSION: &str = "1.0";

pub(crate) async fn run() -> Result<(), BridgeRuntimeError> {
    let config = BridgeConfig::from_environment()
        .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?;
    let paths = BridgeRuntimePaths::new(config.data_root.clone());
    paths.prepare().map_err(BridgeRuntimeError::runtime_files)?;
    let _instance_lock = BridgeExclusiveLock::acquire(paths.instance_lock_path())
        .map_err(BridgeRuntimeError::instance_lock)?;
    let _matrix_store_lock = BridgeExclusiveLock::acquire(paths.matrix_store_lock_path())
        .map_err(BridgeRuntimeError::matrix_store_lock)?;
    let runtime_secrets = OsBridgeRuntimeSecretVault::system(DEFAULT_SECURE_STORAGE_SERVICE)
        .load_or_create()
        .map_err(BridgeRuntimeError::runtime_secrets)?;
    initialize_matrix_store(&config, &paths, &runtime_secrets).await?;
    initialize_handoff_store(&paths, &runtime_secrets).await?;
    let device_session = initialize_device_session(&config).await?;
    let agent_session = initialize_agent_session(&config, device_session.service.clone()).await?;

    let status = Arc::new(BridgeRuntimeStatus::new(
        SystemClock.now().value(),
        agent_session.is_some(),
    ));
    let server = BridgeIpcServer::bind(
        &paths,
        runtime_secrets.installation_id().clone(),
        runtime_secrets.ipc_shared_secret().clone(),
        Arc::new(FoundationBridgeIpcRequestHandler::new(status.clone())),
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
    config: AgentRuntimeSessionConfig,
    initial_session: Option<RegisteredAgentRuntime>,
    reconnect_policy: ReconnectPolicy,
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
        DEFAULT_SECURE_STORAGE_SERVICE,
    ));
    let credentials = Arc::new(OsDeviceCredentialVault::system(
        DEFAULT_SECURE_STORAGE_SERVICE,
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
    device_session: Arc<BridgeSessionService>,
) -> Result<Option<AgentSessionRuntime>, BridgeRuntimeError> {
    let Some(agent_id) = config.agent_id else {
        return Ok(None);
    };
    let control_plane = Arc::new(
        ReqwestControlPlaneAgentRuntimeGateway::new(
            &ControlPlaneHttpConfig {
                base_url: config.control_plane_url.clone(),
                request_timeout: config.request_timeout,
            },
            device_session,
        )
        .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?,
    );
    let service = Arc::new(AgentRuntimeSessionService::new(
        AgentRuntimeSessionDependencies {
            signing_identities: Arc::new(OsAgentInstanceSigningIdentityStore::system(
                DEFAULT_SECURE_STORAGE_SERVICE,
            )),
            control_plane,
            credentials: Arc::new(OsAgentRuntimeCredentialVault::system(
                DEFAULT_SECURE_STORAGE_SERVICE,
            )),
            identifiers: Arc::new(SystemAgentRuntimeIdentifiers),
        },
    ));
    let agent_config =
        AgentRuntimeSessionConfig::new(agent_id, "codex-desktop", CODEX_ADAPTER_CAPABILITY_VERSION)
            .map_err(BridgeRuntimeError::agent_runtime)?;
    let initial_session = match service.ensure_session(&agent_config).await {
        Ok(runtime) => {
            announce_agent_runtime(&runtime)?;
            Some(runtime)
        }
        Err(failure) if is_reconnectable_agent_runtime_failure(failure) => {
            tracing::warn!(
                operation = failure.operation(),
                failure_kind = ?failure.kind(),
                "Agent 运行时暂时不可用，Bridge 将在后台重试"
            );
            None
        }
        Err(failure) => return Err(BridgeRuntimeError::agent_runtime(failure)),
    };
    let reconnect_policy = ReconnectPolicy::new(
        domain_duration(config.reconnect_initial_delay)?,
        domain_duration(config.reconnect_maximum_delay)?,
    )
    .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?;
    Ok(Some(AgentSessionRuntime {
        service,
        config: agent_config,
        initial_session,
        reconnect_policy,
    }))
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

async fn initialize_matrix_store(
    config: &BridgeConfig,
    paths: &BridgeRuntimePaths,
    runtime_secrets: &BridgeRuntimeSecrets,
) -> Result<(), BridgeRuntimeError> {
    let sdk = MatrixSdkConfiguration::new(&config.matrix_homeserver_url, config.request_timeout)
        .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?;
    let store = MatrixSdkStoreConfiguration::encrypted_sqlite(
        paths.matrix_store_root().to_path_buf(),
        runtime_secrets.matrix_store_passphrase().clone(),
    )
    .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?;
    MatrixSdkClientFactory::with_encrypted_sqlite(sdk, store)
        .initialize_store()
        .await
        .map_err(BridgeRuntimeError::matrix_store)
}

async fn initialize_handoff_store(
    paths: &BridgeRuntimePaths,
    runtime_secrets: &BridgeRuntimeSecrets,
) -> Result<(), BridgeRuntimeError> {
    SqliteHandoffStore::open(
        paths.handoff_database(),
        runtime_secrets.handoff_storage_key().clone(),
    )
    .await
    .map_err(|failure| BridgeRuntimeError::handoff_store(&failure))?
    .close()
    .await;
    Ok(())
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
    let Some(runtime) = runtime else {
        wait_for_shutdown(&mut shutdown).await;
        return;
    };
    if runtime.initial_session.is_some() {
        wait_for_shutdown(&mut shutdown).await;
        return;
    }

    let mut backoff = ReconnectBackoff::new(runtime.reconnect_policy);
    loop {
        let delay = backoff.record_failure(retry_entropy());
        if wait_for_refresh(SessionRefreshPlan::After(delay), &mut shutdown).await {
            return;
        }
        match runtime.service.ensure_session(&runtime.config).await {
            Ok(agent) => {
                backoff.record_connected();
                status.set_component_ready(BridgeRuntimeStatus::AGENT_COMPONENT, true);
                if let Err(error) = announce_agent_runtime(&agent) {
                    tracing::warn!(error_code = error.code(), "Agent 运行时已就绪但终端不可写");
                }
                wait_for_shutdown(&mut shutdown).await;
                return;
            }
            Err(failure) if is_reconnectable_agent_runtime_failure(failure) => {
                status.set_component_ready(BridgeRuntimeStatus::AGENT_COMPONENT, false);
                tracing::warn!(
                    operation = failure.operation(),
                    failure_kind = ?failure.kind(),
                    consecutive_failures = backoff.consecutive_failures(),
                    retry_after_ms = delay.value(),
                    "Agent 运行时暂时不可用，已安排重连"
                );
            }
            Err(failure) => {
                status.mark_fatal();
                tracing::error!(
                    operation = failure.operation(),
                    failure_kind = ?failure.kind(),
                    "Agent 运行时进入离线态，禁止不安全重试"
                );
                wait_for_shutdown(&mut shutdown).await;
                return;
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

fn announce_agent_runtime(runtime: &RegisteredAgentRuntime) -> Result<(), BridgeRuntimeError> {
    write_stdout(&format!(
        "Agent 运行时已就绪。\nAgent：{}\n实例：{}\nMatrix 设备：{}\n",
        runtime.identity().display_name(),
        runtime.identity().agent_instance_id(),
        runtime.matrix_session().metadata().device_id().as_str()
    ))
}

struct SystemAgentRuntimeIdentifiers;

impl AgentRuntimeRequestIdFactory for SystemAgentRuntimeIdentifiers {
    fn registration_request_id(&self) -> AgentInstanceRegistrationRequestId {
        AgentInstanceRegistrationRequestId::from_uuid(uuid::Uuid::now_v7())
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
            "Bridge 设备会话维护任务异常终止",
        )
    }

    fn session_stopped_early() -> Self {
        Self::new(
            "bridge.session_stopped_early",
            "Bridge 设备会话维护任务已提前停止",
        )
    }

    fn runtime_secrets(failure: agent_room_bridge_core::ports::BridgeCredentialFailure) -> Self {
        match failure.kind() {
            agent_room_bridge_core::ports::BridgeCredentialFailureKind::Unavailable => Self::new(
                "bridge.runtime_secrets_unavailable",
                "Bridge 运行时秘密无法从操作系统安全存储读取",
            ),
            agent_room_bridge_core::ports::BridgeCredentialFailureKind::Corrupt => Self::new(
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
