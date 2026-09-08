use crate::{
    call,
    cli::{ReceiverCommand, Resolution},
    codex::CodexBinding,
    output::{CliFailure, CliResult, success, write_json},
    scoped,
};
use agent_room_agent_client::{
    BridgeToolClient, MessageReadMode,
    reception::{DeliveryDecision, ReceptionCheckpoint, ReceptionPolicy},
    wait_for_messages,
};
use agent_room_bridge_ipc::{
    IpcBridgeState, IpcCloseHostSessionRequest, IpcHostSessionState, IpcListPreviewsRequest,
    IpcMethod, IpcOpenHostSessionRequest, IpcResponse, IpcSelfSummary,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReceiverBinding {
    session: IpcOpenHostSessionRequest,
    policy: ReceptionPolicy,
    automation_grant_id: String,
    host: CodexBinding,
    start: ReceiverStart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum ReceiverStart {
    Now,
    Beginning,
    After {
        #[serde(rename = "eventId")]
        event_id: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReceiverState {
    binding: ReceiverBinding,
    bridge_service: String,
    agent_id: String,
    checkpoint: ReceptionCheckpoint,
}

struct ReceiverStore {
    _lock: File,
    path: PathBuf,
}
impl ReceiverStore {
    fn open(data_root: &Path, task_id: &str) -> CliResult<Self> {
        let directory = data_root.join("receivers");
        fs::create_dir_all(&directory)
            .map_err(|_| CliFailure::local("receiver.storage_unavailable"))?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join(format!("{task_id}.lock")))
            .map_err(|_| CliFailure::local("receiver.storage_unavailable"))?;
        lock.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => CliFailure::local("receiver.already_running"),
            fs::TryLockError::Error(_) => CliFailure::local("receiver.lock_unavailable"),
        })?;
        Ok(Self {
            _lock: lock,
            path: directory.join(format!("{task_id}.json")),
        })
    }
    fn load(&self) -> CliResult<Option<ReceiverState>> {
        match File::open(&self.path) {
            Ok(file) => read_json(file).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(CliFailure::local("receiver.storage_unavailable")),
        }
    }
    fn save(&self, state: &ReceiverState) -> CliResult<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| CliFailure::local("receiver.storage_path_invalid"))?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|_| CliFailure::local("receiver.storage_unavailable"))?;
        serde_json::to_writer(&mut temporary, state)
            .map_err(|_| CliFailure::local("receiver.checkpoint_write_failed"))?;
        temporary
            .flush()
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|_| CliFailure::local("receiver.checkpoint_write_failed"))?;
        temporary
            .persist(&self.path)
            .map_err(|_| CliFailure::local("receiver.checkpoint_write_failed"))?;
        #[cfg(unix)]
        File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|_| CliFailure::local("receiver.checkpoint_write_failed"))?;
        Ok(())
    }
}

fn read_json<T: serde::de::DeserializeOwned>(file: File) -> CliResult<T> {
    let mut bytes = Vec::new();
    file.take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|_| CliFailure::local("receiver.config_read_failed"))?;
    if bytes.len() > 65_536 {
        return Err(CliFailure::validation("receiver.config_too_large"));
    }
    serde_json::from_slice(&bytes).map_err(|_| CliFailure::validation("receiver.config_invalid"))
}

fn load_binding(path: &Path) -> CliResult<ReceiverBinding> {
    let binding: ReceiverBinding =
        read_json(File::open(path).map_err(|_| CliFailure::local("receiver.config_read_failed"))?)?;
    let task = uuid::Uuid::parse_str(&binding.host.task_id)
        .map_err(|_| CliFailure::validation("receiver.task_id_invalid"))?;
    if task.is_nil() || task.to_string() != binding.host.task_id {
        return Err(CliFailure::validation("receiver.task_id_invalid"));
    }
    IpcMethod::OpenHostSession(binding.session.clone())
        .validate()
        .map_err(|error| CliFailure::validation(error.code()))?;
    let grant = uuid::Uuid::parse_str(&binding.automation_grant_id)
        .map_err(|_| CliFailure::validation("receiver.automation_grant_invalid"))?;
    if grant.get_version() != Some(uuid::Version::SortRand)
        || grant.to_string() != binding.automation_grant_id
    {
        return Err(CliFailure::validation("receiver.automation_grant_invalid"));
    }
    let principal = uuid::Uuid::parse_str(&binding.policy.allowed_principal_id)
        .map_err(|_| CliFailure::validation("receiver.principal_invalid"))?;
    if principal.is_nil() || principal.to_string() != binding.policy.allowed_principal_id {
        return Err(CliFailure::validation("receiver.principal_invalid"));
    }
    IpcMethod::ReadInbox(inbox_request(
        &binding,
        match &binding.start {
            ReceiverStart::After { event_id } => Some(event_id.clone()),
            _ => None,
        },
    ))
    .validate()
    .map_err(|error| CliFailure::validation(error.code()))?;
    Ok(binding)
}

