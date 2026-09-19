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
/// Bridge 重连退避允许配置的最大间隔；更长的值只可能来自损坏的输出。
const MAX_SERVER_RETRY_DELAY: Duration = Duration::from_mins(15);

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

#[derive(Debug)]
pub(crate) struct BridgeRestartPolicy {
    snapshot: BridgeLifecycleSnapshot,
    crashes: VecDeque<i64>,
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
        }
    }

    pub(crate) const fn snapshot(&self) -> &BridgeLifecycleSnapshot {
        &self.snapshot
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
    }

    pub(crate) fn set_diagnostic(&mut self, now_unix_ms: i64, code: impl Into<String>) {
        let code = code.into();
        self.snapshot.diagnostic_code = Some(code.clone());
        self.snapshot.last_failure_code = Some(code);
        self.snapshot.changed_at_unix_ms = now_unix_ms;
    }

    pub(crate) fn halt(&mut self, now_unix_ms: i64, code: impl Into<String>) {
        let code = code.into();
        self.snapshot.phase = BridgePhase::Halted;
        self.snapshot.diagnostic_code = Some(code.clone());
        self.snapshot.last_failure_code = Some(code);
        self.snapshot.next_retry_at_unix_ms = None;
        self.snapshot.changed_at_unix_ms = now_unix_ms;
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
        if shutting_down {
            self.snapshot.phase = BridgePhase::Stopped;
            self.snapshot.diagnostic_code = None;
            return ExitDecision::Stop;
        }

        if self.snapshot.phase == BridgePhase::AuthorizationRequired {
            // 重启会申请全新的设备码，无法继续用户刚批准的那次授权。
            // 保留具体失败原因，等待显式重试，避免把注册失败变成重复授权。
            self.snapshot.phase = BridgePhase::Halted;
            self.snapshot.diagnostic_code = Some("desktop.authorization.failed".to_owned());
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
        ResumeDecision, ResumeProbeState, decide_resume,
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
