use agent_room_domain::{
    DomainError, DomainResult,
    time::{DurationMillis, UtcMillis},
};

/// 守护进程在连接失败后的无上限、封顶退避策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconnectPolicy {
    initial_delay: DurationMillis,
    maximum_delay: DurationMillis,
}

impl ReconnectPolicy {
    /// 创建等抖动指数退避策略。
    ///
    /// # Errors
    ///
    /// 初始延迟大于最大延迟时返回校验错误。
    pub fn new(initial_delay: DurationMillis, maximum_delay: DurationMillis) -> DomainResult<Self> {
        if initial_delay > maximum_delay {
            return Err(DomainError::Validation {
                field: "bridge_reconnect_delay",
                reason: "初始延迟不能大于最大延迟",
            });
        }
        Ok(Self {
            initial_delay,
            maximum_delay,
        })
    }

    fn delay(self, consecutive_failures: u32, entropy: u64) -> DurationMillis {
        let shift = consecutive_failures.saturating_sub(1).min(63);
        let multiplier = 1_u64.checked_shl(shift).unwrap_or(u64::MAX);
        let ceiling = self
            .initial_delay
            .value()
            .saturating_mul(multiplier)
            .min(self.maximum_delay.value());
        let floor = (ceiling / 2).max(1);
        let span = ceiling.saturating_sub(floor).saturating_add(1);
        let delay = floor.saturating_add(entropy % span);
        DurationMillis::new(delay).expect("退避策略保证延迟非零")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconnectBackoff {
    policy: ReconnectPolicy,
    consecutive_failures: u32,
}

impl ReconnectBackoff {
    pub const fn new(policy: ReconnectPolicy) -> Self {
        Self {
            policy,
            consecutive_failures: 0,
        }
    }

    pub fn record_failure(&mut self, entropy: u64) -> DurationMillis {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.policy.delay(self.consecutive_failures, entropy)
    }

    pub const fn record_connected(&mut self) {
        self.consecutive_failures = 0;
    }

    pub const fn consecutive_failures(self) -> u32 {
        self.consecutive_failures
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRefreshPlan {
    DueNow,
    After(DurationMillis),
}

impl SessionRefreshPlan {
    pub fn calculate(
        access_token_expires_at: UtcMillis,
        now: UtcMillis,
        refresh_lead_time: DurationMillis,
    ) -> Self {
        let lead = i64::try_from(refresh_lead_time.value()).unwrap_or(i64::MAX);
        let refresh_at = access_token_expires_at.value().saturating_sub(lead);
        let remaining = refresh_at.saturating_sub(now.value());
        if remaining <= 0 {
            return Self::DueNow;
        }
        let Ok(remaining) = u64::try_from(remaining) else {
            return Self::DueNow;
        };
        let Ok(delay) = DurationMillis::new(remaining) else {
            return Self::DueNow;
        };
        Self::After(delay)
    }
}

#[cfg(test)]
mod tests {
    use agent_room_domain::time::{DurationMillis, UtcMillis};

    use super::{ReconnectBackoff, ReconnectPolicy, SessionRefreshPlan};

    fn duration(value: u64) -> DurationMillis {
        DurationMillis::new(value).expect("测试时长有效")
    }

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("测试时间有效")
    }

    #[test]
    fn 失败退避指数增长_带等抖动且封顶() {
        let policy = ReconnectPolicy::new(duration(1_000), duration(8_000)).expect("策略有效");
        let mut backoff = ReconnectBackoff::new(policy);

        assert_eq!(backoff.record_failure(0), duration(500));
        assert!((1_000..=2_000).contains(&backoff.record_failure(u64::MAX).value()));
        assert_eq!(backoff.record_failure(0), duration(2_000));
        assert_eq!(backoff.record_failure(0), duration(4_000));
        assert!((4_000..=8_000).contains(&backoff.record_failure(u64::MAX).value()));
        assert_eq!(backoff.consecutive_failures(), 5);
    }

    #[test]
    fn 连接成功后下一次失败从初始窗口重新开始() {
        let policy = ReconnectPolicy::new(duration(1_000), duration(8_000)).expect("策略有效");
        let mut backoff = ReconnectBackoff::new(policy);
        let _ = backoff.record_failure(0);
        let _ = backoff.record_failure(0);

        backoff.record_connected();

        assert_eq!(backoff.consecutive_failures(), 0);
        assert_eq!(backoff.record_failure(0), duration(500));
    }

    #[test]
    fn 设备跨休眠越过刷新时间后必须立即恢复() {
        let expires_at = time(120_000);
        let lead = duration(30_000);
        assert_eq!(
            SessionRefreshPlan::calculate(expires_at, time(60_000), lead),
            SessionRefreshPlan::After(duration(30_000))
        );
        assert_eq!(
            SessionRefreshPlan::calculate(expires_at, time(180_000), lead),
            SessionRefreshPlan::DueNow
        );
    }
}
