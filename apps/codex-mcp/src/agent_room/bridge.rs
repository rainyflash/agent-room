use std::{collections::BTreeMap, future::Future, path::PathBuf, pin::Pin};

use agent_room_bridge_ipc::{IpcErrorCategory, IpcMethod, IpcResponse};
use agent_room_bridge_local_adapter::{
    LocalBridgeClient, LocalBridgeClientFailure, SecureStorageService,
};

pub type BridgeToolFuture<'a> =
    Pin<Box<dyn Future<Output = Result<IpcResponse, BridgeToolFailure>> + Send + 'a>>;

/// MCP 用例依赖的唯一 Bridge 端口。
///
/// 该端口刻意不暴露 Matrix、密钥或存储细节，避免插件进程成为第二个客户端。
pub trait BridgeToolClient: Send + Sync + 'static {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_>;
}

pub struct LocalBridgeToolClient {
    client: LocalBridgeClient,
}

impl LocalBridgeToolClient {
    pub fn system(runtime_root: PathBuf) -> Self {
        Self {
            client: LocalBridgeClient::system(runtime_root),
        }
    }

    pub fn system_with_secure_storage_service(
        runtime_root: PathBuf,
        service: SecureStorageService,
    ) -> Self {
        Self {
            client: LocalBridgeClient::system_with_secure_storage_service(runtime_root, service),
        }
    }
}

impl BridgeToolClient for LocalBridgeToolClient {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        Box::pin(async move {
            self.client
                .invoke(method)
                .await
                .map_err(BridgeToolFailure::from)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeToolFailure {
    code: String,
    category: IpcErrorCategory,
    retryable: bool,
    details: BTreeMap<String, String>,
}

impl BridgeToolFailure {
    pub(crate) fn new(
        code: impl Into<String>,
        category: IpcErrorCategory,
        retryable: bool,
        details: BTreeMap<String, String>,
    ) -> Self {
        Self {
            code: code.into(),
            category,
            retryable,
            details,
        }
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

impl From<LocalBridgeClientFailure> for BridgeToolFailure {
    fn from(failure: LocalBridgeClientFailure) -> Self {
        Self::new(
            failure.code(),
            failure.category(),
            failure.retryable(),
            failure.details().clone(),
        )
    }
}
