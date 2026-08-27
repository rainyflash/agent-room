use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use agent_room_bridge_ipc::{IpcMethod, IpcResponse};
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
        BridgeLifecycleSnapshot, BridgeOwnership, BridgePhase, BridgeRestartPolicy, ExitDecision,
        ResumeDecision, ResumeProbeState, decide_resume,
    },
    desktop_config::DesktopBridgeConfig,
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
            view: BridgeRuntimeView {
                lifecycle: policy.snapshot().clone(),
                authorization: None,
            },
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
            .try_send(ActorInput::Reconfigure { config })
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
            ProbeOutcome::Ready => {
                self.policy
                    .discovered_ready(now_unix_ms(), BridgeOwnership::External);
                self.publish();
            }
            ProbeOutcome::Absent => self.start_managed(),
            ProbeOutcome::Blocked(code) => {
                self.policy.halt(now_unix_ms(), code);
                self.publish();
            }
        }

        while let Some(input) = self.receiver.recv().await {
            match input {
                ActorInput::ExplicitRetry => self.handle_explicit_retry(),
                ActorInput::AutomaticRetry { generation } => {
                    if generation == self.generation
                        && self.policy.snapshot().phase == BridgePhase::RetryScheduled
                        && !self.managed_child_active
                    {
                        self.start_managed();
                    }
                }
                ActorInput::ProbeManaged { generation } => {
                    if generation == self.generation && self.managed_child_active {
                        self.handle_managed_probe().await;
                    }
                }
                ActorInput::ProcessEvent { generation, event } => {
                    if generation == self.generation {
                        self.handle_process_event(event);
                    }
                }
                ActorInput::Reconfigure { config } => self.handle_reconfigure(config),
                ActorInput::Resume => self.handle_resume().await,
                ActorInput::Shutdown => {
                    self.shutting_down.store(true, Ordering::SeqCst);
                    self.kill_managed_child();
                    self.policy.stop(now_unix_ms());
                    self.authorization = None;
                    self.publish();
                    break;
                }
            }
        }
    }

    fn start_managed(&mut self) {
        if self.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        self.generation = self.generation.saturating_add(1);
        self.policy.starting(now_unix_ms());
        self.authorization = None;
        self.publish();
        let generation = self.generation;
        let spawned = self
            .app
            .shell()
            .sidecar("agent-room-bridge")
            .map(|command| command.envs(self.config.environment()))
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
        schedule_probe(self.input.clone(), generation);
    }

    fn handle_explicit_retry(&mut self) {
        if self.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        if matches!(
            self.policy.snapshot().phase,
            BridgePhase::Ready | BridgePhase::Starting | BridgePhase::AuthorizationRequired
        ) {
            return;
        }
        self.kill_managed_child();
        self.policy.explicit_retry(now_unix_ms());
        self.start_managed();
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
        self.policy.explicit_retry(now_unix_ms());
        self.start_managed();
    }

    async fn handle_resume(&mut self) {
        let probe = self.probe().await;
        let probe_state = match probe {
            ProbeOutcome::Ready => ResumeProbeState::Ready,
            ProbeOutcome::Absent => ResumeProbeState::Absent,
            ProbeOutcome::Blocked(_) => ResumeProbeState::Blocked,
        };
        match decide_resume(
            probe_state,
            self.managed_child_active,
            self.policy.snapshot().phase,
        ) {
            ResumeDecision::Ready(ownership) => {
                self.policy.discovered_ready(now_unix_ms(), ownership);
                self.publish();
            }
            ResumeDecision::StartManaged => self.start_managed(),
            ResumeDecision::KeepProbing => {
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
            ProbeOutcome::Ready => {
                self.policy
                    .discovered_ready(now_unix_ms(), BridgeOwnership::Managed);
                self.authorization = None;
                self.publish();
            }
            ProbeOutcome::Absent => {
                if !matches!(
                    self.policy.snapshot().phase,
                    BridgePhase::AuthorizationRequired | BridgePhase::Ready
                ) {
                    schedule_probe(self.input.clone(), self.generation);
                }
            }
            ProbeOutcome::Blocked(code) => {
                self.policy.set_diagnostic(now_unix_ms(), code);
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
        let Ok(line) = std::str::from_utf8(bytes) else {
            return;
        };
        let Ok(event) = serde_json::from_str::<BridgeSupervisorEvent>(line.trim()) else {
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
                self.policy.authorization_required(now_unix_ms());
                self.publish();
            }
            BridgeSupervisorEvent::Ready { channel } if channel == SUPERVISOR_CHANNEL => {
                self.authorization = None;
                self.policy
                    .discovered_ready(now_unix_ms(), BridgeOwnership::Managed);
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
                let _ = self
                    .app
                    .notification()
                    .builder()
                    .title("Agent Room Bridge stopped")
                    .body("Automatic restart was stopped after repeated crashes. Open Agent Room for diagnostics.")
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
        let result = LocalBridgeClient::desktop_shell_with_secure_storage_service(
            self.config.runtime_root(),
            self.config.secure_storage_service(),
        )
        .invoke(IpcMethod::BridgeStatus)
        .await;
        match result {
            Ok(IpcResponse::BridgeStatus { .. }) => ProbeOutcome::Ready,
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
            view: BridgeRuntimeView {
                lifecycle: self.policy.snapshot().clone(),
                authorization,
            },
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
    Reconfigure {
        config: DesktopBridgeConfig,
    },
    AutomaticRetry {
        generation: u64,
    },
    ProbeManaged {
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
    Ready,
    Absent,
    Blocked(String),
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
    Ready {
        channel: String,
    },
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

fn schedule_probe(sender: mpsc::Sender<ActorInput>, generation: u64) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(PROBE_INTERVAL).await;
        let _ = sender.send(ActorInput::ProbeManaged { generation }).await;
    });
}

fn stable_bridge_error_code(bytes: &[u8]) -> Option<String> {
    let line = std::str::from_utf8(bytes).ok()?.trim();
    let prefix = "Agent Room Bridge 启动失败 [";
    let suffix_start = line.strip_prefix(prefix)?;
    let end = suffix_start.find(']')?;
    let code = &suffix_start[..end];
    if code.is_empty()
        || code.len() > 128
        || !code.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_')
        })
    {
        return None;
    }
    Some(code.to_owned())
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
    use super::{AuthorizationPrompt, stable_bridge_error_code};

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
}
