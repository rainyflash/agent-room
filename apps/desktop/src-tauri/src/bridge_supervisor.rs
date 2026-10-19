use std::{
    fs::{self, OpenOptions},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use agent_room_bridge_ipc::{IpcBridgeState, IpcMethod, IpcResponse, IpcSelfSummary};
use agent_room_bridge_local_adapter::{LocalBridgeClient, LocalBridgeClientFailureKind};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter as _, Manager as _};
use tauri_plugin_notification::NotificationExt as _;
use tauri_plugin_shell::{
    ShellExt as _,
    process::{CommandChild, CommandEvent},
};
use tokio::sync::{mpsc, oneshot, watch};
use url::Url;

use crate::{
    bridge_lifecycle::{
        AUTHORIZATION_LOST_DIAGNOSTIC, BridgeLifecycleSnapshot, BridgeOwnership, BridgePhase,
        BridgeRestartPolicy, ConnectionProgress, ExitDecision, ResumeDecision, ResumeProbeState,
        UnmanagedAction, UnmanagedBridge, decide_resume,
    },
    computer_grant::{
        ComputerGrant, ComputerGrantClient, ComputerGrantFailure, DEVICE_CODE_LINE, GrantStep,
        GrantWait,
    },
    desktop_config::{BridgeLaunch, DesktopBridgeConfig},
    human_session::HumanSessionRuntime,
    server_move::BRIDGE_LOCK,
};

const SUPERVISOR_CHANNEL: &str = "agent_room_desktop";
const RUNTIME_CHANGED_EVENT: &str = "desktop://runtime-changed";
const ACTOR_QUEUE_CAPACITY: usize = 32;
const PROBE_INTERVAL: Duration = Duration::from_secs(2);
/// 桌面端退出前写给托管 Bridge 的一行（Bridge `runtime.rs` 的 `SUPERVISOR_EXIT_REQUEST`）：
/// 请它发出各人物的离开状态、让进行中的同步收尾后自己退出，而不是被直接结束。
const BRIDGE_EXIT_REQUEST: &[u8] = b"exit\n";
/// 等托管 Bridge 自己退出的上限；到了还没退就直接结束它。平常一两秒就退完。
const ORDERLY_EXIT_TIMEOUT: Duration = Duration::from_secs(8);
/// 换账号时写给托管 Bridge 的一行（Bridge `supervisor_input.rs` 的 `RELEASE_REQUEST`）：把这台电脑
/// 移出原来的账户（`specs/sign-out/design.md` 第 4 步）。
const BRIDGE_RELEASE_REQUEST: &[u8] = b"release\n";
/// 等 Bridge 对 `release` 回话的上限：它要请控制面移出这台电脑，成了再有序退出。
const RELEASE_TIMEOUT: Duration = Duration::from_mins(1);
const MAX_AUTHORIZATION_SECONDS: u64 = 30 * 60;
/// Bridge 拿不到实例锁时的启动失败代码，见 Bridge 的 `BridgeRuntimeError::instance_lock`。
const INSTANCE_LOCK_HELD_CODE: &str = "bridge.already_running";
const OTHER_INSTANCE_CODE: &str = "desktop.bridge.other_instance_running";

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
            grant: GrantWait::default(),
            state: state_tx,
            generation: 0,
            managed_child_active: false,
            child_found_other_instance: false,
            child_lost_authorization: false,
            logged_lifecycle: None,
            release: ReleaseGate::Closed,
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

    /// 应用里登录好了（恢复了本机的登录或刚登录完）：托管的 Bridge 在等电脑授权的话，现在去要。
    pub(crate) fn signed_in(&self) {
        let _ = self.input.try_send(ActorInput::SignedIn);
    }

    /// 换账号：请托管的 Bridge 把这台电脑移出原来的账户、等它退出。从这时起不再拉起 Bridge，
    /// 直到 [`Self::resume_after_release`]：不然新起的 Bridge 会拿还存着的原账户登录要电脑授权。
    /// 没移出去时照旧运行、照旧会自动重启。
    pub(crate) async fn release_computer(&self) -> Result<ReleasedComputer, SupervisorFailure> {
        let (done, result) = oneshot::channel();
        self.input
            .send(ActorInput::ReleaseComputer { done })
            .await
            .map_err(|_| SupervisorFailure::new("desktop.bridge.command_queue_busy", true))?;
        result
            .await
            .unwrap_or_else(|_| Err(SupervisorFailure::new("desktop.release.interrupted", true)))
    }

    /// 换账号时原账户的登录也清掉了：换上不带原账户默认 Agent 的配置，重新拉起 Bridge，
    /// 等新账户的电脑授权。不在换账号时什么都不做。
    pub(crate) fn resume_after_release(&self, config: DesktopBridgeConfig) {
        let _ = self.input.try_send(ActorInput::ResumeAfterRelease {
            config: Box::new(config),
        });
    }

    /// 桌面端正常退出时用：请托管的 Bridge 有序退出并等它，超时再直接结束。
    pub(crate) async fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let (done, stopped) = oneshot::channel();
        if self
            .input
            .send(ActorInput::Shutdown { done: Some(done) })
            .await
            .is_ok()
        {
            // 监督者自己也只等 ORDERLY_EXIT_TIMEOUT；多留一点余量给它收尾。
            let _ =
                tokio::time::timeout(ORDERLY_EXIT_TIMEOUT + Duration::from_secs(2), stopped).await;
        }
        self.shutdown_now();
    }

    /// 更新安装没成功、桌面端没有退出时，重新拉起刚才为安装而停下的托管 Bridge。
    pub(crate) fn recover_after_failed_update(&self) {
        self.shutting_down.store(false, Ordering::SeqCst);
        let _ = self.input.try_send(ActorInput::ExplicitRetry);
    }

    /// 来不及等的时候用：立刻结束托管的 Bridge。
    pub(crate) fn shutdown_now(&self) {
        self.shutting_down.store(true, Ordering::SeqCst);
        let _ = self.input.try_send(ActorInput::Shutdown { done: None });
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
    /// 当前子进程报告另一个 Bridge 进程占着实例锁，它随后的退出不算崩溃。
    child_found_other_instance: bool,
    /// 当前子进程报告这台电脑的授权失效了，它随后的退出不自动重启。
    child_lost_authorization: bool,
    /// 上一次写进日志的生命周期状态；状态没变的发布不再重复记。
    logged_lifecycle: Option<(BridgePhase, Option<String>, Option<String>)>,
    /// 这一代子进程等电脑授权的进度（`specs/desktop-sign-in/design.md`）。
    grant: GrantWait,
    /// 换账号用：这一代子进程读不读 `release`，或者正在换账号、不拉起 Bridge。
    release: ReleaseGate,
}

