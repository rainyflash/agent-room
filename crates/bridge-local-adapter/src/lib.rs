//! Bridge 与本地宿主之间的操作系统 IPC 适配器。

mod client;
mod credentials;
mod endpoint;
mod location;

pub use client::{LocalBridgeClient, LocalBridgeClientFailure, LocalBridgeClientFailureKind};
pub use credentials::{
    DEFAULT_SECURE_STORAGE_SERVICE, IPC_INSTALLATION_ID_ACCOUNT, IPC_SHARED_SECRET_ACCOUNT,
    IpcCredentialFailure, IpcCredentialFailureKind, IpcCredentialSource, OsIpcCredentialReader,
};
pub use endpoint::LocalIpcEndpoint;
pub use location::{
    BridgeLocationFailure, BridgeLocationFailureKind, bridge_data_root_from_environment,
    bridge_runtime_root, resolve_bridge_data_root,
};
