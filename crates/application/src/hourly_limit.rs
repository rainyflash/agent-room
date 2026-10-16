//! 每个主体一小时里最多做几次。只记在进程里：控制面只有一个实例，重启后从头计也无妨。

use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
};

use agent_room_domain::{ids::PrincipalId, time::UtcMillis};

const HOUR_MILLIS: i64 = 3_600_000;

pub(crate) struct HourlyLimit {
    limit: usize,
    taken: Mutex<HashMap<PrincipalId, Vec<UtcMillis>>>,
}

/// 这一小时用完了：`retry_at` 之后再来。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HourlyLimitReached {
    pub(crate) retry_at: UtcMillis,
}

impl HourlyLimit {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            taken: Mutex::new(HashMap::new()),
        }
    }

    /// 记一次；最近一小时已经有 `limit` 次时不记，告诉什么时候能再来。
    pub(crate) fn take(
        &self,
        principal: PrincipalId,
        now: UtcMillis,
    ) -> Result<(), HourlyLimitReached> {
        let mut taken = self.taken.lock().unwrap_or_else(PoisonError::into_inner);
        let recent = taken.entry(principal).or_default();
        recent.retain(|at| now.value().saturating_sub(at.value()) < HOUR_MILLIS);
        if recent.len() >= self.limit {
            let oldest = recent
                .iter()
                .map(|at| at.value())
                .min()
                .unwrap_or(now.value());
            let retry_at = UtcMillis::new(oldest.saturating_add(HOUR_MILLIS)).unwrap_or(now);
            return Err(HourlyLimitReached { retry_at });
        }
        recent.push(now);
        Ok(())
    }
}
