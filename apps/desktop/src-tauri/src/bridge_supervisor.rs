use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use agent_room_bridge_ipc::{IpcBridgeState, IpcMethod, IpcResponse, IpcSelfSummary};
use agent_room_bridge_local_adapter::{LocalBridgeClient, LocalBridgeClientFailureKind};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter as _};
use tauri_plugin_notification::NotificationExt as _;
use tauri_plugin_shell::{
    ShellExt as _,
    process::{CommandChild, CommandEvent},
};
use tokio::sync::{mpsc, watch};
use url::Url;

use crate::{
    bridge_lifecycle::{
        BridgeLifecycleSnapshot, BridgeOwnership, BridgePhase, BridgeRestartPolicy,
        ConnectionProgress, ExitDecision, ResumeDecision, ResumeProbeState, decide_resume,
    },
    desktop_config::{BridgeLaunch, DesktopBridgeConfig},
};

const SUPERVISOR_CHANNEL: &str = "agent_room_desktop";
const RUNTIME_CHANGED_EVENT: &str = "desktop://runtime-changed";
const ACTOR_QUEUE_CAPACITY: usize = 32;
const PROBE_INTERVAL: Duration = Duration::from_secs(2);
const MAX_AUTHORIZATION_SECONDS: u64 = 30 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthorizationPromptView {
    pub(crate) prompt_id: String,
    pub(crate) verification_host: String,
    pub(crate) user_code: String,
    pub(crate) expires_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BridgeRuntimeView {
    pub(crate) lifecycle: BridgeLifecycleSnapshot,
    pub(crate) authorization: Option<AuthorizationPromptView>,
    pub(crate) session: Option<BridgeAgentSessionView>,
    /// 停机视图是否提供「重新授权这台电脑」。
    pub(crate) device_reauthorization_available: bool,
}

impl BridgeRuntimeView {
    fn new(
        lifecycle: BridgeLifecycleSnapshot,
        authorization: Option<AuthorizationPromptView>,
        session: Option<BridgeAgentSessionView>,
    ) -> Self {
        Self {
            device_reauthorization_available: lifecycle.device_reauthorization_available(),
            lifecycle,
            authorization,
            session,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct BridgeAgentSessionView {
    #[serde(rename = "agentId")]
    pub(crate) agent: String,
    #[serde(rename = "instanceId")]
    pub(crate) instance: String,
    #[serde(rename = "matrixRoomId")]
    pub(crate) matrix_room: String,
}

impl From<IpcSelfSummary> for BridgeAgentSessionView {
    fn from(summary: IpcSelfSummary) -> Self {
        Self {
            agent: summary.agent.agent_id,
            instance: summary.instance_id,
            matrix_room: summary.room_id,
        }
    }
}

#[derive(Debug, Clone)]
struct SupervisorState {
    view: BridgeRuntimeView,
    authorization_url: Option<Url>,
}

#[derive(Clone)]
pub(crate) struct BridgeSupervisor {
    input: mpsc::Sender<ActorInput>,
    state: watch::Receiver<SupervisorState>,
    child: Arc<Mutex<Option<CommandChild>>>,
    shutting_down: Arc<AtomicBool>,
}

impl BridgeSupervisor {
    pub(crate) fn start(app: AppHandle, config: DesktopBridgeConfig) -> Self {
        let policy = BridgeRestartPolicy::new(now_unix_ms());
        let initial = SupervisorState {
            view: BridgeRuntimeView::new(policy.snapshot().clone(), None, None),
            authorization_url: None,
        };
        let (state_tx, state_rx) = watch::channel(initial);
        let (input_tx, input_rx) = mpsc::channel(ACTOR_QUEUE_CAPACITY);
        let child = Arc::new(Mutex::new(None));
        let shutting_down = Arc::new(AtomicBool::new(false));
        let actor = BridgeSupervisorActor {
            app,
            config,
            policy,
            authorization: None,
            session: None,
            child: child.clone(),
            shutting_down: shutting_down.clone(),
            input: input_tx.clone(),
            receiver: input_rx,
            state: state_tx,
            generation: 0,
            managed_child_active: false,
        };
        tauri::async_runtime::spawn(actor.run());
        Self {
            input: input_tx,
            state: state_rx,
            child,
            shutting_down,
        }
    }

    pub(crate) fn snapshot(&self) -> BridgeRuntimeView {
        self.state.borrow().view.clone()
    }

    pub(crate) fn authorization_url(&self, prompt_id: &str) -> Result<Url, SupervisorFailure> {
        let state = self.state.borrow();
        let prompt_matches = state
            .view
            .authorization
            .as_ref()
            .is_some_and(|prompt| prompt.prompt_id == prompt_id);
        if !prompt_matches {
            return Err(SupervisorFailure::new(
                "desktop.authorization.prompt_stale",
                false,
            ));
        }
        state
            .authorization_url
            .clone()
            .ok_or_else(|| SupervisorFailure::new("desktop.authorization.prompt_unavailable", true))
    }

    pub(crate) fn retry(&self) -> Result<(), SupervisorFailure> {
        self.input
            .try_send(ActorInput::ExplicitRetry)
            .map_err(|_| SupervisorFailure::new("desktop.bridge.command_queue_busy", true))
    }

    /// 清除本机设备会话凭据并重新申请设备授权，只在停机视图提供。
    pub(crate) fn reauthorize_device(&self) -> Result<(), SupervisorFailure> {
        if !self.state.borrow().view.device_reauthorization_available {
            return Err(SupervisorFailure::new(
                "desktop.bridge.reauthorization_unavailable",
                false,
            ));
        }
        self.input
            .try_send(ActorInput::ReauthorizeDevice)
            .map_err(|_| SupervisorFailure::new("desktop.bridge.command_queue_busy", true))
    }

    pub(crate) fn ensure_reconfigurable(&self) -> Result<(), SupervisorFailure> {
        if self.state.borrow().view.lifecycle.ownership == Some(BridgeOwnership::External) {
            return Err(SupervisorFailure::new(
                "desktop.bridge.external_reconfigure_unsupported",
                false,
            ));
        }
        Ok(())
    }

    pub(crate) fn reconfigure(&self, config: DesktopBridgeConfig) -> Result<(), SupervisorFailure> {
        self.ensure_reconfigurable()?;
        self.input
            .try_send(ActorInput::Reconfigure {
                config: Box::new(config),
            })
            .map_err(|_| SupervisorFailure::new("desktop.bridge.command_queue_busy", true))
    }

    pub(crate) fn resume(&self) {
        let _ = self.input.try_send(ActorInput::Resume);
    }

    pub(crate) fn shutdown_now(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let _ = self.input.try_send(ActorInput::Shutdown);
        if let Ok(mut child) = self.child.lock()
            && let Some(child) = child.take()
        {
            let _ = child.kill();
        }
    }
}

struct BridgeSupervisorActor {
    app: AppHandle,
    config: DesktopBridgeConfig,
    policy: BridgeRestartPolicy,
    authorization: Option<AuthorizationPrompt>,
    session: Option<BridgeAgentSessionView>,
    child: Arc<Mutex<Option<CommandChild>>>,
    shutting_down: Arc<AtomicBool>,
    input: mpsc::Sender<ActorInput>,
    receiver: mpsc::Receiver<ActorInput>,
    state: watch::Sender<SupervisorState>,
    generation: u64,
    managed_child_active: bool,
}

impl BridgeSupervisorActor {
    async fn run(mut self) {
        match self.probe().await {
            ProbeOutcome::Authorized => {
                self.session = None;
                self.policy
                    .discovered_authorized(now_unix_ms(), BridgeOwnership::External);
                self.publish();
            }
            ProbeOutcome::Ready(session) => {
                self.session = Some(session);
                self.policy
                    .discovered_ready(now_unix_ms(), BridgeOwnership::External);
                self.publish();
            }
            ProbeOutcome::Pending(progress) => {
                self.session = None;
                self.policy
                    .discovered_pending(now_unix_ms(), BridgeOwnership::External, progress);
                self.publish();
            }
            ProbeOutcome::Absent => self.start_managed(),
            ProbeOutcome::Blocked(code) => {
                self.policy.halt(now_unix_ms(), code);
                self.publish();
            }
        }

        let mut probes = tokio::time::interval(PROBE_INTERVAL);
        probes.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let input = tokio::select! {
                input = self.receiver.recv() => match input { Some(input) => input, None => break },
                _ = probes.tick() => {
                    if !matches!(self.policy.snapshot().phase, BridgePhase::Halted | BridgePhase::Stopped | BridgePhase::RetryScheduled) {
                        if self.managed_child_active { self.handle_managed_probe().await; }
                        else { self.handle_external_probe().await; }
                    }
                    continue;
                }
            };
            match input {
                ActorInput::ExplicitRetry => self.handle_explicit_retry().await,
                ActorInput::ReauthorizeDevice => self.handle_reauthorize_device(),
                ActorInput::AutomaticRetry { generation } => {
                    if generation == self.generation
                        && self.policy.snapshot().phase == BridgePhase::RetryScheduled
                        && !self.managed_child_active
                    {
                        self.start_managed();
                    }
                }
                ActorInput::ProcessEvent { generation, event } => {
                    if generation == self.generation {
                        self.handle_process_event(event);
                    }
                }
                ActorInput::Reconfigure { config } => self.handle_reconfigure(*config),
                ActorInput::Resume => self.handle_resume().await,
                ActorInput::Shutdown => {
                    self.shutting_down.store(true, Ordering::SeqCst);
                    self.kill_managed_child();
                    self.policy.stop(now_unix_ms());
                    self.authorization = None;
                    self.session = None;
                    self.publish();
                    break;
                }
            }
        }
    }

    fn start_managed(&mut self) {
        self.start_managed_with(BridgeLaunch::Normal);
    }

    fn start_managed_with(&mut self, launch: BridgeLaunch) {
        if self.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        self.generation = self.generation.saturating_add(1);
        self.policy.starting(now_unix_ms());
        self.authorization = None;
        self.session = None;
        self.publish();
        let generation = self.generation;
        let spawned = self
            .app
            .shell()
            .sidecar("agent-room-bridge")
            .map(|command| command.envs(self.config.launch_environment(launch)))
            .and_then(tauri_plugin_shell::process::Command::spawn);
        let Ok((mut events, child)) = spawned else {
            self.policy.halt(
                now_unix_ms(),
                "desktop.bridge.sidecar_spawn_failed".to_owned(),
            );
            self.publish();
            return;
        };
        if let Ok(mut slot) = self.child.lock() {
            *slot = Some(child);
        } else {
            let _ = child.kill();
            self.policy.halt(
                now_unix_ms(),
                "desktop.bridge.child_state_unavailable".to_owned(),
            );
            self.publish();
            return;
        }
        self.managed_child_active = true;
        let event_sender = self.input.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(event) = events.recv().await {
                if event_sender
                    .send(ActorInput::ProcessEvent { generation, event })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
    }

    async fn handle_explicit_retry(&mut self) {
        if self.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        if matches!(
            self.policy.snapshot().phase,
            BridgePhase::Authorized | BridgePhase::Ready | BridgePhase::AuthorizationRequired
        ) {
            return;
        }
        if self.policy.snapshot().ownership == Some(BridgeOwnership::External) {
            // The desktop may inspect an external Bridge, but must not start a
            // competing process or terminate a runtime it does not own.
            self.handle_external_probe().await;
            return;
        }
        self.generation = self.generation.saturating_add(1);
        self.kill_managed_child();
        self.policy.explicit_retry(now_unix_ms());
        self.start_managed();
    }

    fn handle_reauthorize_device(&mut self) {
        // 请求排队期间状态可能已经变化；只对仍然停机的托管 Bridge 生效。
        if self.shutting_down.load(Ordering::SeqCst)
            || !self.policy.snapshot().device_reauthorization_available()
        {
            return;
        }
        // 先推进代次，让旧子进程迟到的事件失效。
        self.generation = self.generation.saturating_add(1);
        self.kill_managed_child();
        self.session = None;
        self.policy.explicit_retry(now_unix_ms());
        self.start_managed_with(BridgeLaunch::ReauthorizeDevice);
    }

    fn handle_reconfigure(&mut self, config: DesktopBridgeConfig) {
        if self.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        if self.policy.snapshot().ownership == Some(BridgeOwnership::External) {
            self.policy.set_diagnostic(
                now_unix_ms(),
                "desktop.bridge.external_reconfigure_unsupported".to_owned(),
            );
            self.publish();
            return;
        }

        // 先推进代次，让旧子进程迟到的 Terminated 事件失效，避免它污染新进程状态。
        self.generation = self.generation.saturating_add(1);
        self.kill_managed_child();
        self.config = config;
        self.session = None;
        self.policy.explicit_retry(now_unix_ms());
        self.start_managed();
    }

    async fn handle_resume(&mut self) {
        let probe = self.probe().await;
        if let ProbeOutcome::Pending(progress) = &probe {
            self.session = None;
            let ownership = if self.managed_child_active {
                BridgeOwnership::Managed
            } else {
                BridgeOwnership::External
            };
            self.policy
                .discovered_pending(now_unix_ms(), ownership, *progress);
            self.publish();
            return;
        }
        let probe_state = match probe {
            ProbeOutcome::Authorized => ResumeProbeState::Authorized,
            ProbeOutcome::Ready(_) => ResumeProbeState::Ready,
            ProbeOutcome::Absent => ResumeProbeState::Absent,
            ProbeOutcome::Blocked(_) => ResumeProbeState::Blocked,
            ProbeOutcome::Pending(_) => unreachable!("等待态已提前处理"),
        };
        match decide_resume(
            probe_state,
            self.managed_child_active,
            self.policy.snapshot().phase,
        ) {
            ResumeDecision::Authorized(ownership) => {
                self.session = None;
                self.policy.discovered_authorized(now_unix_ms(), ownership);
                self.publish();
            }
            ResumeDecision::Ready(ownership) => {
                let ProbeOutcome::Ready(session) = probe else {
                    unreachable!("就绪决策必须携带 Agent 会话")
                };
                self.session = Some(session);
                self.policy.discovered_ready(now_unix_ms(), ownership);
                self.publish();
            }
            ResumeDecision::StartManaged => self.start_managed(),
            ResumeDecision::KeepProbing => {
                // 子进程还在而阶段是 RetryScheduled，说明 Bridge 正按自己的退避等服务器恢复；
                // 保留这个状态，不要用探测诊断盖掉它。
                if matches!(
                    self.policy.snapshot().phase,
                    BridgePhase::Halted | BridgePhase::RetryScheduled
                ) {
                    return;
                }
                self.policy.set_diagnostic(
                    now_unix_ms(),
                    "desktop.bridge.resume_probe_pending".to_owned(),
                );
                self.publish();
            }
            ResumeDecision::Halt => {
                let ProbeOutcome::Blocked(code) = probe else {
                    unreachable!("阻断恢复决策只能来自阻断探测")
                };
                self.policy.halt(now_unix_ms(), code);
                self.publish();
            }
        }
    }

    async fn handle_managed_probe(&mut self) {
        match self.probe().await {
            ProbeOutcome::Authorized => {
                self.session = None;
                self.policy
                    .discovered_authorized(now_unix_ms(), BridgeOwnership::Managed);
                self.authorization = None;
                self.publish();
            }
            ProbeOutcome::Ready(session) => {
                self.session = Some(session);
                self.policy
                    .discovered_ready(now_unix_ms(), BridgeOwnership::Managed);
                self.authorization = None;
                self.publish();
            }
            ProbeOutcome::Pending(progress) => {
                self.session = None;
                self.policy
                    .discovered_pending(now_unix_ms(), BridgeOwnership::Managed, progress);
                self.publish();
            }
            ProbeOutcome::Absent => {
                if matches!(
                    self.policy.snapshot().phase,
                    BridgePhase::Ready | BridgePhase::Authorized
                ) {
                    self.session = None;
                    self.policy.discovered_pending(
                        now_unix_ms(),
                        BridgeOwnership::Managed,
                        ConnectionProgress::Reconnecting,
                    );
                    self.publish();
                }
            }
            ProbeOutcome::Blocked(code) => {
                self.session = None;
                self.policy.halt(now_unix_ms(), code);
                self.publish();
            }
        }
    }

    async fn handle_external_probe(&mut self) {
        match self.probe().await {
            ProbeOutcome::Authorized => {
                self.session = None;
                self.policy
                    .discovered_authorized(now_unix_ms(), BridgeOwnership::External);
                self.publish();
            }
            ProbeOutcome::Ready(session) => {
                self.session = Some(session);
                self.policy
                    .discovered_ready(now_unix_ms(), BridgeOwnership::External);
                self.publish();
            }
            ProbeOutcome::Pending(progress) => {
                self.session = None;
                self.policy
                    .discovered_pending(now_unix_ms(), BridgeOwnership::External, progress);
                self.publish();
            }
            ProbeOutcome::Absent => self.start_managed(),
            ProbeOutcome::Blocked(code) => {
                self.session = None;
                self.policy.halt(now_unix_ms(), code);
                self.publish();
            }
        }
    }

    fn handle_process_event(&mut self, event: CommandEvent) {
        match event {
            CommandEvent::Stdout(bytes) => self.handle_stdout(&bytes),
            CommandEvent::Stderr(bytes) => {
                if let Some(code) = stable_bridge_error_code(&bytes) {
                    self.policy.set_diagnostic(now_unix_ms(), code);
                    self.publish();
                }
            }
            CommandEvent::Error(_) => {
                self.policy.set_diagnostic(
                    now_unix_ms(),
                    "desktop.bridge.output_channel_failed".to_owned(),
                );
                self.publish();
            }
            CommandEvent::Terminated(payload) => self.handle_exit(payload.code),
            _ => {}
        }
    }

    fn handle_stdout(&mut self, bytes: &[u8]) {
        let Some(event) = supervisor_event(bytes) else {
            return;
        };
        match event {
            BridgeSupervisorEvent::AuthorizationRequired {
                channel,
                verification_uri,
                user_code,
                expires_in_seconds,
            } if channel == SUPERVISOR_CHANNEL => {
                let Ok(prompt) = AuthorizationPrompt::new(
                    self.generation,
                    &verification_uri,
                    &user_code,
                    expires_in_seconds,
                ) else {
                    self.policy.halt(
                        now_unix_ms(),
                        "desktop.authorization.prompt_invalid".to_owned(),
                    );
                    self.publish();
                    return;
                };
                self.authorization = Some(prompt);
                self.session = None;
                self.policy.authorization_required(now_unix_ms());
                self.publish();
            }
            BridgeSupervisorEvent::DeviceAuthorized { channel }
            | BridgeSupervisorEvent::Ready { channel }
                if channel == SUPERVISOR_CHANNEL =>
            {
                self.authorization = None;
                self.session = None;
                self.policy.discovered_pending(
                    now_unix_ms(),
                    BridgeOwnership::Managed,
                    ConnectionProgress::Starting,
                );
                self.publish();
            }
            BridgeSupervisorEvent::TransientFailure { channel, code }
                if channel == SUPERVISOR_CHANNEL && is_stable_bridge_code(&code) =>
            {
                self.policy.set_diagnostic(now_unix_ms(), code);
                self.publish();
            }
            BridgeSupervisorEvent::ServerUnreachable {
                channel,
                code,
                retry_after_ms,
            } if channel == SUPERVISOR_CHANNEL && is_stable_bridge_code(&code) => {
                self.authorization = None;
                self.session = None;
                self.policy.server_unreachable(
                    now_unix_ms(),
                    code,
                    Duration::from_millis(retry_after_ms),
                );
                self.publish();
            }
            _ => {}
        }
    }

    fn handle_exit(&mut self, exit_code: Option<i32>) {
        if let Ok(mut child) = self.child.lock() {
            *child = None;
        }
        self.managed_child_active = false;
        self.authorization = None;
        self.session = None;
        match self.policy.child_exited(
            now_unix_ms(),
            exit_code,
            self.shutting_down.load(Ordering::SeqCst),
        ) {
            ExitDecision::RetryAfter(delay) => {
                let sender = self.input.clone();
                let generation = self.generation;
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(delay).await;
                    let _ = sender.send(ActorInput::AutomaticRetry { generation }).await;
                });
            }
            ExitDecision::Halt => {
                let (title, body) = crate::native_language::stopped_message(
                    crate::native_language::language(&self.app),
                );
                let _ = self
                    .app
                    .notification()
                    .builder()
                    .title(title)
                    .body(body)
                    .show();
            }
            ExitDecision::Stop => {}
        }
        self.publish();
    }

    fn kill_managed_child(&mut self) {
        if let Ok(mut child) = self.child.lock()
            && let Some(child) = child.take()
        {
            let _ = child.kill();
        }
        self.managed_child_active = false;
    }

    async fn probe(&self) -> ProbeOutcome {
        let client = LocalBridgeClient::desktop_shell_with_secure_storage_service(
            self.config.runtime_root(),
            self.config.secure_storage_service(),
        );
        let result = client.invoke(IpcMethod::BridgeStatus).await;
        match result {
            Ok(IpcResponse::BridgeStatus {
                state: IpcBridgeState::Ready,
                ..
            }) => match client.invoke(IpcMethod::GetSelf).await {
                Ok(IpcResponse::SelfSummary { summary })
                    if summary.connection_state == IpcBridgeState::Ready =>
                {
                    ProbeOutcome::Ready(summary.into())
                }
                Ok(IpcResponse::SelfSummary { .. }) => ProbeOutcome::Authorized,
                Ok(_) => ProbeOutcome::Blocked("desktop.bridge.self_response_invalid".to_owned()),
                Err(failure) if failure.code() == "bridge.agent_runtime_unavailable" => {
                    ProbeOutcome::Authorized
                }
                Err(failure)
                    if failure.kind() == LocalBridgeClientFailureKind::Timeout
                        || failure.category()
                            == agent_room_bridge_ipc::IpcErrorCategory::DependencyUnavailable =>
                {
                    // The device is healthy; failure of the default character
                    // must not prevent other host tasks from joining.
                    ProbeOutcome::Authorized
                }
                Err(failure) => ProbeOutcome::Blocked(failure.code().to_owned()),
            },
            Ok(IpcResponse::BridgeStatus { state, .. }) => probe_connection_state(state),
            Ok(_) => ProbeOutcome::Blocked("desktop.bridge.probe_response_invalid".to_owned()),
            Err(failure)
                if matches!(
                    failure.kind(),
                    LocalBridgeClientFailureKind::CredentialsMissing
                        | LocalBridgeClientFailureKind::BridgeUnavailable
                        | LocalBridgeClientFailureKind::Timeout
                ) =>
            {
                ProbeOutcome::Absent
            }
            Err(failure) => ProbeOutcome::Blocked(failure.code().to_owned()),
        }
    }

    fn publish(&self) {
        let authorization = self.authorization.as_ref().map(AuthorizationPrompt::view);
        let next = SupervisorState {
            view: BridgeRuntimeView::new(
                self.policy.snapshot().clone(),
                authorization,
                self.session.clone(),
            ),
            authorization_url: self
                .authorization
                .as_ref()
                .map(|prompt| prompt.verification_uri.clone()),
        };
        self.state.send_replace(next.clone());
        let _ = self.app.emit(RUNTIME_CHANGED_EVENT, next.view);
    }
}

#[derive(Debug)]
enum ActorInput {
    ExplicitRetry,
    ReauthorizeDevice,
    Reconfigure {
        config: Box<DesktopBridgeConfig>,
    },
    AutomaticRetry {
        generation: u64,
    },
    ProcessEvent {
        generation: u64,
        event: CommandEvent,
    },
    Resume,
    Shutdown,
}

enum ProbeOutcome {
    Authorized,
    Ready(BridgeAgentSessionView),
    Pending(ConnectionProgress),
    Absent,
    Blocked(String),
}

fn probe_connection_state(state: IpcBridgeState) -> ProbeOutcome {
    match state {
        IpcBridgeState::Starting => ProbeOutcome::Pending(ConnectionProgress::Starting),
        IpcBridgeState::Reconnecting | IpcBridgeState::ShuttingDown => {
            ProbeOutcome::Pending(ConnectionProgress::Reconnecting)
        }
        IpcBridgeState::Offline => ProbeOutcome::Blocked("desktop.bridge.offline".to_owned()),
        IpcBridgeState::Ready => ProbeOutcome::Authorized,
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum BridgeSupervisorEvent {
    AuthorizationRequired {
        channel: String,
        #[serde(rename = "verificationUri")]
        verification_uri: String,
        #[serde(rename = "userCode")]
        user_code: String,
        #[serde(rename = "expiresInSeconds")]
        expires_in_seconds: u64,
    },
    DeviceAuthorized {
        channel: String,
    },
    Ready {
        channel: String,
    },
    TransientFailure {
        channel: String,
        code: String,
    },
    ServerUnreachable {
        channel: String,
        code: String,
        #[serde(rename = "retryAfterMs")]
        retry_after_ms: u64,
    },
}

fn supervisor_event(bytes: &[u8]) -> Option<BridgeSupervisorEvent> {
    let line = std::str::from_utf8(bytes).ok()?;
    serde_json::from_str(line.trim()).ok()
}

#[derive(Debug, Clone)]
struct AuthorizationPrompt {
    prompt_id: String,
    verification_uri: Url,
    verification_host: String,
    user_code: String,
    expires_at_unix_ms: i64,
}

impl AuthorizationPrompt {
    fn new(
        generation: u64,
        verification_uri: &str,
        user_code: &str,
        expires_in_seconds: u64,
    ) -> Result<Self, SupervisorFailure> {
        let verification_uri = Url::parse(verification_uri)
            .map_err(|_| SupervisorFailure::new("desktop.authorization.prompt_invalid", false))?;
        let host = verification_uri
            .host_str()
            .ok_or_else(|| SupervisorFailure::new("desktop.authorization.prompt_invalid", false))?;
        let verification_host = host.to_owned();
        let loopback_http = verification_uri.scheme() == "http"
            && host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        if (verification_uri.scheme() != "https" && !loopback_http)
            || verification_uri.username() != ""
            || verification_uri.password().is_some()
            || user_code.is_empty()
            || user_code.len() > 64
            || user_code.chars().any(char::is_control)
            || !(1..=MAX_AUTHORIZATION_SECONDS).contains(&expires_in_seconds)
        {
            return Err(SupervisorFailure::new(
                "desktop.authorization.prompt_invalid",
                false,
            ));
        }
        let expires_delta =
            i64::try_from(expires_in_seconds.saturating_mul(1_000)).unwrap_or(i64::MAX);
        Ok(Self {
            prompt_id: format!("authorization-{generation}"),
            verification_uri,
            verification_host,
            user_code: user_code.to_owned(),
            expires_at_unix_ms: now_unix_ms().saturating_add(expires_delta),
        })
    }

    fn view(&self) -> AuthorizationPromptView {
        AuthorizationPromptView {
            prompt_id: self.prompt_id.clone(),
            verification_host: self.verification_host.clone(),
            user_code: self.user_code.clone(),
            expires_at_unix_ms: self.expires_at_unix_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SupervisorFailure {
    pub(crate) code: &'static str,
    pub(crate) retryable: bool,
}

impl SupervisorFailure {
    pub(crate) const fn new(code: &'static str, retryable: bool) -> Self {
        Self { code, retryable }
    }
}

fn stable_bridge_error_code(bytes: &[u8]) -> Option<String> {
    let line = std::str::from_utf8(bytes).ok()?.trim();
    let prefix = "Agent Room Bridge 启动失败 [";
    let suffix_start = line.strip_prefix(prefix)?;
    let end = suffix_start.find(']')?;
    let code = &suffix_start[..end];
    if !is_stable_bridge_code(code) {
        return None;
    }
    Some(code.to_owned())
}

fn is_stable_bridge_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 128
        && code.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_')
        })
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        AuthorizationPrompt, BridgeAgentSessionView, BridgeRuntimeView, BridgeSupervisorEvent,
        is_stable_bridge_code, stable_bridge_error_code, supervisor_event,
    };
    use crate::bridge_lifecycle::{BridgeOwnership, BridgeRestartPolicy};

    #[test]
    fn offline_and_reconnecting_are_not_reported_as_starting() {
        use super::{ConnectionProgress, ProbeOutcome, probe_connection_state};
        use agent_room_bridge_ipc::IpcBridgeState;
        assert!(matches!(
            probe_connection_state(IpcBridgeState::Reconnecting),
            ProbeOutcome::Pending(ConnectionProgress::Reconnecting)
        ));
        assert!(
            matches!(probe_connection_state(IpcBridgeState::Offline), ProbeOutcome::Blocked(code) if code == "desktop.bridge.offline")
        );
        assert!(matches!(
            probe_connection_state(IpcBridgeState::Ready),
            ProbeOutcome::Authorized
        ));
    }

    #[test]
    fn 授权提示只接受_https_或本机地址且不暴露完整地址() {
        let prompt = AuthorizationPrompt::new(
            7,
            "https://identity.example/device?user_code=ABCD",
            "ABCD",
            600,
        )
        .expect("HTTPS 授权地址有效");
        let view = prompt.view();

        assert_eq!(view.prompt_id, "authorization-7");
        assert_eq!(view.verification_host, "identity.example");
        assert!(AuthorizationPrompt::new(1, "http://evil.example", "A", 60).is_err());
    }

    #[test]
    fn 子进程错误只提取稳定代码而不保留路径或正文() {
        let code = stable_bridge_error_code(
            "Agent Room Bridge 启动失败 [bridge.config_missing]：C:\\Users\\secret".as_bytes(),
        );

        assert_eq!(code.as_deref(), Some("bridge.config_missing"));
        assert!(stable_bridge_error_code(b"random stderr C:\\Users\\secret").is_none());
    }

    #[test]
    fn 解析_bridge_连不上服务器时的监督事件() {
        let line = br#"{"event":"server_unreachable","channel":"agent_room_desktop","code":"bridge.identity_provider_unavailable","retryAfterMs":1500}
"#;

        let Some(BridgeSupervisorEvent::ServerUnreachable {
            channel,
            code,
            retry_after_ms,
        }) = supervisor_event(line)
        else {
            panic!("Bridge 输出的事件必须能被桌面端解析");
        };

        assert_eq!(channel, "agent_room_desktop");
        assert_eq!(code, "bridge.identity_provider_unavailable");
        assert_eq!(retry_after_ms, 1_500);
        assert!(supervisor_event("Agent Room Bridge 已就绪。".as_bytes()).is_none());
    }

    #[test]
    fn 监督事件诊断只接受有限字符集的稳定代码() {
        assert!(is_stable_bridge_code(
            "bridge.matrix_restore_dependency_unavailable"
        ));
        assert!(!is_stable_bridge_code(""));
        assert!(!is_stable_bridge_code("C:\\Users\\secret"));
        assert!(!is_stable_bridge_code("bridge.MatrixFailure"));
        assert!(!is_stable_bridge_code(&"a".repeat(129)));
    }

    #[test]
    fn 停机视图向界面提供重新授权这台电脑() {
        let mut policy = BridgeRestartPolicy::new(0);
        policy.starting(0);
        for now in 1..=4 {
            let _ = policy.child_exited(now, Some(1), false);
        }

        let halted = serde_json::to_value(BridgeRuntimeView::new(
            policy.snapshot().clone(),
            None,
            None,
        ))
        .expect("运行时视图可序列化");

        assert_eq!(halted["lifecycle"]["phase"], "halted");
        assert_eq!(halted["deviceReauthorizationAvailable"], true);

        policy.explicit_retry(5);
        let restarting = BridgeRuntimeView::new(policy.snapshot().clone(), None, None);
        assert!(!restarting.device_reauthorization_available);

        let mut external = BridgeRestartPolicy::new(0);
        external.discovered_ready(1, BridgeOwnership::External);
        external.halt(2, "desktop.bridge.offline");
        assert!(
            !BridgeRuntimeView::new(external.snapshot().clone(), None, None)
                .device_reauthorization_available
        );
    }

    #[test]
    fn 会话视图保持既有桌面_json_契约() {
        let view = BridgeAgentSessionView {
            agent: "01990d9e-8400-7000-8000-000000000001".to_owned(),
            instance: "01990d9e-8400-7000-8000-000000000002".to_owned(),
            matrix_room: "!room:matrix.agent-room.test".to_owned(),
        };

        assert_eq!(
            serde_json::to_value(view).expect("会话视图可序列化"),
            json!({
                "agentId": "01990d9e-8400-7000-8000-000000000001",
                "instanceId": "01990d9e-8400-7000-8000-000000000002",
                "matrixRoomId": "!room:matrix.agent-room.test"
            })
        );
    }
}
