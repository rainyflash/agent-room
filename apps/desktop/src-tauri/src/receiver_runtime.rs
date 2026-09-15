use crate::{
    commands::{DesktopCommandFailure, DesktopRuntime},
    desktop_config::DesktopBridgeConfig,
};
use agent_room_agent_client::LocalBridgeToolClient;
use agent_room_agent_client::reception::ReceptionPolicy;
use agent_room_agent_reception::{
    HostBinding, ReceiverBinding, ReceiverContext, ReceiverEvent, ReceiverMode, ReceiverStart,
    ReceiverState, ReceiverStore, ReceptionFailure, Resolution,
};
use agent_room_bridge_ipc::{IpcMethod, IpcOpenHostSessionRequest, IpcResponse};
use agent_room_bridge_local_adapter::LocalBridgeClient;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Mutex, RwLock, watch},
    task::JoinHandle,
};

type Result<T> = std::result::Result<T, ReceptionFailure>;

#[derive(Clone)]
pub(crate) struct ReceiverRuntime {
    inner: Arc<RuntimeInner>,
}
struct RuntimeInner {
    config: DesktopBridgeConfig,
    mcp_executable: PathBuf,
    workers: Mutex<BTreeMap<String, Worker>>,
    exiting: AtomicBool,
}
struct Worker {
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<Result<()>>>,
    progress: Arc<RwLock<Option<ReceiverEvent>>>,
    failure: Option<ReceptionFailure>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReceiverView {
    state: ReceiverState,
    running: bool,
    progress: Option<ReceiverEvent>,
    failure: Option<ReceptionFailure>,
}

impl ReceiverRuntime {
    pub(crate) fn new(config: DesktopBridgeConfig, mcp_executable: PathBuf) -> Self {
        Self {
            inner: Arc::new(RuntimeInner {
                config,
                mcp_executable,
                workers: Mutex::new(BTreeMap::new()),
                exiting: AtomicBool::new(false),
            }),
        }
    }
    pub(crate) fn begin_shutdown(&self) -> bool {
        !self.inner.exiting.swap(true, Ordering::AcqRel)
    }

    pub(crate) async fn restore(&self) -> Result<()> {
        for state in ReceiverStore::list(&self.inner.config.data_root())? {
            if state.enabled
                && let Err(error) = self
                    .start(&state.binding.host.task_id, ReceiverMode::Listen)
                    .await
            {
                let (stop, _) = watch::channel(false);
                self.inner.workers.lock().await.insert(
                    state.binding.host.task_id,
                    Worker {
                        stop,
                        task: None,
                        progress: Arc::new(RwLock::new(None)),
                        failure: Some(error),
                    },
                );
            }
        }
        Ok(())
    }
    async fn views(&self) -> Result<Vec<ReceiverView>> {
        let states = ReceiverStore::list(&self.inner.config.data_root())?;
        let mut workers = self.inner.workers.lock().await;
        let mut views = Vec::with_capacity(states.len());
        for state in states {
            let worker = workers.get_mut(&state.binding.host.task_id);
            let (running, progress, failure) = if let Some(worker) = worker {
                if worker.task.as_ref().is_some_and(JoinHandle::is_finished) {
                    let task = worker
                        .task
                        .take()
                        .ok_or_else(|| ReceptionFailure::local("receiver.worker_missing"))?;
                    worker.failure = match task.await {
                        Ok(result) => result.err(),
                        Err(_) => Some(ReceptionFailure::local("receiver.worker_failed")),
                    };
                }
                (
                    worker.task.is_some(),
                    worker.progress.read().await.clone(),
                    worker.failure.clone(),
                )
            } else {
                (false, None, None)
            };
            views.push(ReceiverView {
                state,
                running,
                progress,
                failure,
            });
        }
        Ok(views)
    }
    async fn start(&self, task_id: &str, mode: ReceiverMode) -> Result<()> {
        if self.inner.exiting.load(Ordering::Acquire) {
            return Err(ReceptionFailure::local("receiver.shutting_down"));
        }
        let mut workers = self.inner.workers.lock().await;
        if workers
            .get(task_id)
            .and_then(|worker| worker.task.as_ref())
            .is_some_and(|task| !task.is_finished())
        {
            return Ok(());
        }
        let store = ReceiverStore::open(&self.inner.config.data_root(), task_id)?;
        let mut state = store
            .load()?
            .ok_or_else(|| ReceptionFailure::local("receiver.state_missing"))?;
        if mode == ReceiverMode::Listen {
            state.binding.host.validate()?;
        }
        if mode == ReceiverMode::Listen {
            state.enabled = true;
        }
        store.save(&state)?;
        drop(store);
        let (stop, mut stopped) = watch::channel(false);
        let progress = Arc::new(RwLock::new(None));
        let progress_writer = progress.clone();
        let config = self.inner.config.clone();
        let task_id_owned = task_id.to_owned();
        let task = tokio::spawn(async move {
            let (event_sender, mut events) = tokio::sync::mpsc::unbounded_channel();
            let emit = |event| {
                event_sender
                    .send(event)
                    .map_err(|_| ReceptionFailure::local("receiver.observer_closed"))
            };
            let backend = LocalBridgeToolClient::agent_cli(
                config.runtime_root(),
                config.secure_storage_service(),
            );
            let data_root = config.data_root();
            let service = config.secure_storage_service();
            let operation = agent_room_agent_reception::run(
                ReceiverContext {
                    host: &agent_room_agent_reception::NativeHost,
                    mode,
                    backend: &backend,
                    data_root: &data_root,
                    service: service.as_str(),
                    emit: &emit,
                },
                &task_id_owned,
                async {
                    let _ = stopped.wait_for(|stop| *stop).await;
                },
            );
            tokio::pin!(operation);
            loop {
                tokio::select! {
                    result = &mut operation => return result,
                    event = events.recv() => if let Some(event) = event { *progress_writer.write().await = Some(event); },
                }
            }
        });
        workers.insert(
            task_id.to_owned(),
            Worker {
                stop,
                task: Some(task),
                progress,
                failure: None,
            },
        );
        Ok(())
    }

