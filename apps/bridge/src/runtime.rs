use std::{
    fmt,
    io::{self, Write as _},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use agent_room_application::ports::{
    Clock, MatrixFailure, MatrixFailureKind, OidcDeviceAuthorizationPrompt,
    OidcDeviceAuthorizationPromptSink, OidcDevicePromptFailure, ProfileImportConsent,
};
use agent_room_bridge_core::{
    authorization::{
        AuthorizeBridgeDevice, AuthorizedBridgeDevice, BridgeAuthorizationDependencies,
        BridgeAuthorizationFailure, BridgeAuthorizationFailureKind, BridgeAuthorizationService,
    },
    session::{
        ActiveBridgeSession, BridgeSessionDependencies, BridgeSessionFailure,
        BridgeSessionFailureKind, BridgeSessionPolicy, BridgeSessionService,
    },
};
use agent_room_bridge_ipc::IpcBridgeState;
use agent_room_domain::{
    devices::DevicePlatform,
    time::{DurationMillis, UtcMillis},
};
use agent_room_identity_adapter::{
    DiscoveredOidcDeviceGrant, OidcDeviceGrantConfig, SecureSecretFactory,
};
use agent_room_matrix_adapter::{
    MatrixSdkClientFactory, MatrixSdkConfiguration, MatrixSdkStoreConfiguration,
};
use tokio::sync::watch;

use crate::{
    config::BridgeConfig,
    control_plane::{ControlPlaneHttpConfig, ReqwestControlPlaneDeviceGateway},
    ipc::{
        BridgeIpcFailure, BridgeIpcFailureKind, BridgeIpcServer, BridgeStatusReader,
        BridgeStatusSnapshot,
    },
    runtime_files::{
        BridgeExclusiveLock, BridgeRuntimeFileFailure, BridgeRuntimeFileFailureKind,
        BridgeRuntimePaths,
    },
    secure_storage::{
        BridgeRuntimeSecrets, OsBridgeRuntimeSecretVault, OsDeviceCredentialVault,
        OsDeviceSigningIdentityStore,
    },
};

const SECURE_STORAGE_SERVICE: &str = "dev.agent-room.bridge";

pub(crate) async fn run() -> Result<(), BridgeRuntimeError> {
    let config = BridgeConfig::from_environment()
        .map_err(|error| BridgeRuntimeError::configuration(error.to_string()))?;
    let paths = BridgeRuntimePaths::new(config.data_root.clone());
    paths.prepare().map_err(BridgeRuntimeError::runtime_files)?;
    let _instance_lock = BridgeExclusiveLock::acquire(paths.instance_lock_path())
        .map_err(BridgeRuntimeError::instance_lock)?;
    let _matrix_store_lock = BridgeExclusiveLock::acquire(paths.matrix_store_lock_path())
        .map_err(BridgeRuntimeError::matrix_store_lock)?;
    let runtime_secrets = OsBridgeRuntimeSecretVault::system(SECURE_STORAGE_SERVICE)
        .load_or_create()
        .map_err(BridgeRuntimeError::runtime_secrets)?;
    initialize_matrix_store(&config, &paths, &runtime_secrets).await?;
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
    let signing_identities = Arc::new(OsDeviceSigningIdentityStore::system(SECURE_STORAGE_SERVICE));
    let credentials = Arc::new(OsDeviceCredentialVault::system(SECURE_STORAGE_SERVICE));
    let secrets = Arc::new(SecureSecretFactory);
    let clock = Arc::new(SystemClock);
    let session_service = BridgeSessionService::new(
        BridgeSessionDependencies {
            signing_identities: signing_identities.clone(),
            control_plane: control_plane.clone(),
            credentials: credentials.clone(),
            secrets: secrets.clone(),
            clock,
        },
        BridgeSessionPolicy::new(domain_duration(config.refresh_lead_time)?),
    );

    match session_service.active_session().await {
        Ok(session) => announce_active_session(&session)?,
        Err(error) if error.kind() == BridgeSessionFailureKind::NotAuthorized => {
            let authorization_service =
                BridgeAuthorizationService::new(BridgeAuthorizationDependencies {
                    oidc,
                    signing_identities,
                    control_plane,
                    credentials,
                    secrets,
                });
            let authorized = authorization_service
                .authorize(
                    AuthorizeBridgeDevice {
                        label: config.device_label,
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
        }
        Err(error) => return Err(BridgeRuntimeError::session(error)),
    }

    let status = Arc::new(BridgeRuntimeStatus::new(SystemClock.now().value()));
    let server = BridgeIpcServer::bind(
        &paths,
        runtime_secrets.installation_id().clone(),
        runtime_secrets.ipc_shared_secret().clone(),
        status.clone(),
    )
    .map_err(BridgeRuntimeError::ipc)?;
    status.transition(IpcBridgeState::Ready);
    run_until_shutdown(server, status).await
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

async fn run_until_shutdown(
    server: BridgeIpcServer,
    status: Arc<BridgeRuntimeStatus>,
) -> Result<(), BridgeRuntimeError> {
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    let mut server_task = tokio::spawn(server.run(shutdown_receiver));

    tokio::select! {
        signal = tokio::signal::ctrl_c() => {
            signal.map_err(|_| BridgeRuntimeError::shutdown_signal())?;
            status.transition(IpcBridgeState::ShuttingDown);
            shutdown_sender
                .send(true)
                .map_err(|_| BridgeRuntimeError::ipc_stopped_early())?;
            server_task
                .await
                .map_err(|_| BridgeRuntimeError::ipc_task())?
                .map_err(BridgeRuntimeError::ipc)
        }
        completed = &mut server_task => {
            completed
                .map_err(|_| BridgeRuntimeError::ipc_task())?
                .map_err(BridgeRuntimeError::ipc)
        }
    }
}

struct BridgeRuntimeStatus {
    state: AtomicU8,
    started_at_unix_ms: i64,
}

impl BridgeRuntimeStatus {
    const STARTING: u8 = 0;
    const READY: u8 = 1;
    const RECONNECTING: u8 = 2;
    const OFFLINE: u8 = 3;
    const SHUTTING_DOWN: u8 = 4;

    const fn new(started_at_unix_ms: i64) -> Self {
        Self {
            state: AtomicU8::new(Self::STARTING),
            started_at_unix_ms,
        }
    }

    fn transition(&self, state: IpcBridgeState) {
        self.state.store(Self::encode(state), Ordering::Release);
    }

    const fn encode(state: IpcBridgeState) -> u8 {
        match state {
            IpcBridgeState::Starting => Self::STARTING,
            IpcBridgeState::Ready => Self::READY,
            IpcBridgeState::Reconnecting => Self::RECONNECTING,
            IpcBridgeState::Offline => Self::OFFLINE,
            IpcBridgeState::ShuttingDown => Self::SHUTTING_DOWN,
        }
    }

    const fn decode(state: u8) -> IpcBridgeState {
        match state {
            Self::STARTING => IpcBridgeState::Starting,
            Self::READY => IpcBridgeState::Ready,
            Self::RECONNECTING => IpcBridgeState::Reconnecting,
            Self::SHUTTING_DOWN => IpcBridgeState::ShuttingDown,
            _ => IpcBridgeState::Offline,
        }
    }
}

impl BridgeStatusReader for BridgeRuntimeStatus {
    fn read_status(&self) -> BridgeStatusSnapshot {
        BridgeStatusSnapshot {
            state: Self::decode(self.state.load(Ordering::Acquire)),
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
