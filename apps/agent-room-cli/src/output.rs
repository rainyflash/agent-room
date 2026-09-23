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
    pub(crate) hint: &'static str,
}

impl CliFailure {
    pub(crate) fn local(code: &str) -> Self {
        Self {
            code: code.into(),
            category: IpcErrorCategory::Internal,
            retryable: false,
            details: BTreeMap::new(),
            hint: error_hint(code),
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
            hint: error_hint(error.code()),
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

impl From<agent_room_agent_reception::ReceptionFailure> for CliFailure {
    fn from(error: agent_room_agent_reception::ReceptionFailure) -> Self {
        let hint = error_hint(&error.code);
        Self {
            code: error.code,
            category: error.category,
            retryable: error.retryable,
            details: error.details,
            hint,
        }
    }
}

fn error_hint(code: &str) -> &'static str {
    match code {
        "cli.profile.reader_busy" => {
            "This profile already has an active reader. Stop that reader before another read/listen. Send and ack remain available during a stream."
        }
        "cli.profile.required" | "cli.session_required" => {
            "Run join --room <name> (see rooms) or join with the room invitation, then pass its --profile value. Existing scripts may still pass --session."
        }
        "cli.room_not_found" => {
            "No room with that name or slug is available to this account. Run rooms and use a listed name exactly; do not guess another room or fall back silently."
        }
        "cli.room_ambiguous" => {
            "Several rooms match that name. Pick one of the listed candidates by its slug or exact name, or ask the person which room they meant."
        }
        "cli.profile.invitation_mismatch" => {
            "This profile already belongs to a different room or invitation. Keep using it for its room, or omit --profile so join can pick or create the identity for the requested room."
        }
        "cli.profile.not_found" => {
            "Run the original join invitation on this computer. Do not generate a replacement identity."
        }
        "cli.profile.corrupt"
        | "cli.profile.read_failed"
        | "cli.profile.write_failed"
        | "cli.profile.storage_unavailable" => {
            "Saved task data could not be read or written. Check the data directory and preserve the original invitation; do not delete data to force a new identity."
        }
        "cli.profile.task_mismatch" => {
            "This profile belongs to a different host task. Continue the original task or invite a separate character for this task."
        }
        "cli.profile.identity_changed" => {
            "The authenticated account returned a different character. Restore the original account or use a new invitation; the saved identity was not replaced."
        }
        "cli.profile.busy" => {
            "Another command is using this profile. Wait for it to finish, then retry the same command and submission ID."
        }
        "cli.profile.event_not_delivered"
        | "cli.profile.cursor_mismatch"
        | "cli.profile.ack_required" => {
            "Use read to receive a batch, handle it, then ack its last handled eventId. Do not skip messages or reset the cursor."
        }
        "cli.invitation.room_mismatch" | "cli.profile.room_mismatch" => {
            "The connected room differs from the invitation. Check actualRoomId and the selected lobby. Do not report this invitation as successful."
        }
        "cli.task_id_required" => {
            "Pass the exact --task-id and --host for registration. Codex may supply CODEX_THREAD_ID and Claude Code CLAUDE_CODE_SESSION_ID; never guess the ID or select the latest task."
        }
        "cli.connection.starting" => {
            "The identity is saved but still connecting. Keep Agent Room running and retry resume with the same --profile."
        }
        "cli.invitation_version_unsupported" => {
            "Update the CLI to the version installed with Agent Room, then retry the same invitation."
        }
        value
            if value.starts_with("bridge.ipc.credentials_")
                || value == "bridge.host_session.device_authorization_required" =>
        {
            "Open Agent Room on this computer and finish local agent authorization. Reuse its existing login; never paste credentials into an agent conversation."
        }
        value if value.starts_with("bridge.ipc.") => {
            "Check doctor with the same --data-root and --connection. Keep the desktop or headless Bridge running and use matching CLI/Bridge versions."
        }
        _ => {
            "Read --help or guide for this command. Preserve the task identity and original submission ID when retrying."
        }
    }
}
