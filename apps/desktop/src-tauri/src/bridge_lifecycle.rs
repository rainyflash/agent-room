use std::{collections::VecDeque, time::Duration};

use serde::{Deserialize, Serialize};

const MAX_AUTOMATIC_RESTARTS: usize = 3;
const RESTART_WINDOW: Duration = Duration::from_mins(10);
const RESTART_DELAYS: [Duration; MAX_AUTOMATIC_RESTARTS] = [
    Duration::from_secs(1),
    Duration::from_secs(4),
    Duration::from_secs(16),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BridgePhase {
    Discovering,
    Starting,
    AuthorizationRequired,
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
    Ready,
    Absent,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResumeDecision {
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
        self.snapshot.phase = BridgePhase::Ready;
        self.snapshot.ownership = Some(ownership);
        self.snapshot.diagnostic_code = None;
        self.snapshot.last_failure_code = None;
        self.snapshot.next_retry_at_unix_ms = None;
        self.snapshot.changed_at_unix_ms = now_unix_ms;
    }

    pub(crate) fn discovered_pending(&mut self, now_unix_ms: i64, ownership: BridgeOwnership) {
        self.snapshot.phase = BridgePhase::Starting;
        self.snapshot.ownership = Some(ownership);
        self.snapshot.diagnostic_code = Some("desktop.bridge.session_pending".to_owned());
        self.snapshot.next_retry_at_unix_ms = None;
        self.snapshot.changed_at_unix_ms = now_unix_ms;
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
        BridgeOwnership, BridgePhase, BridgeRestartPolicy, ExitDecision, ResumeDecision,
        ResumeProbeState, decide_resume,
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

        policy.discovered_pending(1_100, BridgeOwnership::External);

        assert_eq!(policy.snapshot().phase, BridgePhase::Starting);
        assert_eq!(policy.snapshot().ownership, Some(BridgeOwnership::External));
        assert_eq!(
            policy.snapshot().diagnostic_code.as_deref(),
            Some("desktop.bridge.session_pending")
        );
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
    fn 关闭期间的退出不会触发重启() {
        let mut policy = BridgeRestartPolicy::new(0);
        assert_eq!(policy.child_exited(1, Some(0), true), ExitDecision::Stop);
        assert_eq!(policy.snapshot().phase, BridgePhase::Stopped);
        assert_eq!(policy.snapshot().automatic_restart_count, 0);
    }

    #[test]
    fn 系统唤醒后重新探测而不沿用休眠前的假绿色状态() {
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
