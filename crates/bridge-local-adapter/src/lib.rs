//! Bridge 与本地宿主之间的操作系统 IPC 适配器。

mod client;
mod credentials;
mod endpoint;
mod location;
mod secure_storage_service;

pub use client::{LocalBridgeClient, LocalBridgeClientFailure, LocalBridgeClientFailureKind};
pub use credentials::{
    IPC_INSTALLATION_ID_ACCOUNT, IPC_SHARED_SECRET_ACCOUNT, IpcCredentialFailure,
    IpcCredentialFailureKind, IpcCredentialSource, OsIpcCredentialReader,
};
pub use endpoint::LocalIpcEndpoint;
pub use location::{
    BridgeLocationFailure, BridgeLocationFailureKind, bridge_data_root_from_environment,
    bridge_runtime_root, resolve_bridge_data_root,
};
pub use secure_storage_service::{
    DEFAULT_SECURE_STORAGE_SERVICE, SecureStorageService, SecureStorageServiceFailure,
    resolve_secure_storage_service, secure_storage_service_from_environment,
};
