//! 找回房间密钥后重读之前解不开的消息（设计见 `specs/room-key-recovery/pre-join-history.md`）。

use std::{
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

use super::{AgentOnlineSession, AgentSessionRuntime};

/// 重连后不必每次都重新请求：同一个 Bridge 进程里隔这么久才再请求一次。
const REREQUEST_INTERVAL: Duration = Duration::from_hours(1);

/// 上次对仍然解不开的消息重新请求房间密钥的时间。
#[derive(Default)]
pub(super) struct IsolatedKeyRerequests {
    last: Mutex<Option<Instant>>,
}

impl IsolatedKeyRerequests {
    fn due(&self, now: Instant) -> bool {
        let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
        if last.is_some_and(|at| now.duration_since(at) < REREQUEST_INTERVAL) {
            return false;
        }
        *last = Some(now);
        true
    }
}

/// 每轮同步后调用：取走新导入的会话，重读用到它们的隔离消息，读不完的留到下一轮。
///
/// 上线后的第一次同步还要对仍然解不开的消息重新请求房间密钥：等应答的请求只记在内存里，
/// Bridge 重启就没了。
pub(super) async fn recover_isolated_messages(
    runtime: &AgentSessionRuntime,
    online: &mut AgentOnlineSession,
    first_sync: bool,
) {
    if first_sync && runtime.isolated_key_rerequests.due(Instant::now()) {
        match runtime
            .messages
            .rerequest_isolated(online.matrix.as_ref())
            .await
        {
            Ok(0) => {}
            Ok(sessions) => tracing::info!(sessions, "对仍然解不开的消息重新请求房间密钥"),
            Err(failure) => tracing::warn!(
                failure_kind = ?failure.kind(),
                "读取仍然解不开的消息失败，下次上线再请求房间密钥"
            ),
        }
    }
    online
        .recovered_sessions
        .extend(online.matrix.take_recovered_sessions());
    if online.recovered_sessions.is_empty() {
        return;
    }
    let sessions = std::mem::take(&mut online.recovered_sessions)
        .into_iter()
        .collect::<Vec<_>>();
    match runtime
        .messages
        .recover_isolated(online.matrix.as_ref(), &sessions)
        .await
    {
        Ok(outcome) => {
            if outcome.recovered_events > 0 || outcome.rejected_events > 0 {
                tracing::info!(
                    recovered_events = outcome.recovered_events,
                    still_isolated = outcome.still_isolated,
                    rejected_events = outcome.rejected_events,
                    "找回房间密钥后重读了之前解不开的消息"
                );
            }
            online.recovered_sessions.extend(outcome.deferred);
        }
        Err(failure) => {
            tracing::warn!(failure_kind = ?failure.kind(), "重读解不开的消息失败，下一轮再试");
            online.recovered_sessions.extend(sessions);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{IsolatedKeyRerequests, REREQUEST_INTERVAL};

    #[test]
    fn 重连时一小时内只重新请求一次() {
        let rerequests = IsolatedKeyRerequests::default();
        let start = Instant::now();

        assert!(rerequests.due(start), "Bridge 启动后第一次上线要请求");
        assert!(!rerequests.due(start + Duration::from_mins(5)));
        assert!(rerequests.due(start + REREQUEST_INTERVAL));
    }
}
