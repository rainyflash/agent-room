use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};

use agent_room_bridge_core::ipc::IpcCallerKind;
use agent_room_bridge_ipc::{
    IpcClientFailure, IpcClientFailureKind, IpcClientSession, IpcErrorCategory, IpcMethod,
    IpcResponse,
};
use interprocess::local_socket::tokio::{Stream, prelude::*};
use tokio::time::timeout;

use crate::{
    IpcCredentialFailure, IpcCredentialFailureKind, IpcCredentialSource, LocalIpcEndpoint,
    OsIpcCredentialReader, SecureStorageService,
};

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const DEFAULT_OPERATION_TIMEOUT: Duration = Duration::from_secs(15);

pub struct LocalBridgeClient {
    runtime_root: PathBuf,
    credentials: Arc<dyn IpcCredentialSource>,
    caller: IpcCallerKind,
    connect_timeout: Duration,
    operation_timeout: Duration,
}

impl LocalBridgeClient {
    /// 创建只能以 Codex 插件身份协商作用域的本地客户端。
    pub fn system(runtime_root: PathBuf) -> Self {
        Self::system_with_secure_storage_service(runtime_root, SecureStorageService::default())
    }

    /// 使用显式安全存储命名空间创建 Codex 插件客户端。
    pub fn system_with_secure_storage_service(
        runtime_root: PathBuf,
        service: SecureStorageService,
    ) -> Self {
        Self::for_caller(runtime_root, IpcCallerKind::CodexPlugin, service)
    }

    /// 创建供受信桌面壳执行用户确认操作的本地客户端。
    pub fn desktop_shell(runtime_root: PathBuf) -> Self {
        Self::desktop_shell_with_secure_storage_service(
            runtime_root,
            SecureStorageService::default(),
        )
    }

    /// 使用显式安全存储命名空间创建受信桌面壳客户端。
    pub fn desktop_shell_with_secure_storage_service(
        runtime_root: PathBuf,
        service: SecureStorageService,
    ) -> Self {
        Self::for_caller(runtime_root, IpcCallerKind::DesktopShell, service)
    }

    fn for_caller(
        runtime_root: PathBuf,
        caller: IpcCallerKind,
        service: SecureStorageService,
    ) -> Self {
        Self {
            runtime_root,
            credentials: Arc::new(OsIpcCredentialReader::system(service.into_string())),
            caller,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            operation_timeout: DEFAULT_OPERATION_TIMEOUT,
        }
    }