    async fn pause(&self, task_id: &str, persist: bool) -> Result<()> {
        let mut workers = self.inner.workers.lock().await;
        let mut stop_failure = None;
        if let Some(worker) = workers.get_mut(task_id) {
            worker.stop.send_replace(true);
            if let Some(mut task) = worker.task.take() {
                match tokio::time::timeout(Duration::from_secs(135), &mut task).await {
                    Ok(Ok(result)) => worker.failure = result.err(),
                    Ok(Err(_)) => {
                        worker.failure = Some(ReceptionFailure::local("receiver.worker_failed"));
                    }
                    Err(_) => {
                        task.abort();
                        // Await cancellation so the durable receiver lock is released before editing state.
                        if let Err(error) = task.await
                            && !error.is_cancelled()
                        {
                            return Err(ReceptionFailure::local("receiver.worker_failed"));
                        }
                        worker.failure = Some(ReceptionFailure::local("receiver.stop_timeout"));
                    }
                }
            }
            stop_failure = worker.failure.clone();
        }
        let store = ReceiverStore::open(&self.inner.config.data_root(), task_id)?;
        let mut state = store
            .load()?
            .ok_or_else(|| ReceptionFailure::local("receiver.state_missing"))?;
        if persist {
            state.enabled = false;
            store.save(&state)?;
        }
        // A failed host turn does not prevent manual takeover when the receiver
        // has drained its session and confirmed release with the server.
        if state.execution.is_some()
            || stop_failure.as_ref().is_some_and(|error| {
                error.details.contains_key("closeError")
                    || matches!(
                        error.code.as_str(),
                        "receiver.stop_timeout" | "receiver.worker_failed"
                    )
            })
        {
            return Err(stop_failure
                .unwrap_or_else(|| ReceptionFailure::local("receiver.release_unconfirmed")));
        }
        Ok(())
    }

