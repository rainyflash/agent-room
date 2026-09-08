//! Durable, ordered reception shared by the desktop and CLI.
mod codex;
mod error;
mod model;
mod receipt;
mod runtime;
mod store;
pub use codex::CodexBinding;
pub use error::{ReceptionFailure, ReceptionResult};
pub use model::*;
pub use runtime::{ReceiverContext, ReceiverMode, run};
pub use store::{ReceiverStore, load_binding};

use agent_room_agent_client::BridgeToolClient;
use agent_room_bridge_ipc::{IpcMethod, IpcResponse};
fn scoped(session_id: &str, method: IpcMethod) -> IpcMethod {
    IpcMethod::WithSession {
        session_id: session_id.to_owned(),
        method: Box::new(method),
    }
}
async fn call(backend: &dyn BridgeToolClient, method: IpcMethod) -> ReceptionResult<IpcResponse> {
    method
        .validate()
        .map_err(|e| ReceptionFailure::validation(e.code()))?;
    backend.invoke(method).await.map_err(Into::into)
}

/// An explicit host boundary keeps delivery and cursor decisions testable without a model.
pub type HostFuture<'a> = std::pin::Pin<Box<dyn Future<Output = ReceptionResult<()>> + Send + 'a>>;
pub trait HostRunner: Send + Sync {
    fn resume<'a>(&'a self, delivery: HostDelivery<'a>) -> HostFuture<'a>;
}
pub struct HostDelivery<'a> {
    pub binding: &'a CodexBinding,
    pub data_root: &'a std::path::Path,
    pub service: &'a str,
    pub session_id: &'a str,
    pub automation_grant_id: &'a str,
    pub submission_id: &'a str,
    pub message: &'a agent_room_bridge_ipc::IpcMessagePreviewSummary,
}
pub struct CodexHost;
impl HostRunner for CodexHost {
    fn resume<'a>(&'a self, delivery: HostDelivery<'a>) -> HostFuture<'a> {
        Box::pin(codex::resume(
            delivery.binding,
            delivery.data_root,
            delivery.service,
            delivery.session_id,
            delivery.automation_grant_id,
            delivery.submission_id,
            delivery.message,
        ))
    }
}
