//! Bridge 与本地宿主之间的操作系统 IPC 适配器。

mod client;
mod credentials;
mod endpoint;

pub use client::{LocalBridgeClient, LocalBridgeClientFailure, LocalBridgeClientFailureKind};
pub use credentials::{
    DEFAULT_SECURE_STORAGE_SERVICE, IPC_INSTALLATION_ID_ACCOUNT, IPC_SHARED_SECRET_ACCOUNT,
    IpcCredentialFailure, IpcCredentialFailureKind, IpcCredentialSource, OsIpcCredentialReader,
};
pub use endpoint::LocalIpcEndpoint;