fn inbox_request(
    binding: &ReceiverBinding,
    after_event_id: Option<String>,
) -> IpcListPreviewsRequest {
    IpcListPreviewsRequest {
        room_id: Some(binding.policy.room_id.clone()),
        after_event_id,
        before_event_id: None,
        limit: 20,
    }
}

pub(crate) async fn run(
    backend: &dyn BridgeToolClient,
    data_root: &Path,
    service: &str,
    path: &Path,
) -> CliResult<()> {
    let binding = load_binding(path)?;
    binding.host.validate()?;
    let store = ReceiverStore::open(data_root, &binding.host.task_id)?;
    let mut state = store.load()?;
    if let Some(state) = &state {
        if state.binding != binding || state.bridge_service != service {
            return Err(CliFailure::validation("receiver.binding_changed"));
        }
        if matches!(state.checkpoint, ReceptionCheckpoint::Pending { .. }) {
            return Err(CliFailure::local("receiver.pending_review_required"));
        }
    }
    let mut session_id = None;
    let result = tokio::select! {
        result = receive_loop(backend, data_root, service, &binding, &store, &mut state, &mut session_id) => result,
        signal = tokio::signal::ctrl_c() => signal.map_err(|_| CliFailure::local("cli.signal_failed")),
    };
    if let Some(session_id) = session_id {
        let close = call(
            backend,
            IpcMethod::CloseHostSession(IpcCloseHostSessionRequest { session_id }),
        )
        .await;
        if let Err(mut error) = result {
            if let Err(close_error) = close {
                error.details.insert("closeError".into(), close_error.code);
            }
            return Err(error);
        }
        close?;
    } else {
        result?;
    }
    success(json!({"type": "receiver_stopped"}))
}

async fn receive_loop(
    backend: &dyn BridgeToolClient,
    data_root: &Path,
    service: &str,
    binding: &ReceiverBinding,
    store: &ReceiverStore,
    state: &mut Option<ReceiverState>,
    session_id: &mut Option<String>,
) -> CliResult<()> {
    let mut delay = 1;
    loop {
        match receive_connected(
            backend, data_root, service, binding, store, state, session_id,
        )
        .await
        {
            Err(error) if error.retryable => {
                write_json(&json!({"ok": false, "error": error, "retryInSeconds": delay}))?;
                tokio::time::sleep(Duration::from_secs(delay)).await;
                delay = (delay * 2).min(30);
            }
            result => return result,
        }
    }
}

async fn receive_connected(
    backend: &dyn BridgeToolClient,
    data_root: &Path,
    service: &str,
    binding: &ReceiverBinding,
    store: &ReceiverStore,
    state: &mut Option<ReceiverState>,
    session_id: &mut Option<String>,
) -> CliResult<()> {
    let IpcResponse::HostSession { session } =
        call(backend, IpcMethod::OpenHostSession(binding.session.clone())).await?
    else {
        return Err(CliFailure::local("receiver.response_invalid"));
    };
    *session_id = Some(session.session_id.clone());
    if matches!(
        session.state,
        IpcHostSessionState::Failed | IpcHostSessionState::Closed
    ) {
        return Err(CliFailure::local(
            session
                .error_code
                .as_deref()
                .unwrap_or("receiver.session_failed"),
        ));
    }
    let summary = wait_until_ready(backend, &session.session_id).await?;
    if let Some(existing) = state.as_ref() {
        if existing.agent_id != summary.agent.agent_id {
            return Err(CliFailure::local("receiver.identity_changed"));
        }
    } else {
        let after_event_id = initial_cursor(backend, binding, &session.session_id).await?;
        let initial = ReceiverState {
            binding: binding.clone(),
            bridge_service: service.into(),
            agent_id: summary.agent.agent_id.clone(),
            checkpoint: ReceptionCheckpoint::Ready { after_event_id },
        };
        store.save(&initial)?;
        *state = Some(initial);
    }
    let state = state
        .as_mut()
        .ok_or_else(|| CliFailure::local("receiver.state_missing"))?;
    success(
        json!({"type": "receiver_ready", "agentId": summary.agent.agent_id, "hostTaskId": binding.host.task_id, "roomId": binding.policy.room_id}),
    )?;
    loop {
        let result = wait_for_messages(
            backend,
            session.session_id.clone(),
            inbox_request(binding, state.checkpoint.cursor().map(str::to_owned)),
            MessageReadMode::Inbox,
            25,
        )
        .await;
        let response = match result {
            Err(error)
                if matches!(
                    error.code(),
                    "bridge.host_session.not_found" | "bridge.host_session.closed"
                ) =>
            {
                let mut error = CliFailure::from(error);
                error.retryable = true;
                return Err(error);
            }
            result => result?,
        };
        let IpcResponse::MessagePreviews { previews, .. } = response else {
            return Err(CliFailure::local("receiver.response_invalid"));
        };
        for message in previews {
            let delivered = deliver_message(
                state,
                store,
                &message,
                &summary.agent.matrix_user_id,
                || {
                    crate::codex::resume(
                        &binding.host,
                        data_root,
                        service,
                        &session.session_id,
                        &binding.automation_grant_id,
                        &message,
                    )
                },
            )
            .await?;
            if delivered {
                success(
                    json!({"type": "host_turn_completed", "eventId": message.event_id, "hostTaskId": binding.host.task_id}),
                )?;
            }
        }
    }
}

