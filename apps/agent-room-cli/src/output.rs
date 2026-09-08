use agent_room_agent_client::BridgeToolFailure;
use agent_room_bridge_ipc::IpcErrorCategory;
use serde::Serialize;
use serde_json::json;
use std::{
    collections::BTreeMap,
    io::{self, Write},
};

pub(crate) type CliResult<T> = Result<T, CliFailure>;

#[derive(Debug, Serialize)]
pub(crate) struct CliFailure {
    pub(crate) code: String,
    pub(crate) category: IpcErrorCategory,
    pub(crate) retryable: bool,
    pub(crate) details: BTreeMap<String, String>,
}

impl CliFailure {
    pub(crate) fn local(code: &str) -> Self {
        Self {
            code: code.into(),
            category: IpcErrorCategory::Internal,
            retryable: false,
            details: BTreeMap::new(),
        }
    }
    pub(crate) fn validation(code: &str) -> Self {
        Self {
            category: IpcErrorCategory::Validation,
            ..Self::local(code)
        }
    }
    pub(crate) const fn exit_code(&self) -> u8 {
        match self.category {
            IpcErrorCategory::Validation => 2,
            IpcErrorCategory::Authentication | IpcErrorCategory::Authorization => 3,
            IpcErrorCategory::DependencyUnavailable => 4,
            _ => 1,
        }
    }
}
impl From<BridgeToolFailure> for CliFailure {
    fn from(error: BridgeToolFailure) -> Self {
        Self {
            code: error.code().into(),
            category: error.category(),
            retryable: error.retryable(),
            details: error.details().clone(),
        }
    }
}

pub(crate) fn write_json(value: &impl Serialize) -> CliResult<()> {
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, value).map_err(|_| CliFailure::local("cli.output_failed"))?;
    out.write_all(b"\n")
        .and_then(|()| out.flush())
        .map_err(|_| CliFailure::local("cli.output_failed"))
}
pub(crate) fn success(value: impl Serialize) -> CliResult<()> {
    write_json(&json!({"ok": true, "data": value}))
}
