use std::{collections::VecDeque, time::Duration};

use serde::{Deserialize, Serialize};

const MAX_AUTOMATIC_RESTARTS: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_mins(10);
const RESTART_DELAYS: [Duration; MAX_AUTOMATIC_RESTARTS] = [
    Duration::from_secs(1),
    Duration::from_secs(4),
    Duration::from_secs(16),
];
const SERVER_UNREACHABLE_DIAGNOSTIC: &str = "desktop.bridge.server_unreachable";
const AUTHORIZATION_FAILED_DIAGNOSTIC: &str = "desktop.authorization.failed";
const OTHER_INSTANCE_DIAGNOSTIC: &str = "desktop.bridge.other_instance_running";
/// Bridge 重连退避允许配置的最大间隔；更长的值只可能来自损坏的输出。
const MAX_SERVER_RETRY_DELAY: Duration = Duration::from_mins(15);
/// 另一个 Bridge 进程占着实例锁时最多等这么久再停机提示。上一个桌面留下的 Bridge 要逐个排空宿主
/// Agent 会话，每个最多约两分钟，收尾通常几十秒，也可能要几分钟。
const OTHER_INSTANCE_PATIENCE: Duration = Duration::from_mins(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BridgePhase {
    Discovering,
    Starting,
    Reconnecting,
    AuthorizationRequired,
    Authorized,
    Ready,
    RetryScheduled,
    Halted,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BridgeOwnership {
    External,
    Managed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BridgeLifecycleSnapshot {
    pub(crate) phase: BridgePhase,
    pub(crate) ownership: Option<BridgeOwnership>,
    pub(crate) diagnostic_code: Option<String>,
    pub(crate) last_failure_code: Option<String>,
    pub(crate) automatic_restart_count: usize,
    pub(crate) next_retry_at_unix_ms: Option<i64>,
    pub(crate) last_exit_code: Option<i32>,
    pub(crate) changed_at_unix_ms: i64,
}

impl BridgeLifecycleSnapshot {
    /// 停机的托管 Bridge 可以「重新授权这台电脑」：清除本机设备凭据后重新申请设备码。
    /// 外部 Bridge 不归桌面管，桌面不能重启它，也不应清除它的凭据；授权流程本身失败时，
    /// 本机尚无凭据，普通重试就会重新授权，不必再给第二个入口。
    pub(crate) fn device_reauthorization_available(&self) -> bool {
        self.phase == BridgePhase::Halted
            && self.ownership == Some(BridgeOwnership::Managed)
            && self.diagnostic_code.as_deref() != Some(AUTHORIZATION_FAILED_DIAGNOSTIC)
    }
}

#[derive(Debug)]
pub(crate) struct BridgeRestartPolicy {
    snapshot: BridgeLifecycleSnapshot,
    crashes: VecDeque<i64>,
    /// 开始等另一个 Bridge 进程让出实例锁的时间；见到能用的 Bridge、子进程另有退出原因或用户重试时清除。
    other_instance_since: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExitDecision {
    Stop,
    RetryAfter(Duration),
    Halt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResumeProbeState {
    Authorized,
    Ready,
    Absent,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectionProgress {
    Starting,
    Reconnecting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResumeDecision {
    Authorized(BridgeOwnership),
    Ready(BridgeOwnership),
    StartManaged,
    KeepProbing,
    Halt,
}

/// 桌面没有托管子进程时在本机看到的 Bridge。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UnmanagedBridge {
    Ready,
    Authorized,
    Pending(ConnectionProgress),
    /// 有 Bridge 应答，但它正在退出。
    Departing,
    /// 没有 Bridge 应答，却有 Bridge 进程占着实例锁：它还在启动，或者已经关闭 IPC、正在收尾。
    Locked,
    /// 本机没有 Bridge。
    Absent,
    /// 应答的 Bridge 处于桌面处理不了的状态。
    Blocked(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnmanagedAction {
    None,
    StartManaged,
    /// 刚刚停机，提示用户。
    NotifyStopped,
}

pub(crate) const fn decide_resume(
    probe: ResumeProbeState,
    managed_child_active: bool,
    phase: BridgePhase,
) -> ResumeDecision {
    match probe {
        ResumeProbeState::Authorized => ResumeDecision::Authorized(if managed_child_active {
            BridgeOwnership::Managed
        } else {
            BridgeOwnership::External
        }),
        ResumeProbeState::Ready => ResumeDecision::Ready(if managed_child_active {
            BridgeOwnership::Managed
        } else {
            BridgeOwnership::External
        }),
        ResumeProbeState::Absent
            if !managed_child_active && !matches!(phase, BridgePhase::Halted) =>
        {
            ResumeDecision::StartManaged
        }
        ResumeProbeState::Absent => ResumeDecision::KeepProbing,
        ResumeProbeState::Blocked => ResumeDecision::Halt,
    }
}

impl BridgeRestartPolicy {
    pub(crate) fn new(now_unix_ms: i64) -> Self {
        Self {
            snapshot: BridgeLifecycleSnapshot {
                phase: BridgePhase::Discovering,
                ownership: None,
                diagnostic_code: None,
                last_failure_code: None,
                automatic_restart_count: 0,
                next_retry_at_unix_ms: None,
                last_exit_code: None,
                changed_at_unix_ms: now_unix_ms,
            },
            crashes: VecDeque::new(),
            other_instance_since: None,
        }
    }

    pub(crate) const fn snapshot(&self) -> &BridgeLifecycleSnapshot {
        &self.snapshot
    }

    /// 定时探测是否继续。托管 Bridge 按退避等待重启、因反复崩溃或授权失败停机、或者已经关闭时不探测，
    /// 免得绕过重启预算；外部或来源不明的 Bridge 停机时仍然探测，它一退出桌面就接手启动托管 Bridge。
    pub(crate) const fn probes_periodically(&self, managed_child_active: bool) -> bool {
        match self.snapshot.phase {
            BridgePhase::Stopped | BridgePhase::RetryScheduled => false,
            BridgePhase::Halted => {
                !managed_child_active
                    && !matches!(self.snapshot.ownership, Some(BridgeOwnership::Managed))
            }
            _ => true,
        }
    }

    /// 桌面没有托管子进程时根据本机看到的 Bridge 更新状态，并返回下一步。
    ///
    /// 能应答的 Bridge 只观察、不争夺生命周期。正在退出、或占着实例锁却不应答的 Bridge 不接管：
    /// 桌面也不能结束一个不归自己管的进程，只能等它走完。什么都没有时由桌面启动托管 Bridge，
    /// 所以接管过的外部 Bridge 一消失，桌面就会接手。
    pub(crate) fn observe_unmanaged(
        &mut self,
        now_unix_ms: i64,
        bridge: UnmanagedBridge,
    ) -> UnmanagedAction {
        match bridge {
            UnmanagedBridge::Ready => self.discovered_ready(now_unix_ms, BridgeOwnership::External),
            UnmanagedBridge::Authorized => {
                self.discovered_authorized(now_unix_ms, BridgeOwnership::External);
            }
            UnmanagedBridge::Pending(progress) => {
                self.discovered_pending(now_unix_ms, BridgeOwnership::External, progress);
            }
            UnmanagedBridge::Departing | UnmanagedBridge::Locked => {
                return self.await_other_instance(now_unix_ms);
            }
            UnmanagedBridge::Absent => return UnmanagedAction::StartManaged,
            UnmanagedBridge::Blocked(code) => self.halt(now_unix_ms, code),
        }
        UnmanagedAction::None
    }

    /// 等另一个 Bridge 进程走完。这不是托管 Bridge 崩溃，不占用自动重启预算；等得太久才停机提示，
    /// 之后定时探测照常进行，它一走就启动托管 Bridge。
    fn await_other_instance(&mut self, now_unix_ms: i64) -> UnmanagedAction {
        if self.snapshot.phase == BridgePhase::Halted
            && self.snapshot.diagnostic_code.as_deref() == Some(OTHER_INSTANCE_DIAGNOSTIC)
        {
            return UnmanagedAction::None;
        }
        let since = *self.other_instance_since.get_or_insert(now_unix_ms);
        let patience_ms = i64::try_from(OTHER_INSTANCE_PATIENCE.as_millis()).unwrap_or(i64::MAX);
        let exhausted = now_unix_ms.saturating_sub(since) >= patience_ms;
        let phase = if exhausted {
            BridgePhase::Halted
        } else {
            BridgePhase::Discovering
        };
        if self.snapshot.phase != phase
            || self.snapshot.ownership.is_some()
            || self.snapshot.diagnostic_code.as_deref() != Some(OTHER_INSTANCE_DIAGNOSTIC)
        {
            self.snapshot.changed_at_unix_ms = now_unix_ms;
        }
        self.snapshot.phase = phase;
        self.snapshot.ownership = None;
        self.snapshot.diagnostic_code = Some(OTHER_INSTANCE_DIAGNOSTIC.to_owned());
        self.snapshot.next_retry_at_unix_ms = None;
        if exhausted {
            UnmanagedAction::NotifyStopped
        } else {
            UnmanagedAction::None
        }
    }

    pub(crate) fn discovered_ready(&mut self, now_unix_ms: i64, ownership: BridgeOwnership) {
        if self.snapshot.phase != BridgePhase::Ready || self.snapshot.ownership != Some(ownership) {
            self.snapshot.changed_at_unix_ms = now_unix_ms;
        }
        self.snapshot.phase = BridgePhase::Ready;
        self.snapshot.ownership = Some(ownership);
        self.snapshot.diagnostic_code = None;
        self.snapshot.last_failure_code = None;
        self.snapshot.next_retry_at_unix_ms = None;
        self.other_instance_since = None;
    }

    pub(crate) fn discovered_authorized(&mut self, now_unix_ms: i64, ownership: BridgeOwnership) {
        if self.snapshot.phase != BridgePhase::Authorized
            || self.snapshot.ownership != Some(ownership)
        {
            self.snapshot.changed_at_unix_ms = now_unix_ms;
        }
        self.snapshot.phase = BridgePhase::Authorized;
        self.snapshot.ownership = Some(ownership);
        self.snapshot.diagnostic_code = None;
        self.snapshot.last_failure_code = None;
        self.snapshot.next_retry_at_unix_ms = None;
        self.other_instance_since = None;
    }

    pub(crate) fn discovered_pending(
        &mut self,
        now_unix_ms: i64,
        ownership: BridgeOwnership,
        progress: ConnectionProgress,
    ) {
        let phase = match progress {
            ConnectionProgress::Starting => BridgePhase::Starting,
            ConnectionProgress::Reconnecting => BridgePhase::Reconnecting,
        };
        if self.snapshot.phase != phase || self.snapshot.ownership != Some(ownership) {
            self.snapshot.changed_at_unix_ms = now_unix_ms;
        }
        self.snapshot.phase = phase;
        self.snapshot.ownership = Some(ownership);
        self.snapshot.diagnostic_code = Some(
            match progress {
                ConnectionProgress::Starting => "desktop.bridge.session_pending",
                ConnectionProgress::Reconnecting => "desktop.bridge.reconnecting",
            }
            .to_owned(),
        );
        self.snapshot.next_retry_at_unix_ms = None;
        self.other_instance_since = None;
    }

    pub(crate) fn starting(&mut self, now_unix_ms: i64) {
        self.snapshot.phase = BridgePhase::Starting;
        self.snapshot.ownership = Some(BridgeOwnership::Managed);
        self.snapshot.diagnostic_code = None;
        self.snapshot.next_retry_at_unix_ms = None;
        self.snapshot.changed_at_unix_ms = now_unix_ms;
    }

    pub(crate) fn authorization_required(&mut self, now_unix_ms: i64) {
        self.snapshot.phase = BridgePhase::AuthorizationRequired;
        self.snapshot.ownership = Some(BridgeOwnership::Managed);
        self.snapshot.diagnostic_code = None;
        self.snapshot.last_failure_code = None;
        self.snapshot.next_retry_at_unix_ms = None;
        self.snapshot.changed_at_unix_ms = now_unix_ms;
        self.other_instance_since = None;
    }

    /// Bridge 仍在运行，只是连不上服务器，已按自己的退避安排好下一次尝试。
    /// 这不是崩溃：不占用自动重启预算，也不发停止通知；用户随时可以显式重试。
    pub(crate) fn server_unreachable(
        &mut self,
        now_unix_ms: i64,
        failure_code: impl Into<String>,
        retry_after: Duration,
    ) {
        let already_waiting = self.snapshot.phase == BridgePhase::RetryScheduled
            && self.snapshot.diagnostic_code.as_deref() == Some(SERVER_UNREACHABLE_DIAGNOSTIC);
        if !already_waiting {
            self.snapshot.changed_at_unix_ms = now_unix_ms;
        }
        let delay_ms =
            i64::try_from(retry_after.min(MAX_SERVER_RETRY_DELAY).as_millis()).unwrap_or(i64::MAX);
        self.snapshot.phase = BridgePhase::RetryScheduled;
        self.snapshot.ownership = Some(BridgeOwnership::Managed);
        self.snapshot.diagnostic_code = Some(SERVER_UNREACHABLE_DIAGNOSTIC.to_owned());
        self.snapshot.last_failure_code = Some(failure_code.into());
        self.snapshot.next_retry_at_unix_ms = Some(now_unix_ms.saturating_add(delay_ms));
        self.other_instance_since = None;
    }

    pub(crate) fn set_diagnostic(&mut self, now_unix_ms: i64, code: impl Into<String>) {
        let code = code.into();
        self.snapshot.diagnostic_code = Some(code.clone());
        self.snapshot.last_failure_code = Some(code);
        self.snapshot.changed_at_unix_ms = now_unix_ms;
    }

    pub(crate) fn halt(&mut self, now_unix_ms: i64, code: impl Into<String>) {
        let code = code.into();
        // 外部 Bridge 停机后仍会定时探测，同一原因反复出现时保留最初停机的时间。
        if self.snapshot.phase != BridgePhase::Halted
            || self.snapshot.diagnostic_code.as_deref() != Some(code.as_str())
        {
            self.snapshot.changed_at_unix_ms = now_unix_ms;
        }
        self.snapshot.phase = BridgePhase::Halted;
        self.snapshot.diagnostic_code = Some(code.clone());
        self.snapshot.last_failure_code = Some(code);
        self.snapshot.next_retry_at_unix_ms = None;
    }

    pub(crate) fn child_exited(
        &mut self,
        now_unix_ms: i64,
        exit_code: Option<i32>,
        shutting_down: bool,
    ) -> ExitDecision {
        self.snapshot.last_exit_code = exit_code;
        self.snapshot.changed_at_unix_ms = now_unix_ms;
        self.snapshot.ownership = Some(BridgeOwnership::Managed);
        self.snapshot.next_retry_at_unix_ms = None;
        self.other_instance_since = None;
        if shutting_down {
            self.snapshot.phase = BridgePhase::Stopped;
            self.snapshot.diagnostic_code = None;
            return ExitDecision::Stop;
        }

        if self.snapshot.phase == BridgePhase::AuthorizationRequired {
            // 重启会申请全新的设备码，无法继续用户刚批准的那次授权。
            // 保留具体失败原因，等待显式重试，避免把注册失败变成重复授权。
            self.snapshot.phase = BridgePhase::Halted;
            self.snapshot.diagnostic_code = Some(AUTHORIZATION_FAILED_DIAGNOSTIC.to_owned());
            return ExitDecision::Halt;
        }

        let restart_window_ms = i64::try_from(RESTART_WINDOW.as_millis()).unwrap_or(i64::MAX);
        while self
            .crashes
            .front()
            .is_some_and(|recorded| now_unix_ms.saturating_sub(*recorded) > restart_window_ms)
        {
            self.crashes.pop_front();
        }
        self.crashes.push_back(now_unix_ms);
        self.snapshot.automatic_restart_count = self.crashes.len();

        let Some(delay) = RESTART_DELAYS
            .get(self.crashes.len().saturating_sub(1))
            .copied()
        else {
            self.snapshot.phase = BridgePhase::Halted;
            self.snapshot.diagnostic_code =
                Some("desktop.bridge.restart_budget_exhausted".to_owned());
            return ExitDecision::Halt;
        };
        let delay_ms = i64::try_from(delay.as_millis()).unwrap_or(i64::MAX);
        self.snapshot.phase = BridgePhase::RetryScheduled;
        self.snapshot.diagnostic_code = Some("desktop.bridge.process_exited".to_owned());
        self.snapshot.next_retry_at_unix_ms = Some(now_unix_ms.saturating_add(delay_ms));
        ExitDecision::RetryAfter(delay)
    }

    pub(crate) fn explicit_retry(&mut self, now_unix_ms: i64) {
        self.crashes.clear();
        self.snapshot.automatic_restart_count = 0;
        self.other_instance_since = None;
        self.starting(now_unix_ms);
    }

    pub(crate) fn stop(&mut self, now_unix_ms: i64) {
        self.snapshot.phase = BridgePhase::Stopped;
        self.snapshot.next_retry_at_unix_ms = None;
        self.snapshot.changed_at_unix_ms = now_unix_ms;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BridgeOwnership, BridgePhase, BridgeRestartPolicy, ConnectionProgress, ExitDecision,
        ResumeDecision, ResumeProbeState, UnmanagedAction, UnmanagedBridge, decide_resume,
    };

    #[test]
    fn 已运行_bridge_被发现后不会争夺生命周期所有权() {
        let mut policy = BridgeRestartPolicy::new(1_000);
        policy.discovered_ready(1_100, BridgeOwnership::External);

        assert_eq!(policy.snapshot().phase, BridgePhase::Ready);
        assert_eq!(policy.snapshot().ownership, Some(BridgeOwnership::External));
    }

    #[test]
    fn 可达但未就绪的外部_bridge_不会被误报为_ready() {
        let mut policy = BridgeRestartPolicy::new(1_000);

        policy.discovered_pending(
            1_100,
            BridgeOwnership::External,
            ConnectionProgress::Starting,
        );

        assert_eq!(policy.snapshot().phase, BridgePhase::Starting);
        assert_eq!(policy.snapshot().ownership, Some(BridgeOwnership::External));
        assert_eq!(
            policy.snapshot().diagnostic_code.as_deref(),
            Some("desktop.bridge.session_pending")
        );
    }

    #[test]
    fn 设备已授权但尚无_agent_时暴露可操作阶段() {
        let mut policy = BridgeRestartPolicy::new(1_000);

        policy.discovered_authorized(1_100, BridgeOwnership::Managed);

        assert_eq!(policy.snapshot().phase, BridgePhase::Authorized);
        assert_eq!(policy.snapshot().ownership, Some(BridgeOwnership::Managed));
        assert_eq!(policy.snapshot().diagnostic_code, None);
    }

    #[test]
    fn repeated_probes_preserve_reconnect_start_time_until_recovery() {
        let mut policy = BridgeRestartPolicy::new(1_000);
        policy.discovered_pending(
            1_100,
            BridgeOwnership::Managed,
            ConnectionProgress::Reconnecting,
        );
        policy.discovered_pending(
            2_100,
            BridgeOwnership::Managed,
            ConnectionProgress::Reconnecting,
        );
        assert_eq!(policy.snapshot().phase, BridgePhase::Reconnecting);
        assert_eq!(policy.snapshot().changed_at_unix_ms, 1_100);
        policy.discovered_authorized(3_100, BridgeOwnership::Managed);
        assert_eq!(policy.snapshot().phase, BridgePhase::Authorized);
        assert_eq!(policy.snapshot().diagnostic_code, None);
    }

    #[test]
    fn 自动重启有指数退避且第四次崩溃进入停机态() {
        let mut policy = BridgeRestartPolicy::new(0);
        policy.starting(0);
        policy.set_diagnostic(50, "bridge.identity.discovery_failed");

        assert_eq!(
            policy.child_exited(100, Some(10), false),
            ExitDecision::RetryAfter(std::time::Duration::from_secs(1))
        );
        assert_eq!(
            policy.child_exited(200, Some(11), false),
            ExitDecision::RetryAfter(std::time::Duration::from_secs(4))
        );
        assert_eq!(
            policy.child_exited(300, Some(12), false),
            ExitDecision::RetryAfter(std::time::Duration::from_secs(16))
        );
        assert_eq!(
            policy.child_exited(400, Some(13), false),
            ExitDecision::Halt
        );
        assert_eq!(policy.snapshot().phase, BridgePhase::Halted);
        assert_eq!(
            policy.snapshot().diagnostic_code.as_deref(),
            Some("desktop.bridge.restart_budget_exhausted")
        );
        assert_eq!(policy.snapshot().last_exit_code, Some(13));
        assert_eq!(
            policy.snapshot().last_failure_code.as_deref(),
            Some("bridge.identity.discovery_failed")
        );
        assert!(
            !policy.probes_periodically(false),
            "反复崩溃停机后不能靠定时探测绕过预算再启动"
        );
    }

    #[test]
    fn 接管的外部_bridge_退出后桌面启动托管_bridge() {
        let mut policy = BridgeRestartPolicy::new(0);
        assert_eq!(
            policy.observe_unmanaged(1_000, UnmanagedBridge::Ready),
            UnmanagedAction::None
        );
        assert_eq!(policy.snapshot().ownership, Some(BridgeOwnership::External));
        assert!(policy.probes_periodically(false));

        // 上一个桌面留下的 Bridge 关闭了 IPC，还在收尾，占着实例锁：不接管也不启动竞争进程。
        assert_eq!(
            policy.observe_unmanaged(3_000, UnmanagedBridge::Locked),
            UnmanagedAction::None
        );
        assert_eq!(policy.snapshot().phase, BridgePhase::Discovering);
        assert_eq!(policy.snapshot().ownership, None);
        assert_eq!(
            policy.snapshot().diagnostic_code.as_deref(),
            Some("desktop.bridge.other_instance_running")
        );
        assert!(policy.probes_periodically(false), "等待期间继续探测");
        assert_eq!(
            policy.observe_unmanaged(5_000, UnmanagedBridge::Locked),
            UnmanagedAction::None
        );
        assert_eq!(
            policy.snapshot().changed_at_unix_ms,
            3_000,
            "持续等待不刷新开始时间"
        );

        // 它彻底退出后桌面接手。
        assert_eq!(
            policy.observe_unmanaged(33_000, UnmanagedBridge::Absent),
            UnmanagedAction::StartManaged
        );
        policy.starting(33_000);
        assert_eq!(policy.snapshot().phase, BridgePhase::Starting);
        assert_eq!(policy.snapshot().ownership, Some(BridgeOwnership::Managed));
        assert_eq!(
            policy.snapshot().automatic_restart_count,
            0,
            "等待不占重启预算"
        );
        // 托管 Bridge 真正崩溃时仍按原有预算重启。
        assert_eq!(
            policy.child_exited(34_000, Some(1), false),
            ExitDecision::RetryAfter(std::time::Duration::from_secs(1))
        );
    }

    #[test]
    fn 正在退出的_bridge_不会被当成外部_bridge_接管() {
        let mut policy = BridgeRestartPolicy::new(0);

        assert_eq!(
            policy.observe_unmanaged(1_000, UnmanagedBridge::Departing),
            UnmanagedAction::None
        );

        assert_eq!(policy.snapshot().phase, BridgePhase::Discovering);
        assert_eq!(policy.snapshot().ownership, None);
        assert!(policy.probes_periodically(false));
        assert_eq!(
            policy.observe_unmanaged(3_000, UnmanagedBridge::Absent),
            UnmanagedAction::StartManaged
        );
    }

    #[test]
    fn 外部_bridge_停机后继续探测_它一退出就启动托管_bridge() {
        let mut policy = BridgeRestartPolicy::new(0);
        policy.observe_unmanaged(1_000, UnmanagedBridge::Ready);

        assert_eq!(
            policy.observe_unmanaged(
                3_000,
                UnmanagedBridge::Blocked("desktop.bridge.offline".to_owned())
            ),
            UnmanagedAction::None
        );
        assert_eq!(policy.snapshot().phase, BridgePhase::Halted);
        assert_eq!(policy.snapshot().ownership, Some(BridgeOwnership::External));
        assert!(
            policy.probes_periodically(false),
            "外部 Bridge 停机后仍要发现它退出"
        );
        policy.observe_unmanaged(
            5_000,
            UnmanagedBridge::Blocked("desktop.bridge.offline".to_owned()),
        );
        assert_eq!(policy.snapshot().changed_at_unix_ms, 3_000);

        assert_eq!(
            policy.observe_unmanaged(7_000, UnmanagedBridge::Absent),
            UnmanagedAction::StartManaged
        );

        // 启动时第一次探测就受阻，也继续探测，而不是永远停在这里。
        let mut initial = BridgeRestartPolicy::new(0);
        initial.observe_unmanaged(
            1,
            UnmanagedBridge::Blocked("bridge.ipc.credentials_unavailable".to_owned()),
        );
        assert_eq!(initial.snapshot().phase, BridgePhase::Halted);
        assert!(initial.probes_periodically(false));
    }

    fn patience_ms() -> i64 {
        i64::try_from(super::OTHER_INSTANCE_PATIENCE.as_millis()).expect("等待上限可用毫秒表示")
    }

    #[test]
    fn 另一个_bridge_久占实例锁时停机提示一次并继续等它() {
        let mut policy = BridgeRestartPolicy::new(0);
        policy.starting(0);
        // 托管子进程报告 `bridge.already_running` 后退出。
        assert_eq!(
            policy.observe_unmanaged(100, UnmanagedBridge::Locked),
            UnmanagedAction::None
        );
        assert_eq!(policy.snapshot().automatic_restart_count, 0);
        assert_eq!(
            policy.observe_unmanaged(100 + patience_ms() - 1, UnmanagedBridge::Locked),
            UnmanagedAction::None
        );

        let halted_at = 100 + patience_ms();
        assert_eq!(
            policy.observe_unmanaged(halted_at, UnmanagedBridge::Locked),
            UnmanagedAction::NotifyStopped
        );
        assert_eq!(policy.snapshot().phase, BridgePhase::Halted);
        assert_eq!(policy.snapshot().ownership, None);
        assert_eq!(
            policy.snapshot().diagnostic_code.as_deref(),
            Some("desktop.bridge.other_instance_running")
        );
        assert!(
            !policy.snapshot().device_reauthorization_available(),
            "重新授权放不开别的进程占着的锁"
        );
        assert_eq!(
            policy.observe_unmanaged(halted_at + 2_000, UnmanagedBridge::Locked),
            UnmanagedAction::None,
            "只提示一次"
        );
        assert_eq!(policy.snapshot().changed_at_unix_ms, halted_at);
        assert!(policy.probes_periodically(false), "停机后仍等它走");
        assert_eq!(
            policy.observe_unmanaged(halted_at + 60_000, UnmanagedBridge::Absent),
            UnmanagedAction::StartManaged
        );
    }

    #[test]
    fn 能用的_bridge_或用户重试会重新计算等待时间() {
        let mut policy = BridgeRestartPolicy::new(0);
        policy.observe_unmanaged(0, UnmanagedBridge::Locked);
        policy.observe_unmanaged(60_000, UnmanagedBridge::Ready);
        assert_eq!(
            policy.observe_unmanaged(100_000, UnmanagedBridge::Locked),
            UnmanagedAction::None
        );
        let before_patience = 100_000 + patience_ms() - 1;
        assert_eq!(
            policy.observe_unmanaged(before_patience, UnmanagedBridge::Locked),
            UnmanagedAction::None
        );

        policy.explicit_retry(before_patience);
        let retried = before_patience + 1;
        assert_eq!(
            policy.observe_unmanaged(retried, UnmanagedBridge::Locked),
            UnmanagedAction::None
        );
        assert_eq!(
            policy.observe_unmanaged(retried + patience_ms(), UnmanagedBridge::Locked),
            UnmanagedAction::NotifyStopped
        );
    }

    #[test]
    fn 用户显式重试才会重置崩溃预算() {
        let mut policy = BridgeRestartPolicy::new(0);
        for now in 1..=4 {
            let _ = policy.child_exited(now, Some(1), false);
        }
        assert_eq!(policy.snapshot().phase, BridgePhase::Halted);

        policy.explicit_retry(10);

        assert_eq!(policy.snapshot().phase, BridgePhase::Starting);
        assert_eq!(policy.snapshot().automatic_restart_count, 0);
    }

    #[test]
    fn 授权期间失败不会自动申请新设备码并保留错误() {
        for code in [
            "bridge.identity_assertion_invalid",
            "bridge.authorization_denied",
            "bridge.authorization_expired",
            "bridge.secure_storage_unavailable",
            "bridge.registration_outcome_unknown",
        ] {
            let mut policy = BridgeRestartPolicy::new(0);
            policy.authorization_required(1);
            policy.set_diagnostic(2, code);

            assert_eq!(policy.child_exited(3, Some(1), false), ExitDecision::Halt);
            assert_eq!(policy.snapshot().phase, BridgePhase::Halted);
            assert_eq!(policy.snapshot().last_failure_code.as_deref(), Some(code));
            assert_eq!(
                policy.snapshot().diagnostic_code.as_deref(),
                Some("desktop.authorization.failed")
            );
            assert_eq!(policy.snapshot().next_retry_at_unix_ms, None);
            assert_eq!(policy.snapshot().automatic_restart_count, 0);
            assert_eq!(
                decide_resume(ResumeProbeState::Absent, false, BridgePhase::Halted),
                ResumeDecision::KeepProbing
            );

            policy.explicit_retry(4);
            assert_eq!(policy.snapshot().phase, BridgePhase::Starting);
        }
    }

    #[test]
    fn 授权期间无诊断退出也不会自动创建新授权() {
        let mut policy = BridgeRestartPolicy::new(0);
        policy.authorization_required(1);
        assert_eq!(policy.child_exited(2, None, false), ExitDecision::Halt);
        assert_eq!(policy.snapshot().last_failure_code, None);
        assert_eq!(
            policy.snapshot().diagnostic_code.as_deref(),
            Some("desktop.authorization.failed")
        );
    }

    #[test]
    fn 授权已保存后启动故障仍可使用已有凭据自动重连() {
        let mut policy = BridgeRestartPolicy::new(0);
        policy.authorization_required(1);
        policy.discovered_pending(2, BridgeOwnership::Managed, ConnectionProgress::Starting);
        policy.set_diagnostic(3, "bridge.matrix_store_unavailable");
        assert_eq!(
            policy.child_exited(4, Some(1), false),
            ExitDecision::RetryAfter(std::time::Duration::from_secs(1))
        );
        assert_eq!(policy.snapshot().phase, BridgePhase::RetryScheduled);
    }

    #[test]
    fn 连不上服务器时显示下次尝试时间且不占用自动重启预算() {
        let mut policy = BridgeRestartPolicy::new(0);
        policy.starting(10);

        policy.server_unreachable(
            1_000,
            "bridge.identity_provider_unavailable",
            std::time::Duration::from_millis(2_500),
        );

        let snapshot = policy.snapshot();
        assert_eq!(snapshot.phase, BridgePhase::RetryScheduled);
        assert_eq!(snapshot.ownership, Some(BridgeOwnership::Managed));
        assert_eq!(
            snapshot.diagnostic_code.as_deref(),
            Some("desktop.bridge.server_unreachable")
        );
        assert_eq!(
            snapshot.last_failure_code.as_deref(),
            Some("bridge.identity_provider_unavailable")
        );
        assert_eq!(snapshot.next_retry_at_unix_ms, Some(3_500));
        assert_eq!(snapshot.automatic_restart_count, 0);
        assert_eq!(snapshot.changed_at_unix_ms, 1_000);
    }

    #[test]
    fn 服务器长时间不可达也不会停机并保留开始等待的时间() {
        let mut policy = BridgeRestartPolicy::new(0);
        policy.starting(0);

        for minute in 1..=30_i64 {
            policy.server_unreachable(
                minute * 60_000,
                "bridge.identity_provider_unavailable",
                std::time::Duration::from_mins(1),
            );
        }

        let snapshot = policy.snapshot();
        assert_eq!(snapshot.phase, BridgePhase::RetryScheduled);
        assert_eq!(snapshot.changed_at_unix_ms, 60_000);
        assert_eq!(snapshot.next_retry_at_unix_ms, Some(31 * 60_000));
        assert_eq!(snapshot.automatic_restart_count, 0);
        // Bridge 本身仍在运行，唤醒后不应再启动一个与它竞争的进程。
        assert_eq!(
            decide_resume(ResumeProbeState::Absent, true, BridgePhase::RetryScheduled),
            ResumeDecision::KeepProbing
        );
        // 真正的崩溃仍按原有预算重启。
        assert_eq!(
            policy.child_exited(31 * 60_000, Some(1), false),
            ExitDecision::RetryAfter(std::time::Duration::from_secs(1))
        );
        assert_eq!(
            policy.snapshot().diagnostic_code.as_deref(),
            Some("desktop.bridge.process_exited")
        );
    }

    #[test]
    fn 服务器恢复后进入授权并清除连不上的诊断() {
        let mut policy = BridgeRestartPolicy::new(0);
        policy.starting(0);
        policy.server_unreachable(
            100,
            "bridge.identity_provider_unavailable",
            std::time::Duration::from_secs(1),
        );

        policy.authorization_required(900);

        let snapshot = policy.snapshot();
        assert_eq!(snapshot.phase, BridgePhase::AuthorizationRequired);
        assert_eq!(snapshot.diagnostic_code, None);
        assert_eq!(snapshot.last_failure_code, None);
        assert_eq!(snapshot.next_retry_at_unix_ms, None);
    }

    #[test]
    fn 用户可以在等待服务器时立即重试() {
        let mut policy = BridgeRestartPolicy::new(0);
        policy.server_unreachable(
            100,
            "bridge.identity_provider_unavailable",
            std::time::Duration::from_secs(30),
        );

        policy.explicit_retry(200);

        assert_eq!(policy.snapshot().phase, BridgePhase::Starting);
        assert_eq!(policy.snapshot().next_retry_at_unix_ms, None);
    }

    #[test]
    fn 异常的重试间隔被限制在_bridge_退避上限内() {
        let mut policy = BridgeRestartPolicy::new(0);

        policy.server_unreachable(
            0,
            "bridge.identity_provider_unavailable",
            std::time::Duration::MAX,
        );

        assert_eq!(
            policy.snapshot().next_retry_at_unix_ms,
            Some(15 * 60 * 1_000)
        );
    }

    #[test]
    fn 停机的托管_bridge_提供重新授权而运行中或外部的不提供() {
        let mut policy = BridgeRestartPolicy::new(0);
        policy.starting(0);
        policy.set_diagnostic(1, "bridge.refresh_outcome_unknown");
        assert!(!policy.snapshot().device_reauthorization_available());
        for now in 2..=5 {
            let _ = policy.child_exited(now, Some(1), false);
        }
        assert_eq!(policy.snapshot().phase, BridgePhase::Halted);
        assert!(
            policy.snapshot().device_reauthorization_available(),
            "自动重启预算耗尽后应能从停机视图重新授权"
        );

        policy.explicit_retry(6);
        assert!(!policy.snapshot().device_reauthorization_available());

        let mut external = BridgeRestartPolicy::new(0);
        external.discovered_ready(1, BridgeOwnership::External);
        external.halt(2, "desktop.bridge.offline");
        assert!(
            !external.snapshot().device_reauthorization_available(),
            "外部 Bridge 的凭据不归桌面清除"
        );
    }

    #[test]
    fn 授权流程失败的停机视图只保留普通重试() {
        let mut policy = BridgeRestartPolicy::new(0);
        policy.authorization_required(1);
        assert_eq!(policy.child_exited(2, Some(1), false), ExitDecision::Halt);

        assert!(!policy.snapshot().device_reauthorization_available());
    }

    #[test]
    fn 关闭期间的退出不会触发重启() {
        let mut policy = BridgeRestartPolicy::new(0);
        assert_eq!(policy.child_exited(1, Some(0), true), ExitDecision::Stop);
        assert_eq!(policy.snapshot().phase, BridgePhase::Stopped);
        assert_eq!(policy.snapshot().automatic_restart_count, 0);
    }

    #[test]
    fn 系统唤醒后重新探测而不沿用休眠前的假绿色状态() {
        assert_eq!(
            decide_resume(ResumeProbeState::Authorized, true, BridgePhase::Starting),
            ResumeDecision::Authorized(BridgeOwnership::Managed)
        );
        assert_eq!(
            decide_resume(ResumeProbeState::Absent, false, BridgePhase::Ready),
            ResumeDecision::StartManaged
        );
        assert_eq!(
            decide_resume(ResumeProbeState::Ready, false, BridgePhase::Ready),
            ResumeDecision::Ready(BridgeOwnership::External)
        );
        assert_eq!(
            decide_resume(ResumeProbeState::Blocked, true, BridgePhase::Ready),
            ResumeDecision::Halt
        );
        assert_eq!(
            decide_resume(ResumeProbeState::Absent, true, BridgePhase::Ready),
            ResumeDecision::KeepProbing
        );
    }
}
