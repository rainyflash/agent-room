use agent_room_agent_client::BridgeToolFailure;
use agent_room_bridge_ipc::IpcErrorCategory;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type ReceptionResult<T> = Result<T, ReceptionFailure>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceptionFailure {
    pub code: String,
    pub category: IpcErrorCategory,
    pub retryable: bool,
    pub details: BTreeMap<String, String>,
}

impl ReceptionFailure {
    pub fn local(code: &str) -> Self {
        Self {
            code: code.into(),
            category: IpcErrorCategory::Internal,
            retryable: false,
            details: BTreeMap::new(),
        }
    }
    pub fn validation(code: &str) -> Self {
        Self {
            category: IpcErrorCategory::Validation,
            ..Self::local(code)
        }
    }
}
impl From<BridgeToolFailure> for ReceptionFailure {
    fn from(error: BridgeToolFailure) -> Self {
        Self {
            code: error.code().into(),
            category: error.category(),
            retryable: error.retryable(),
            details: error.details().clone(),
        }
    }
}
