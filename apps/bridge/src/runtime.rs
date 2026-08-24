use std::{
    fmt,
    io::{self, Write as _},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use agent_room_application::ports::{
    Clock, OidcDeviceAuthorizationPrompt, OidcDeviceAuthorizationPromptSink,
    OidcDevicePromptFailure, ProfileImportConsent,
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
use agent_room_domain::{
    devices::DevicePlatform,
    time::{DurationMillis, UtcMillis},
};
use agent_room_identity_adapter::{
    DiscoveredOidcDeviceGrant, OidcDeviceGrantConfig, SecureSecretFactory,
};

use crate::{
    config::BridgeConfig,
    control_plane::{ControlPlaneHttpConfig, ReqwestControlPlaneDeviceGateway},
    runtime_files::{
        BridgeExclusiveLock, BridgeRuntimeFileFailure, BridgeRuntimeFileFailureKind,
        BridgeRuntimePaths,
    },
    secure_storage::{OsDeviceCredentialVault, OsDeviceSigningIdentityStore},
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
        Ok(session) => announce_active_session(&session),
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
            announce_authorized_device(authorized)
        }
        Err(error) => Err(BridgeRuntimeError::session(error)),
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
