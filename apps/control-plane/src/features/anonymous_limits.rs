//! 不要登录的发现页和分享页按来源限流（`specs/public-rooms/design.md` 的“分享链接”）：
//! `GET /public-rooms`、`GET /public-rooms/{分享码}`、`GET /r/{分享码}` 合起来每个来源每分钟
//! [`PER_MINUTE`] 次，挡住挨个猜分享码。分享页的样式和脚本不限。
//!
//! 来源和网络 Agent 的一样：IPv6 按 /64 归并，共享出口（网页版的聊天助手、MCP 目录的网关）整段算
//! 一个、名额乘上分量。只记在这个进程的内存里，键是来源经进程启动时随机取钥的哈希，不落库、不进
//! 日志；一分钟一个窗口，过了就作废。开多个控制面副本时各算各的。

use std::{
    collections::HashMap,
    hash::{BuildHasher, RandomState},
    sync::{Arc, Mutex, PoisonError},
};

use agent_room_application::ports::Clock;
use axum::http::HeaderMap;

use crate::features::network_agents::{SharedSource, source_label};

/// 每个来源每分钟的次数。翻几页目录、点开几间房、打开几个分享页，一分钟用不到这么多。
pub(crate) const PER_MINUTE: u32 = 120;
const WINDOW_MILLIS: i64 = 60_000;
/// 最多记着这么多个来源。满了先扔掉过了窗口的；还满就不记新来的来源、放它过去，免得一大片地址
/// 把内存撑大。
const MAX_TRACKED_SOURCES: usize = 100_000;

pub(crate) struct AnonymousLimiter {
    shared: Vec<SharedSource>,
    /// 每个进程随机取钥：重启以后同一个来源算出来的不一样，内存里也没有地址原文。
    keys: RandomState,
    clock: Arc<dyn Clock>,
    windows: Mutex<HashMap<u64, Window>>,
}

#[derive(Debug, Clone, Copy)]
struct Window {
    started_at: i64,
    used: u32,
}

/// 这个来源这一分钟的名额用完了，过这么多秒再来。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limited {
    pub(crate) retry_after_seconds: u64,
}

impl AnonymousLimiter {
    pub(crate) fn new(shared: Vec<SharedSource>, clock: Arc<dyn Clock>) -> Self {
        Self {
            shared,
            keys: RandomState::new(),
            clock,
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// 记上一次请求；名额用完了回 [`Limited`]，这次不算数。
    pub(crate) fn admit(&self, headers: &HeaderMap) -> Result<(), Limited> {
        let (source, weight) = source_label(&self.shared, headers);
        let key = self.keys.hash_one(source);
        let limit = PER_MINUTE.saturating_mul(weight);
        let now = self.clock.now().value();
        let mut windows = self.windows.lock().unwrap_or_else(PoisonError::into_inner);
        if !windows.contains_key(&key) && windows.len() >= MAX_TRACKED_SOURCES {
            windows.retain(|_, window| current(window, now));
            if windows.len() >= MAX_TRACKED_SOURCES {
                return Ok(());
            }
        }
        let window = windows.entry(key).or_insert(Window {
            started_at: now,
            used: 0,
        });
        if !current(window, now) {
            *window = Window {
                started_at: now,
                used: 0,
            };
        }
        if window.used >= limit {
            let remaining = window.started_at + WINDOW_MILLIS - now;
            return Err(Limited {
                retry_after_seconds: u64::try_from(remaining).unwrap_or(1).div_ceil(1_000).max(1),
            });
        }
        window.used += 1;
        Ok(())
    }
}

/// 窗口还没过去。钟往回拨了也当过去了，重新数。
fn current(window: &Window, now: i64) -> bool {
    (window.started_at..window.started_at + WINDOW_MILLIS).contains(&now)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, Ordering};

    use agent_room_domain::time::UtcMillis;
    use axum::http::HeaderValue;

    use super::*;
    use crate::features::network_agents::parse_shared_sources;

    struct ManualClock(AtomicI64);

    impl ManualClock {
        fn advance(&self, millis: i64) {
            self.0.fetch_add(millis, Ordering::SeqCst);
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> UtcMillis {
            UtcMillis::new(self.0.load(Ordering::SeqCst)).expect("测试时间有效")
        }
    }

    fn limiter(shared: &str) -> (AnonymousLimiter, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock(AtomicI64::new(1_800_000_000_000)));
        let shared = parse_shared_sources(shared).expect("共享出口写得对");
        (AnonymousLimiter::new(shared, clock.clone()), clock)
    }

    fn from(address: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_str(address).expect("地址能当请求头"),
        );
        headers
    }

    fn use_up(limiter: &AnonymousLimiter, headers: &HeaderMap, times: u32) {
        for _ in 0..times {
            limiter.admit(headers).expect("名额还没用完");
        }
    }

    #[test]
    fn 每个来源一分钟一百二十次_用完了说等到窗口过去() {
        let (limiter, clock) = limiter("");
        let headers = from("203.0.113.7");
        use_up(&limiter, &headers, PER_MINUTE);
        assert_eq!(
            limiter.admit(&headers),
            Err(Limited {
                retry_after_seconds: 60
            })
        );
        // 别的来源不受影响。
        assert_eq!(limiter.admit(&from("203.0.113.8")), Ok(()));

        clock.advance(59_500);
        assert_eq!(
            limiter.admit(&headers),
            Err(Limited {
                retry_after_seconds: 1
            })
        );
        clock.advance(500);
        assert_eq!(limiter.admit(&headers), Ok(()));
    }

    #[test]
    fn 同一个六十四位网段里换地址绕不过() {
        let (limiter, _) = limiter("");
        use_up(&limiter, &from("2001:db8:1:2::1"), PER_MINUTE);
        assert!(limiter.admit(&from("2001:db8:1:2:ffff::9")).is_err());
        assert_eq!(limiter.admit(&from("2001:db8:1:3::1")), Ok(()));
    }

    #[test]
    fn 共享出口整段算一个来源_名额乘上分量() {
        let (limiter, _) = limiter("assistant@3:198.51.100.0/24");
        for index in 0..PER_MINUTE * 3 {
            let address = format!("198.51.100.{}", index % 200);
            assert_eq!(limiter.admit(&from(&address)), Ok(()), "第 {index} 次");
        }
        assert!(limiter.admit(&from("198.51.100.250")).is_err());
        assert_eq!(limiter.admit(&from("198.51.101.1")), Ok(()));
    }

    #[test]
    fn 钟往回拨了就重新数() {
        let (limiter, clock) = limiter("");
        let headers = from("203.0.113.7");
        use_up(&limiter, &headers, PER_MINUTE);
        clock.advance(-1_000);
        assert_eq!(limiter.admit(&headers), Ok(()));
    }
}