    /// 以方法需要的唯一作用域建立短会话并转发一次请求。
    ///
    /// # Errors
    ///
    /// Bridge 未启动、本地凭据不可用、超时或远端用例失败时返回可修复错误。
    pub async fn invoke(&self, method: IpcMethod) -> Result<IpcResponse, LocalBridgeClientFailure> {
        let required_scope = method.required_scope();
        let credentials = self
            .credentials
            .load()
            .map_err(LocalBridgeClientFailure::credential)?;
        let endpoint =
            LocalIpcEndpoint::from_installation(&self.runtime_root, credentials.installation_id());
        let name = endpoint
            .to_name()
            .map_err(|_| LocalBridgeClientFailure::endpoint())?;
        let stream = timeout(self.connect_timeout, Stream::connect(name))
            .await
            .map_err(|_| LocalBridgeClientFailure::timeout())?
            .map_err(|_| LocalBridgeClientFailure::unavailable())?;
        let mut client = timeout(
            self.operation_timeout,
            IpcClientSession::authenticate(stream, &credentials, self.caller, [required_scope]),
        )
        .await
        .map_err(|_| LocalBridgeClientFailure::timeout())?
        .map_err(|failure| LocalBridgeClientFailure::ipc(&failure))?;
        timeout(self.operation_timeout, client.request(method))
            .await
            .map_err(|_| LocalBridgeClientFailure::timeout())?
            .map_err(|failure| LocalBridgeClientFailure::ipc(&failure))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalBridgeClientFailureKind {
    CredentialsMissing,
    CredentialsUnavailable,
    CredentialsCorrupt,
    EndpointInvalid,
    BridgeUnavailable,
    Timeout,
    Validation,
    Authentication,
    Authorization,
    IncompatibleVersion,
    Protocol,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalBridgeClientFailure {
    kind: LocalBridgeClientFailureKind,
    code: String,
    category: IpcErrorCategory,
    retryable: bool,
    details: BTreeMap<String, String>,
}

impl LocalBridgeClientFailure {
    fn new(
        kind: LocalBridgeClientFailureKind,
        code: impl Into<String>,
        category: IpcErrorCategory,
        retryable: bool,
    ) -> Self {
        Self {
            kind,
            code: code.into(),
            category,
            retryable,
            details: BTreeMap::new(),
        }
    }

    fn credential(failure: IpcCredentialFailure) -> Self {
        match failure.kind() {
            IpcCredentialFailureKind::Missing => Self::new(
                LocalBridgeClientFailureKind::CredentialsMissing,
                "bridge.ipc.credentials_missing",
                IpcErrorCategory::DependencyUnavailable,
                false,
            ),
            IpcCredentialFailureKind::Unavailable => Self::new(
                LocalBridgeClientFailureKind::CredentialsUnavailable,
                "bridge.ipc.credentials_unavailable",
                IpcErrorCategory::DependencyUnavailable,
                true,
            ),
            IpcCredentialFailureKind::Corrupt => Self::new(
                LocalBridgeClientFailureKind::CredentialsCorrupt,
                "bridge.ipc.credentials_corrupt",
                IpcErrorCategory::Authentication,
                false,
            ),
        }
    }

    fn endpoint() -> Self {
        Self::new(
            LocalBridgeClientFailureKind::EndpointInvalid,
            "bridge.ipc.endpoint_invalid",
            IpcErrorCategory::Internal,
            false,
        )
    }

    fn unavailable() -> Self {
        Self::new(
            LocalBridgeClientFailureKind::BridgeUnavailable,
            "bridge.ipc.bridge_unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        )
    }

    fn timeout() -> Self {
        Self::new(
            LocalBridgeClientFailureKind::Timeout,
            "bridge.ipc.timeout",
            IpcErrorCategory::DependencyUnavailable,
            true,
        )
    }

    fn ipc(failure: &IpcClientFailure) -> Self {
        let kind = match failure.kind() {
            IpcClientFailureKind::Validation => LocalBridgeClientFailureKind::Validation,
            IpcClientFailureKind::Protocol | IpcClientFailureKind::InvalidHandshake => {
                LocalBridgeClientFailureKind::Protocol
            }
            IpcClientFailureKind::Authentication => LocalBridgeClientFailureKind::Authentication,
            IpcClientFailureKind::Authorization => LocalBridgeClientFailureKind::Authorization,
            IpcClientFailureKind::IncompatibleVersion => {
                LocalBridgeClientFailureKind::IncompatibleVersion
            }
            IpcClientFailureKind::Remote => LocalBridgeClientFailureKind::Remote,
        };
        Self {
            kind,
            code: failure.code().to_owned(),
            category: failure.category(),
            retryable: failure.retryable(),
            details: failure.details().clone(),
        }
    }

    pub const fn kind(&self) -> LocalBridgeClientFailureKind {
        self.kind
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub const fn category(&self) -> IpcErrorCategory {
        self.category
    }

    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    pub const fn details(&self) -> &BTreeMap<String, String> {
        &self.details
    }
}

#[cfg(test)]
mod tests {
    use super::{
        IpcCredentialFailure, IpcCredentialFailureKind, LocalBridgeClientFailure,
        LocalBridgeClientFailureKind,
    };

    #[test]
    fn 客户端给凭据故障保留可修复语义() {
        let missing = LocalBridgeClientFailure::credential(IpcCredentialFailure::new(
            IpcCredentialFailureKind::Missing,
        ));
        assert_eq!(
            missing.kind(),
            LocalBridgeClientFailureKind::CredentialsMissing
        );
        assert_eq!(missing.code(), "bridge.ipc.credentials_missing");
        assert!(!missing.retryable());
    }
}