async fn wait_until_ready(
    backend: &dyn BridgeToolClient,
    session_id: &str,
) -> CliResult<IpcSelfSummary> {
    let deadline = tokio::time::Instant::now() + Duration::from_mins(2);
    loop {
        match call(backend, scoped(session_id.into(), IpcMethod::GetSelf)).await {
            Ok(IpcResponse::SelfSummary { summary })
                if summary.connection_state == IpcBridgeState::Ready =>
            {
                return Ok(summary);
            }
            Ok(IpcResponse::SelfSummary { .. }) => {}
            Ok(_) => return Err(CliFailure::local("receiver.response_invalid")),
            Err(error) if error.retryable => {}
            Err(error) => return Err(error),
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(CliFailure::local("receiver.session_start_timeout"));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

pub(crate) fn manage(data_root: &Path, action: ReceiverCommand) -> CliResult<()> {
    let path = match &action {
        ReceiverCommand::Inspect { binding } | ReceiverCommand::Resolve { binding, .. } => binding,
    };
    let binding = load_binding(path)?;
    if matches!(&action, ReceiverCommand::Inspect { .. }) {
        let path = data_root
            .join("receivers")
            .join(format!("{}.json", binding.host.task_id));
        let state: ReceiverState =
            read_json(File::open(path).map_err(|_| CliFailure::local("receiver.state_missing"))?)?;
        if state.binding != binding {
            return Err(CliFailure::validation("receiver.binding_changed"));
        }
        return success(state);
    }
    let store = ReceiverStore::open(data_root, &binding.host.task_id)?;
    let mut state = store
        .load()?
        .ok_or_else(|| CliFailure::local("receiver.state_missing"))?;
    if state.binding != binding {
        return Err(CliFailure::validation("receiver.binding_changed"));
    }
    if let ReceiverCommand::Resolve { event, action, .. } = action {
        let ReceptionCheckpoint::Pending {
            after_event_id,
            event_id,
        } = &state.checkpoint
        else {
            return Err(CliFailure::validation("receiver.no_pending_delivery"));
        };
        if event != *event_id {
            return Err(CliFailure::validation("receiver.event_mismatch"));
        }
        state.checkpoint = ReceptionCheckpoint::Ready {
            after_event_id: match action {
                Resolution::Retry => after_event_id.clone(),
                Resolution::Skip => Some(event),
            },
        };
        store.save(&state)?;
    }
    success(state)
}

async fn deliver_message<F, Fut>(
    state: &mut ReceiverState,
    store: &ReceiverStore,
    message: &agent_room_bridge_ipc::IpcMessagePreviewSummary,
    agent_matrix_user_id: &str,
    deliver: F,
) -> CliResult<bool>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = CliResult<()>>,
{
    let decision = state
        .checkpoint
        .prepare(message, &state.binding.policy, agent_matrix_user_id)
        .map_err(|_| CliFailure::local("receiver.pending_review_required"))?;
    store.save(state)?;
    if decision == DeliveryDecision::Skip {
        return Ok(false);
    }
    deliver().await?;
    state
        .checkpoint
        .complete(&message.event_id)
        .map_err(|_| CliFailure::local("receiver.checkpoint_mismatch"))?;
    store.save(state)?;
    Ok(true)
}

async fn initial_cursor(
    backend: &dyn BridgeToolClient,
    binding: &ReceiverBinding,
    session_id: &str,
) -> CliResult<Option<String>> {
    match &binding.start {
        ReceiverStart::After { event_id } => Ok(Some(event_id.clone())),
        ReceiverStart::Beginning => Ok(None),
        ReceiverStart::Now => {
            let mut request = inbox_request(binding, None);
            request.limit = 1;
            let IpcResponse::MessagePreviews { previews, .. } = call(
                backend,
                scoped(session_id.into(), IpcMethod::ListPreviews(request)),
            )
            .await?
            else {
                return Err(CliFailure::local("receiver.response_invalid"));
            };
            Ok(previews.first().map(|message| message.event_id.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> ReceiverState {
        let id = uuid::Uuid::now_v7().to_string();
        ReceiverState {
            binding: ReceiverBinding {
                session: IpcOpenHostSessionRequest {
                    session_key: id.clone(),
                    display_name: "Receiver".into(),
                },
                policy: ReceptionPolicy {
                    room_id: "!room:test".into(),
                    allowed_principal_id: id.clone(),
                },
                automation_grant_id: id.clone(),
                host: CodexBinding {
                    task_id: id.clone(),
                    executable: std::env::current_exe().unwrap(),
                    mcp_executable: std::env::current_exe().unwrap(),
                    workspace: std::env::current_dir().unwrap(),
                },
                start: ReceiverStart::Beginning,
            },
            bridge_service: "test.receiver".into(),
            agent_id: id,
            checkpoint: ReceptionCheckpoint::Ready {
                after_event_id: None,
            },
        }
    }
    fn message(state: &ReceiverState) -> agent_room_bridge_ipc::IpcMessagePreviewSummary {
        serde_json::from_value(json!({
            "conversation": {"text":"hello", "mentions":["@agent:test"]}, "replyToMessageId":null,
            "messageId":state.agent_id, "eventId":"$one", "roomId":"!room:test",
            "actor":{"kind":"human", "principalId":state.binding.policy.allowed_principal_id,"displayName":"Owner","matrixUserId":"@owner:test","avatarUrl":null},
            "createdAtUnixMs":1,"title":"hello","summary":"hello", "content":{"contentId":state.agent_id,"digestSha256":"0".repeat(64),"mediaType":"text/plain","sizeBytes":5},
            "language":null,"sensitivity":"normal","riskFlags":[]
        })).unwrap()
    }
    #[test]
    fn 同一宿主任务跨接收器独占且退出自动释放() {
        let directory = tempfile::tempdir().unwrap();
        let state = state();
        let first = ReceiverStore::open(directory.path(), &state.binding.host.task_id).unwrap();
        assert!(ReceiverStore::open(directory.path(), &state.binding.host.task_id).is_err());
        drop(first);
        assert!(ReceiverStore::open(directory.path(), &state.binding.host.task_id).is_ok());
    }
    #[tokio::test]
    async fn 启动宿主前持久化待处理状态且宿主失败后不确认游标() {
        let directory = tempfile::tempdir().unwrap();
        let mut state = state();
        let store = ReceiverStore::open(directory.path(), &state.binding.host.task_id).unwrap();
        let message = message(&state);
        let result = deliver_message(&mut state, &store, &message, "@agent:test", || async {
            assert!(matches!(
                store.load().unwrap().unwrap().checkpoint,
                ReceptionCheckpoint::Pending { .. }
            ));
            Err(CliFailure::local("test.host_failed"))
        })
        .await;
        assert_eq!(result.unwrap_err().code, "test.host_failed");
        assert!(matches!(
            store.load().unwrap().unwrap().checkpoint,
            ReceptionCheckpoint::Pending { .. }
        ));
        assert!(
            deliver_message(&mut state, &store, &message, "@agent:test", || async {
                panic!("不允许重复启动");
            })
            .await
            .is_err()
        );
    }
    #[tokio::test]
    async fn 明确完成后提交游标而重复事件不再启动宿主() {
        let directory = tempfile::tempdir().unwrap();
        let mut state = state();
        let store = ReceiverStore::open(directory.path(), &state.binding.host.task_id).unwrap();
        let message = message(&state);
        assert!(
            deliver_message(&mut state, &store, &message, "@agent:test", || async {
                Ok(())
            })
            .await
            .unwrap()
        );
        assert_eq!(
            store.load().unwrap().unwrap().checkpoint.cursor(),
            Some("$one")
        );
        assert!(
            !deliver_message(&mut state, &store, &message, "@agent:test", || async {
                panic!("同一事件不应重入");
            })
            .await
            .unwrap()
        );
    }
}