impl BridgeSupervisorActor {
    async fn run(mut self) {
        self.handle_unmanaged_probe().await;

        let mut probes = tokio::time::interval(PROBE_INTERVAL);
        probes.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let input = tokio::select! {
                input = self.receiver.recv() => match input { Some(input) => input, None => break },
                _ = probes.tick() => {
                    if self.policy.probes_periodically(self.managed_child_active) {
                        if self.managed_child_active { self.handle_managed_probe().await; }
                        else { self.handle_unmanaged_probe().await; }
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
                ActorInput::SignedIn => {
                    self.grant.signed_in();
                    self.request_grant();
                }
                ActorInput::GrantResolved { generation, result } => {
                    if generation == self.generation {
                        let step = self.grant.resolved(result);
                        self.follow_grant_step(step);
                    }
                }
                ActorInput::GrantRetry { generation } => {
                    if generation == self.generation {
                        self.grant.retry_due();
                        self.request_grant();
                    }
                }
                ActorInput::ReleaseComputer { done } => {
                    let result = self.handle_release_computer().await;
                    let _ = done.send(result);
                }
                ActorInput::ResumeAfterRelease { config } => {
                    self.handle_resume_after_release(*config);
                }
                ActorInput::Shutdown { done } => {
                    self.shutting_down.store(true, Ordering::SeqCst);
                    self.stop_managed_child_orderly().await;
                    self.policy.stop(now_unix_ms());
                    self.authorization = None;
                    self.session = None;
                    self.publish();
                    if let Some(done) = done {
                        let _ = done.send(());
                    }
                }
            }
        }
    }

    fn start_managed(&mut self) {
        self.start_managed_with(BridgeLaunch::Normal);
    }

    fn start_managed_with(&mut self, launch: BridgeLaunch) {
        if self.shutting_down.load(Ordering::SeqCst) || self.releasing() {
            return;
        }
        self.generation = self.generation.saturating_add(1);
        self.child_found_other_instance = false;
        self.child_lost_authorization = false;
        self.release = ReleaseGate::Closed;
        self.grant.reset();
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
            tracing::error!("Bridge 子进程启动失败：安装包里的 Bridge 不可执行或被拒绝");
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
        if self.shutting_down.load(Ordering::SeqCst) || self.releasing() {
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
            self.handle_unmanaged_probe().await;
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
            || self.releasing()
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
        if self.shutting_down.load(Ordering::SeqCst) || self.releasing() {
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

    /// 换账号：请 Bridge 移出这台电脑，等它回话、退出，然后停着不再拉起。
    async fn handle_release_computer(&mut self) -> Result<ReleasedComputer, SupervisorFailure> {
        if self.shutting_down.load(Ordering::SeqCst) {
            return Err(SupervisorFailure::new(
                "desktop.release.shutting_down",
                false,
            ));
        }
        if self.policy.snapshot().ownership == Some(BridgeOwnership::External) {
            return Err(SupervisorFailure::new(
                "desktop.release.external_bridge",
                false,
            ));
        }
        if !self.managed_child_active {
            // 没有在跑的托管 Bridge（停机了、在等重试）：服务器上的这台电脑这次移不了，本机照样收走。
            tracing::warn!("换账号时托管的 Bridge 不在运行，本机照样收走原账户的东西");
            self.stop_after_release();
            return Ok(ReleasedComputer::LocalOnly);
        }
        if self.release != ReleaseGate::Open {
            if self.policy.snapshot().phase != BridgePhase::AuthorizationRequired {
                // 还在启动：Bridge 报就绪之前写的行会被丢掉。
                return Err(SupervisorFailure::new(
                    "desktop.release.bridge_starting",
                    true,
                ));
            }
            // 在等设备码：服务器上还没有这台电脑，结束它就好。
            self.kill_managed_child();
            self.stop_after_release();
            return Ok(ReleasedComputer::AlreadyReleased);
        }
        if !self.write_to_bridge(BRIDGE_RELEASE_REQUEST) {
            return Err(SupervisorFailure::new(
                "desktop.release.bridge_unavailable",
                true,
            ));
        }
        tracing::info!("请托管的 Bridge 把这台电脑移出原来的账户");
        // 先挡住重启：Bridge 移出去以后马上退出，退出事件不能再把它拉起来。
        self.release = ReleaseGate::Releasing;
        let generation = self.generation;
        match wait_for_release(&mut self.receiver, generation, RELEASE_TIMEOUT).await {
            ReleaseWait::Released { already_released } => {
                if !wait_for_child_exit(&mut self.receiver, generation, ORDERLY_EXIT_TIMEOUT).await
                {
                    tracing::warn!("移出以后 Bridge 没有按时退出，直接结束它");
                }
                self.kill_managed_child();
                self.stop_after_release();
                Ok(if already_released {
                    ReleasedComputer::AlreadyReleased
                } else {
                    ReleasedComputer::Released
                })
            }
            ReleaseWait::Failed(failure) => {
                // 没移出去，Bridge 照常运行，可以再写一次。
                self.release = ReleaseGate::Open;
                tracing::warn!(
                    error_code = failure.code,
                    "Bridge 没能把这台电脑移出原来的账户"
                );
                Err(failure)
            }
            ReleaseWait::Exited(exit_code) => {
                self.release = ReleaseGate::Closed;
                self.handle_exit(exit_code);
                Err(SupervisorFailure::new(
                    "desktop.release.bridge_exited",
                    true,
                ))
            }
            ReleaseWait::TimedOut => {
                // Bridge 自己的请求 10 秒就超时，一分钟没回话多半是卡住了：结束它、照常重启。
                tracing::warn!("托管的 Bridge 一分钟没回 release，结束它再重启");
                self.release = ReleaseGate::Closed;
                self.generation = self.generation.saturating_add(1);
                self.kill_managed_child();
                self.policy.explicit_retry(now_unix_ms());
                self.start_managed();
                Err(SupervisorFailure::new("desktop.release.timeout", true))
            }
        }
    }

    /// 这台电脑已经移出原来的账户：停在“已停止”，让这一代子进程迟到的事件作废。
    fn stop_after_release(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.grant.reset();
        self.authorization = None;
        self.session = None;
        self.release = ReleaseGate::Releasing;
        self.policy.stop(now_unix_ms());
        self.publish();
    }

    fn handle_resume_after_release(&mut self, config: DesktopBridgeConfig) {
        if !self.releasing() {
            return;
        }
        tracing::info!("原账户的登录已清掉，重新拉起 Bridge 等新账户的电脑授权");
        self.release = ReleaseGate::Closed;
        self.config = config;
        self.policy.explicit_retry(now_unix_ms());
        self.start_managed();
    }

    fn releasing(&self) -> bool {
        self.release == ReleaseGate::Releasing
    }

    async fn handle_resume(&mut self) {
        if self.releasing() {
            return;
        }
        let probe = match self.probe().await {
            // 托管子进程自己在退出或连接中途断开时沿用原来的处理，它的退出事件会接着决定是否重启。
            ProbeOutcome::Departing if self.managed_child_active => {
                ProbeOutcome::Pending(ConnectionProgress::Reconnecting)
            }
            ProbeOutcome::Interrupted(code) if self.managed_child_active => {
                ProbeOutcome::Blocked(code)
            }
            // 正在退出的 Bridge 不接管：按没有 Bridge 处理，启动前再看它是否还占着实例锁。
            ProbeOutcome::Departing | ProbeOutcome::Interrupted(_) => ProbeOutcome::Absent,
            probe => probe,
        };
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
            ProbeOutcome::Pending(_) | ProbeOutcome::Departing | ProbeOutcome::Interrupted(_) => {
                unreachable!("等待、退出与中断已提前处理")
            }
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
            ResumeDecision::StartManaged => self.start_managed_unless_locked(),
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
            // 自己的子进程正在退出：它的退出事件会接着决定是否重启。
            ProbeOutcome::Departing => {
                self.session = None;
                self.policy.discovered_pending(
                    now_unix_ms(),
                    BridgeOwnership::Managed,
                    ConnectionProgress::Reconnecting,
                );
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
            ProbeOutcome::Interrupted(code) | ProbeOutcome::Blocked(code) => {
                self.session = None;
                self.policy.halt(now_unix_ms(), code);
                self.publish();
            }
        }
    }

    /// 没有托管子进程时探测本机 Bridge：启动时、观察外部 Bridge 时，以及等另一个 Bridge 进程走完时。
    async fn handle_unmanaged_probe(&mut self) {
        let bridge = match self.probe().await {
            ProbeOutcome::Ready(session) => {
                self.session = Some(session);
                UnmanagedBridge::Ready
            }
            ProbeOutcome::Authorized => UnmanagedBridge::Authorized,
            ProbeOutcome::Pending(progress) => UnmanagedBridge::Pending(progress),
            ProbeOutcome::Departing => UnmanagedBridge::Departing,
            // 连上后会话中途断开，多半是 Bridge 正在退出；和没有应答一样看实例锁。
            ProbeOutcome::Absent | ProbeOutcome::Interrupted(_) => self.unanswered_bridge(),
            ProbeOutcome::Blocked(code) => UnmanagedBridge::Blocked(code),
        };
        if bridge != UnmanagedBridge::Ready {
            self.session = None;
        }
        self.observe_unmanaged(bridge);
    }

    /// 本机没有 Bridge 应答：仍有 Bridge 进程占着实例锁就等它走完，否则启动托管 Bridge。
    fn start_managed_unless_locked(&mut self) {
        let bridge = self.unanswered_bridge();
        self.observe_unmanaged(bridge);
    }

    fn unanswered_bridge(&self) -> UnmanagedBridge {
        let data_root = self.config.data_root();
        if instance_lock_held(&data_root) {
            if !self
                .policy
                .snapshot()
                .diagnostic_code
                .as_deref()
                .eq(&Some(OTHER_INSTANCE_CODE))
            {
                tracing::info!(
                    holder_pid = instance_lock_holder(&data_root),
                    "另一个 Bridge 进程占着实例锁，等它退出"
                );
            }
            UnmanagedBridge::Locked
        } else {
            UnmanagedBridge::Absent
        }
    }

    fn observe_unmanaged(&mut self, bridge: UnmanagedBridge) {
        match self.policy.observe_unmanaged(now_unix_ms(), bridge) {
            UnmanagedAction::StartManaged => self.start_managed(),
            UnmanagedAction::NotifyStopped => {
                self.notify_stopped();
                self.publish();
            }
            UnmanagedAction::None => self.publish(),
        }
    }

    fn handle_process_event(&mut self, event: CommandEvent) {
        match event {
            CommandEvent::Stdout(bytes) => self.handle_stdout(&bytes),
            CommandEvent::Stderr(bytes) => {
                if let Some(code) = stable_bridge_error_code(&bytes) {
                    if code == INSTANCE_LOCK_HELD_CODE {
                        self.child_found_other_instance = true;
                    }
                    if code == AUTHORIZATION_LOST_DIAGNOSTIC {
                        self.child_lost_authorization = true;
                    }
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
        let event_kind = if matches!(event, BridgeSupervisorEvent::Ready { .. }) {
            SupervisorEventKind::Ready
        } else {
            SupervisorEventKind::Other
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
                self.release = ReleaseGate::Closed;
                self.policy.authorization_required(now_unix_ms());
                self.publish();
            }
            BridgeSupervisorEvent::DeviceGrantRequired { channel }
                if channel == SUPERVISOR_CHANNEL =>
            {
                // 界面照旧是“正在连接”，不出授权横幅：先拿账户登录换电脑授权。
                tracing::info!("托管的 Bridge 在等电脑授权");
                self.release = ReleaseGate::Open;
                self.grant.bridge_asked();
                self.request_grant();
            }
            BridgeSupervisorEvent::DeviceGrantFailed { channel, code }
                if channel == SUPERVISOR_CHANNEL && is_stable_bridge_code(&code) =>
            {
                tracing::warn!(error_code = code, "Bridge 拿电脑授权注册没成");
                let step = self.grant.bridge_rejected(&code);
                self.follow_grant_step(step);
            }
            BridgeSupervisorEvent::DeviceAuthorized { channel }
            | BridgeSupervisorEvent::Ready { channel }
                if channel == SUPERVISOR_CHANNEL =>
            {
                // 授权完到就绪之间 Bridge 不读标准输入，就绪以后才读 `release`。
                self.release = match event_kind {
                    SupervisorEventKind::Ready => ReleaseGate::Open,
                    SupervisorEventKind::Other => ReleaseGate::Closed,
                };
                self.grant.bridge_authorized();
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

    /// Bridge 在等电脑授权、手上又没有没回来的请求时，拿应用里的登录去要一份。
    /// 还没登录就等着：登录好了 `signed_in` 会再叫一次。
    fn request_grant(&mut self) {
        if !self.grant.should_request() {
            return;
        }
        let secret = self
            .app
            .try_state::<HumanSessionRuntime>()
            .and_then(|sessions| sessions.session_secret().ok().flatten());
        let Some(secret) = secret else {
            tracing::info!("应用里还没登录，登录以后再连上这台电脑");
            return;
        };
        let Some(client) = ComputerGrantClient::new(self.config.control_plane_url()) else {
            self.follow_grant_step(GrantStep::DeviceCode);
            return;
        };
        let origin = self
            .app
            .get_webview_window("main")
            .and_then(|window| window.url().ok())
            .as_ref()
            .and_then(crate::control_plane_proxy::webview_origin);
        self.grant.requesting();
        let sender = self.input.clone();
        let generation = self.generation;
        let app = self.app.clone();
        tauri::async_runtime::spawn(async move {
            let result = match origin {
                Some(origin) => client.request(&origin, &secret).await,
                // 窗口还没起来：等一会儿再要。
                None => Err(ComputerGrantFailure::Transient),
            };
            if result == Err(ComputerGrantFailure::SessionRejected)
                && let Some(sessions) = app.try_state::<HumanSessionRuntime>()
                && let Err(failure) = sessions.forget_rejected(&secret)
            {
                tracing::warn!(
                    error_code = failure.code(),
                    "控制面说登录无效，本机这份没删掉"
                );
            }
            let _ = sender
                .send(ActorInput::GrantResolved { generation, result })
                .await;
        });
    }

    fn follow_grant_step(&mut self, step: GrantStep) {
        match step {
            GrantStep::Wait => {}
            GrantStep::Deliver(grant) => {
                if self.write_to_bridge(grant.bridge_line().as_bytes()) {
                    tracing::info!("电脑授权已交给 Bridge");
                }
            }
            GrantStep::RetryAfter(delay) => {
                tracing::info!(
                    delay_ms = delay.as_millis(),
                    "暂时要不到电脑授权，过一会儿再要"
                );
                let sender = self.input.clone();
                let generation = self.generation;
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(delay).await;
                    let _ = sender.send(ActorInput::GrantRetry { generation }).await;
                });
            }
            GrantStep::ForgetSession => {
                tracing::info!("控制面说应用里的登录无效，重新登录以后再连上这台电脑");
            }
            GrantStep::DeviceCode => {
                tracing::warn!("要不到电脑授权，请 Bridge 改走设备码");
                self.write_to_bridge(DEVICE_CODE_LINE.as_bytes());
            }
        }
    }

    /// 往托管 Bridge 的标准输入写一行；它已经不在就算了，它的退出事件会接着处理。
    fn write_to_bridge(&self, line: &[u8]) -> bool {
        self.managed_child_active
            && self
                .child
                .lock()
                .ok()
                .and_then(|mut child| child.as_mut().map(|child| child.write(line).is_ok()))
                .unwrap_or(false)
    }

    fn handle_exit(&mut self, exit_code: Option<i32>) {
        tracing::warn!(?exit_code, "Bridge 子进程退出");
        self.grant.reset();
        if let Ok(mut child) = self.child.lock() {
            *child = None;
        }
        self.managed_child_active = false;
        self.authorization = None;
        self.session = None;
        let shutting_down = self.shutting_down.load(Ordering::SeqCst);
        if self.child_found_other_instance && !shutting_down {
            // 子进程没拿到实例锁就退出了：另一个 Bridge 进程还在，比如上一个桌面留下、正在收尾的那个。
            // 这不是崩溃，不占重启预算；等它走完再按普通方式启动。重新授权的请求随之作废：Bridge 先拿锁
            // 后清凭据，这个子进程什么也没动，之后若仍停机，停机视图会再提供重新授权。
            self.observe_unmanaged(UnmanagedBridge::Locked);
            return;
        }
        if self.child_lost_authorization && !shutting_down {
            tracing::warn!("这台电脑的授权失效了，停下等人重新授权");
            self.policy.authorization_lost(now_unix_ms(), exit_code);
            self.notify_stopped();
            self.publish();
            return;
        }
        match self
            .policy
            .child_exited(now_unix_ms(), exit_code, shutting_down)
        {
            ExitDecision::RetryAfter(delay) => {
                let sender = self.input.clone();
                let generation = self.generation;
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(delay).await;
                    let _ = sender.send(ActorInput::AutomaticRetry { generation }).await;
                });
            }
            ExitDecision::Halt => self.notify_stopped(),
            ExitDecision::Stop => {}
        }
        self.publish();
    }

    fn notify_stopped(&self) {
        let (title, body) =
            crate::native_language::stopped_message(crate::native_language::language(&self.app));
        let _ = self
            .app
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show();
    }

    /// 请托管的 Bridge 自己退出：写一行退出请求，等这个子进程的退出事件；
    /// 写不进去（比如它已经退了）或者等不到，就直接结束它。
    async fn stop_managed_child_orderly(&mut self) {
        let asked = self.managed_child_active
            && self
                .child
                .lock()
                .ok()
                .and_then(|mut child| {
                    child
                        .as_mut()
                        .map(|child| child.write(BRIDGE_EXIT_REQUEST).is_ok())
                })
                .unwrap_or(false);
        if asked {
            let generation = self.generation;
            let exited =
                wait_for_child_exit(&mut self.receiver, generation, ORDERLY_EXIT_TIMEOUT).await;
            if exited {
                tracing::info!("托管的 Bridge 已按请求有序退出");
                if let Ok(mut child) = self.child.lock() {
                    *child = None;
                }
                self.managed_child_active = false;
                return;
            }
            tracing::warn!("托管的 Bridge 没有按时退出，直接结束它");
        }
        self.kill_managed_child();
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
            Ok(IpcResponse::BridgeStatus {
                state,
                failure_code,
                ..
            }) => probe_connection_state(state, failure_code),
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
            Err(failure)
                if failure.kind() == LocalBridgeClientFailureKind::Protocol
                    && failure.retryable() =>
            {
                ProbeOutcome::Interrupted(failure.code().to_owned())
            }
            Err(failure) => ProbeOutcome::Blocked(failure.code().to_owned()),
        }
    }

    fn publish(&mut self) {
        self.log_lifecycle();
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
        crate::tray_hint::update_tooltip(&self.app, self.policy.snapshot().phase);
        self.state.send_replace(next.clone());
        let _ = self.app.emit(RUNTIME_CHANGED_EVENT, next.view);
    }

    fn log_lifecycle(&mut self) {
        let snapshot = self.policy.snapshot();
        let current = (
            snapshot.phase,
            snapshot.diagnostic_code.clone(),
            snapshot.last_failure_code.clone(),
        );
        if self.logged_lifecycle.as_ref() == Some(&current) {
            return;
        }
        tracing::info!(
            phase = ?snapshot.phase,
            ownership = ?snapshot.ownership,
            diagnostic_code = snapshot.diagnostic_code.as_deref(),
            last_failure_code = snapshot.last_failure_code.as_deref(),
            automatic_restart_count = snapshot.automatic_restart_count,
            last_exit_code = snapshot.last_exit_code,
            "Bridge 连接状态变化"
        );
        self.logged_lifecycle = Some(current);
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
    SignedIn,
    GrantResolved {
        generation: u64,
        result: Result<ComputerGrant, ComputerGrantFailure>,
    },
    GrantRetry {
        generation: u64,
    },
    /// 换账号：把这台电脑移出原来的账户，结果交给 `done`。
    ReleaseComputer {
        done: oneshot::Sender<Result<ReleasedComputer, SupervisorFailure>>,
    },
    /// 换账号时原账户的登录清掉了：换上新配置、重新拉起 Bridge。
    ResumeAfterRelease {
        config: Box<DesktopBridgeConfig>,
    },
    /// 停掉托管的 Bridge 并结束监督；`done` 在收尾之后通知等着的一方。
    Shutdown {
        done: Option<oneshot::Sender<()>>,
    },
}

/// 退出期间只关心这个子进程的退出事件，别的输入一律不再处理。
/// 等到了返回 `true`；超时或者输入通道关了返回 `false`。
async fn wait_for_child_exit(
    receiver: &mut mpsc::Receiver<ActorInput>,
    generation: u64,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match tokio::time::timeout_at(deadline, receiver.recv()).await {
            Ok(Some(ActorInput::ProcessEvent {
                generation: current,
                event: CommandEvent::Terminated(_),
            })) if current == generation => return true,
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => return false,
        }
    }
}

/// 换账号时 `release` 这一行能不能写、要不要挡住 Bridge 重启。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReleaseGate {
    /// 子进程还没就绪（启动中、授权完还没就绪、在等设备码），写了它也不读。
    Closed,
    /// 子进程报了就绪或者在等电脑授权，读 `release`。
    Open,
    /// 正在换账号：这台电脑已经（或者正在）移出原来的账户，原账户的登录还没清掉，不拉起 Bridge。
    Releasing,
}

/// 换账号时这台电脑怎么离开原来的账户的。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReleasedComputer {
    /// 这次移出去了。
    Released,
    /// 服务器上早就不算这个账户的电脑（在别处移除了，或者还没授权完）。
    AlreadyReleased,
    /// 托管的 Bridge 不在运行，服务器上的这台电脑这次没移；原账户在设置里还看得到它。
    LocalOnly,
}

#[derive(Debug)]
enum ReleaseWait {
    Released { already_released: bool },
    Failed(SupervisorFailure),
    Exited(Option<i32>),
    TimedOut,
}

/// 等托管 Bridge 对 `release` 的回话。和有序退出时一样，期间只看这一代子进程的输出和退出，
/// 别的输入不再处理。
async fn wait_for_release(
    receiver: &mut mpsc::Receiver<ActorInput>,
    generation: u64,
    timeout: Duration,
) -> ReleaseWait {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let event = match tokio::time::timeout_at(deadline, receiver.recv()).await {
            Ok(Some(ActorInput::ProcessEvent {
                generation: current,
                event,
            })) if current == generation => event,
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => return ReleaseWait::TimedOut,
        };
        match event {
            CommandEvent::Stdout(bytes) => match supervisor_event(&bytes) {
                Some(BridgeSupervisorEvent::ComputerReleased {
                    channel,
                    already_released,
                }) if channel == SUPERVISOR_CHANNEL => {
                    return ReleaseWait::Released { already_released };
                }
                Some(BridgeSupervisorEvent::ComputerReleaseFailed { channel, code })
                    if channel == SUPERVISOR_CHANNEL =>
                {
                    return ReleaseWait::Failed(release_failure(&code));
                }
                _ => {}
            },
            CommandEvent::Terminated(payload) => return ReleaseWait::Exited(payload.code),
            _ => {}
        }
    }
}

/// Bridge 报的错误码换成固定的几种；认不出的笼统算没成。
fn release_failure(code: &str) -> SupervisorFailure {
    match code {
        "bridge.release_unavailable" => SupervisorFailure::new("bridge.release_unavailable", true),
        "bridge.release_rejected" => SupervisorFailure::new("bridge.release_rejected", true),
        "bridge.release_unsupported" => SupervisorFailure::new("bridge.release_unsupported", false),
        _ => SupervisorFailure::new("desktop.release.failed", true),
    }
}

#[derive(Clone, Copy)]
enum SupervisorEventKind {
    Ready,
    Other,
}

enum ProbeOutcome {
    Authorized,
    Ready(BridgeAgentSessionView),
    Pending(ConnectionProgress),
    /// 应答的 Bridge 正在退出，比如监督它的桌面已经不在。
    Departing,
    Absent,
    /// 连上后会话中途断开。
    Interrupted(String),
    Blocked(String),
}

fn probe_connection_state(state: IpcBridgeState, failure_code: Option<String>) -> ProbeOutcome {
    match state {
        IpcBridgeState::Starting => ProbeOutcome::Pending(ConnectionProgress::Starting),
        IpcBridgeState::Reconnecting => ProbeOutcome::Pending(ConnectionProgress::Reconnecting),
        IpcBridgeState::ShuttingDown => ProbeOutcome::Departing,
        // Bridge 自己判定的原因（如设备授权失效、加密身份冲突）比笼统的「离线」有用得多。
        IpcBridgeState::Offline => ProbeOutcome::Blocked(
            failure_code.unwrap_or_else(|| "desktop.bridge.offline".to_owned()),
        ),
        IpcBridgeState::Ready => ProbeOutcome::Authorized,
    }
}

/// 占着实例锁的 Bridge 进程号，来自 Bridge 写在锁文件旁边的 `.pid` 文件；锁空闲时没有意义。
fn instance_lock_holder(data_root: &Path) -> Option<u32> {
    let mut name = std::ffi::OsString::from(BRIDGE_LOCK);
    name.push(".pid");
    fs::read_to_string(data_root.join(name))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// 是否还有 Bridge 进程占着本机实例锁。Bridge 在绑定 IPC 之前（启动中）和关闭 IPC 之后（退出收尾）
/// 都持有这把锁，这时再启动一个只会因锁被占立即退出。只做非阻塞尝试，拿到就立即释放；
/// 锁文件打不开时照常启动，由 Bridge 自己报告原因。
fn instance_lock_held(data_root: &Path) -> bool {
    let Ok(file) = OpenOptions::new()
        .read(true)
        .write(true)
        .open(data_root.join(BRIDGE_LOCK))
    else {
        return false;
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(fs::TryLockError::WouldBlock) => true,
        Err(fs::TryLockError::Error(_)) => false,
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
    /// 还没授权的 Bridge 在等桌面端交电脑授权。
    DeviceGrantRequired {
        channel: String,
    },
    /// Bridge 拿电脑授权注册没成，接着等下一行。
    DeviceGrantFailed {
        channel: String,
        code: String,
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
    /// 这台电脑已经移出原来的账户，Bridge 随后退出。
    ComputerReleased {
        channel: String,
        #[serde(rename = "alreadyReleased")]
        already_released: bool,
    },
    /// 没移出去，Bridge 照常运行。
    ComputerReleaseFailed {
        channel: String,
        code: String,
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

    fn terminated(generation: u64) -> super::ActorInput {
        super::ActorInput::ProcessEvent {
            generation,
            event: tauri_plugin_shell::process::CommandEvent::Terminated(
                tauri_plugin_shell::process::TerminatedPayload {
                    code: Some(0),
                    signal: None,
                },
            ),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn 退出时只等这一代子进程自己退出_别的输入和旧代的退出都不算() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
        sender
            .send(super::ActorInput::Resume)
            .await
            .expect("可以排队");
        sender
            .send(super::ActorInput::ProcessEvent {
                generation: 7,
                event: tauri_plugin_shell::process::CommandEvent::Stdout(b"line".to_vec()),
            })
            .await
            .expect("可以排队");
        sender.send(terminated(6)).await.expect("可以排队");
        sender.send(terminated(7)).await.expect("可以排队");

        assert!(super::wait_for_child_exit(&mut receiver, 7, super::ORDERLY_EXIT_TIMEOUT).await);
    }

    fn stdout(generation: u64, line: &str) -> super::ActorInput {
        super::ActorInput::ProcessEvent {
            generation,
            event: tauri_plugin_shell::process::CommandEvent::Stdout(line.as_bytes().to_vec()),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn 换账号时只认这一代子进程报的移出结果() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
        sender
            .send(super::ActorInput::Resume)
            .await
            .expect("可以排队");
        // 旧代的、别的事件、别的频道的都不算。
        sender
            .send(stdout(
                6,
                r#"{"event":"computer_released","channel":"agent_room_desktop","alreadyReleased":false}"#,
            ))
            .await
            .expect("可以排队");
        sender
            .send(stdout(
                7,
                r#"{"event":"ready","channel":"agent_room_desktop"}"#,
            ))
            .await
            .expect("可以排队");
        sender
            .send(stdout(
                7,
                r#"{"event":"computer_released","channel":"other","alreadyReleased":false}"#,
            ))
            .await
            .expect("可以排队");
        sender
            .send(stdout(
                7,
                r#"{"event":"computer_released","channel":"agent_room_desktop","alreadyReleased":true}"#,
            ))
            .await
            .expect("可以排队");

        assert!(matches!(
            super::wait_for_release(&mut receiver, 7, super::RELEASE_TIMEOUT).await,
            super::ReleaseWait::Released {
                already_released: true
            }
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn 换账号时_bridge_报没成_先退出了_或者等不到_都算没移出去() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
        sender
            .send(stdout(
                3,
                r#"{"event":"computer_release_failed","channel":"agent_room_desktop","code":"bridge.release_unavailable"}"#,
            ))
            .await
            .expect("可以排队");
        let super::ReleaseWait::Failed(failure) =
            super::wait_for_release(&mut receiver, 3, super::RELEASE_TIMEOUT).await
        else {
            panic!("应当是没成");
        };
        assert_eq!(failure.code, "bridge.release_unavailable");
        assert!(failure.retryable);

        sender.send(terminated(3)).await.expect("可以排队");
        assert!(matches!(
            super::wait_for_release(&mut receiver, 3, super::RELEASE_TIMEOUT).await,
            super::ReleaseWait::Exited(Some(0))
        ));

        let started = tokio::time::Instant::now();
        assert!(matches!(
            super::wait_for_release(&mut receiver, 3, super::RELEASE_TIMEOUT).await,
            super::ReleaseWait::TimedOut
        ));
        assert!(started.elapsed() >= super::RELEASE_TIMEOUT);
    }

    #[test]
    fn 认不出的移出错误码笼统算没成_控制面没有接口的不用再试() {
        let unsupported = super::release_failure("bridge.release_unsupported");
        assert_eq!(unsupported.code, "bridge.release_unsupported");
        assert!(!unsupported.retryable);
        let rejected = super::release_failure("bridge.release_rejected");
        assert_eq!(rejected.code, "bridge.release_rejected");
        let unknown = super::release_failure("bridge.something_new");
        assert_eq!(unknown.code, "desktop.release.failed");
        assert!(unknown.retryable);
    }

    #[tokio::test(start_paused = true)]
    async fn 子进程没按时退出_或者输入通道关了_都返回未退出() {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
        let started = tokio::time::Instant::now();
        assert!(!super::wait_for_child_exit(&mut receiver, 3, super::ORDERLY_EXIT_TIMEOUT).await);
        assert!(started.elapsed() >= super::ORDERLY_EXIT_TIMEOUT);

        drop(sender);
        assert!(!super::wait_for_child_exit(&mut receiver, 3, super::ORDERLY_EXIT_TIMEOUT).await);
    }

    #[test]
    fn offline_and_reconnecting_are_not_reported_as_starting() {
        use super::{ConnectionProgress, ProbeOutcome, probe_connection_state};
        use agent_room_bridge_ipc::IpcBridgeState;
        assert!(matches!(
            probe_connection_state(IpcBridgeState::Reconnecting, None),
            ProbeOutcome::Pending(ConnectionProgress::Reconnecting)
        ));
        assert!(
            matches!(probe_connection_state(IpcBridgeState::Offline, None), ProbeOutcome::Blocked(code) if code == "desktop.bridge.offline")
        );
        // Bridge 说得出原因时，停机诊断用它的原因码，而不是笼统的「离线」。
        assert!(matches!(
            probe_connection_state(IpcBridgeState::Offline, Some("bridge.secure_storage_corrupt".to_owned())),
            ProbeOutcome::Blocked(code) if code == "bridge.secure_storage_corrupt"
        ));
        assert!(matches!(
            probe_connection_state(IpcBridgeState::Ready, None),
            ProbeOutcome::Authorized
        ));
        assert!(
            matches!(
                probe_connection_state(IpcBridgeState::ShuttingDown, None),
                ProbeOutcome::Departing
            ),
            "正在退出的 Bridge 不能当成重连中的外部 Bridge 接管"
        );
    }

    #[test]
    fn 实例锁被另一个进程占着时不启动新的_bridge() {
        use super::instance_lock_held;

        let root = tempfile::tempdir().expect("临时目录有效");
        assert!(!instance_lock_held(root.path()), "从未启动过 Bridge");

        let path = root.path().join("bridge.lock");
        std::fs::write(&path, "pid=1\n").expect("锁文件可写");
        assert!(!instance_lock_held(root.path()), "上一个 Bridge 已经退出");

        let holder = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("锁文件可打开");
        holder
            .try_lock()
            .expect("检查之后锁已释放，Bridge 可以拿到");
        assert!(instance_lock_held(root.path()));
        holder.unlock().expect("可释放");
        assert!(!instance_lock_held(root.path()));
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
