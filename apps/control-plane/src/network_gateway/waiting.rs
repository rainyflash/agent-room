//! 此刻有几个网络 Agent 的请求在等消息（HTTP 和远程 MCP 都算），运行指标每轮读一次。首发当天
//! 拿它看服务器的负担：Synapse 的活大致跟“在等的个数 × 大厅里的动静”成正比
//! （`specs/launch-capacity/design.md`）。

use std::sync::atomic::{AtomicI64, Ordering};

static WAITING: Counter = Counter::new();

/// 开始等：返回的守卫丢掉时（等完了、出错了、对方断开了）减回去。
pub(crate) fn start() -> Waiting<'static> {
    WAITING.start()
}

/// 此刻在等的个数。
pub(crate) fn now() -> i64 {
    WAITING.now()
}

struct Counter(AtomicI64);

impl Counter {
    const fn new() -> Self {
        Self(AtomicI64::new(0))
    }

    fn start(&self) -> Waiting<'_> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Waiting(self)
    }

    fn now(&self) -> i64 {
        self.0.load(Ordering::Relaxed)
    }
}

pub(crate) struct Waiting<'a>(&'a Counter);

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.0.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::Counter;

    #[test]
    fn 开始等加一_等完或者中途丢掉都减回去() {
        let counter = Counter::new();
        let first = counter.start();
        let second = counter.start();
        assert_eq!(counter.now(), 2);
        drop(first);
        assert_eq!(counter.now(), 1);
        let pending = async move {
            let _waiting = second;
            std::future::pending::<()>().await;
        };
        // 对方断开时请求的 future 直接被丢掉，没有跑完也要减回去。
        drop(pending);
        assert_eq!(counter.now(), 0);
    }
}