    pub(crate) async fn shutdown(&self) -> Result<()> {
        let task_ids: Vec<_> = self.inner.workers.lock().await.keys().cloned().collect();
        let mut failure = None;
        for task_id in task_ids {
            if let Err(error) = self.pause(&task_id, false).await {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }

    async fn configure(&self, request: ConfigureReceiver) -> Result<()> {
        let config = &self.inner.config;
        let client = LocalBridgeClient::desktop_shell_with_secure_storage_service(
            config.runtime_root(),
            config.secure_storage_service(),
        );
        let IpcResponse::HostSessionDiagnostics { sessions } = client
            .invoke(IpcMethod::HostSessionDiagnostics)
            .await
            .map_err(|_| ReceptionFailure::local("receiver.sessions_unavailable"))?
        else {
            return Err(ReceptionFailure::local("receiver.response_invalid"));
        };
        let session = sessions
            .into_iter()
            .find(|s| s.session.session_id == request.session_id)
            .ok_or_else(|| ReceptionFailure::validation("receiver.session_not_found"))?;
        let offer = session
            .reception_offer
            .ok_or_else(|| ReceptionFailure::validation("receiver.task_not_registered"))?;
        let executable = if let Some(path) = request.executable {
            path
        } else {
            let host_type = offer.task.host_type;
            let mcp = self.inner.mcp_executable.clone();
            tokio::task::spawn_blocking(move || discover_host(host_type, mcp))
                .await
                .map_err(|_| ReceptionFailure::local("receiver.host_detection_failed"))??
        };
        let binding = ReceiverBinding {
            session: IpcOpenHostSessionRequest {
                session_key: session
                    .session_key
                    .ok_or_else(|| ReceptionFailure::local("receiver.session_key_missing"))?,
                display_name: session.display_name,
                room: session.requested_room,
            },
            policy: ReceptionPolicy {
                room_id: offer.room_id,
                allowed_principal_id: request.principal_id,
            },
            automation_grant_id: request.automation_grant_id,
            host: HostBinding {
                host_type: offer.task.host_type,
                task_id: offer.task.task_id,
                executable,
                mcp_executable: self.inner.mcp_executable.clone(),
                workspace: offer.task.workspace.into(),
            },
            start: ReceiverStart::Now,
        };
        binding.host.validate()?;
        let store = ReceiverStore::open(&config.data_root(), &binding.host.task_id)?;
        let mut state = store.configure(binding, config.secure_storage_service().as_str())?;
        state.room_catalog_id = offer.room_catalog_id;
        state.instance_id = Some(offer.instance_id);
        store.save(&state)?;
        Ok(())
    }
}

fn discover_host(host: agent_room_bridge_ipc::IpcReceptionHost, mcp: PathBuf) -> Result<PathBuf> {
    use agent_room_host_adapters::{HostConfigurator, HostContext, HostKind};
    let context = HostContext::from_environment(mcp)
        .map_err(|error| ReceptionFailure::local(error.code()))?;
    HostConfigurator::system(context)
        .reception_executable(match host {
            agent_room_bridge_ipc::IpcReceptionHost::Codex => HostKind::Codex,
            agent_room_bridge_ipc::IpcReceptionHost::ClaudeCode => HostKind::ClaudeCode,
        })
        .map_err(|error| ReceptionFailure::local(error.code()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ConfigureReceiver {
    session_id: String,
    principal_id: String,
    automation_grant_id: String,
    executable: Option<PathBuf>,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ReceiverAction {
    Start,
    Verify,
    Pause,
    Remove,
    Resolve {
        event: String,
        resolution: Resolution,
    },
    Update {
        #[serde(rename = "automationGrantId")]
        automation_grant_id: String,
        executable: Option<PathBuf>,
        workspace: Option<PathBuf>,
    },
}

#[tauri::command]
pub(crate) async fn desktop_receiver_list(
    runtime: tauri::State<'_, DesktopRuntime>,
) -> std::result::Result<Vec<ReceiverView>, DesktopCommandFailure> {
    runtime.receivers.views().await.map_err(Into::into)
}
#[tauri::command]
pub(crate) async fn desktop_receiver_configure(
    runtime: tauri::State<'_, DesktopRuntime>,
    request: ConfigureReceiver,
) -> std::result::Result<(), DesktopCommandFailure> {
    runtime
        .receivers
        .configure(request)
        .await
        .map_err(Into::into)
}
#[tauri::command]
pub(crate) async fn desktop_receiver_action(
    runtime: tauri::State<'_, DesktopRuntime>,
    task_id: String,
    request: ReceiverAction,
) -> std::result::Result<(), DesktopCommandFailure> {
    let receivers = &runtime.receivers;
    match request {
        ReceiverAction::Start => receivers.start(&task_id, ReceiverMode::Listen).await?,
        ReceiverAction::Verify => {
            receivers
                .start(&task_id, ReceiverMode::VerifyReceipt)
                .await?;
        }
        ReceiverAction::Pause => receivers.pause(&task_id, true).await?,
        ReceiverAction::Remove => {
            receivers.pause(&task_id, true).await?;
            ReceiverStore::open(&receivers.inner.config.data_root(), &task_id)?.remove()?;
            receivers.inner.workers.lock().await.remove(&task_id);
        }
        request => {
            let store = ReceiverStore::open(&receivers.inner.config.data_root(), &task_id)?;
            match request {
                ReceiverAction::Resolve { event, resolution } => {
                    store.resolve(&event, resolution)?;
                    drop(store);
                    if matches!(resolution, Resolution::Retry) {
                        receivers.start(&task_id, ReceiverMode::Listen).await?;
                    }
                }
                ReceiverAction::Update {
                    automation_grant_id,
                    executable,
                    workspace,
                } => {
                    let state = store
                        .load()?
                        .ok_or_else(|| ReceptionFailure::local("receiver.state_missing"))?;
                    let mut binding = state.binding;
                    binding.automation_grant_id = automation_grant_id;
                    if let Some(path) = executable {
                        binding.host.executable = path;
                    }
                    if let Some(path) = workspace {
                        binding.host.workspace = path;
                    }
                    binding
                        .host
                        .mcp_executable
                        .clone_from(&receivers.inner.mcp_executable);
                    binding.host.validate()?;
                    store.configure(binding, &state.bridge_service)?;
                }
                ReceiverAction::Start
                | ReceiverAction::Verify
                | ReceiverAction::Pause
                | ReceiverAction::Remove => unreachable!(),
            }
        }
    }
    Ok(())
}
