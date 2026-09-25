//! 断网（换网、睡眠醒来）后别干等退避期满。
//!
//! 退避等待开始时先探一下 Matrix 服务器：能连上说明失败另有原因，照常退避；连不上就每隔几秒再探，
//! 网络一恢复立刻重连。只有已经在失败重连时才探，平时不发任何请求。

use std::time::Duration;

use agent_room_domain::time::DurationMillis;
use tokio::{
    sync::watch,
    time::{Instant, sleep},
};
use url::Url;

const PROBE_INTERVAL: Duration = Duration::from_secs(3);
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

pub(super) struct ConnectivityProbe {
    client: reqwest::Client,
    url: Url,
    interval: Duration,
}

impl ConnectivityProbe {
    /// 探 `<homeserver>/_matrix/client/versions`：不需要登录，服务器正常时返回 200。
    pub(super) fn new(homeserver: &str) -> Option<Self> {
        Self::with_timing(homeserver, PROBE_INTERVAL, PROBE_TIMEOUT)
    }

    /// 测试用短间隔时也给探测留足超时：机器忙时一次慢探测不该被当成断网。
    fn with_timing(homeserver: &str, interval: Duration, timeout: Duration) -> Option<Self> {
        let mut base = Url::parse(homeserver).ok()?;
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        let url = base.join("_matrix/client/versions").ok()?;
        let client = reqwest::Client::builder().timeout(timeout).build().ok()?;
        Some(Self {
            client,
            url,
            interval,
        })
    }

    async fn reachable(&self) -> bool {
        self.client
            .get(self.url.clone())
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    }

    /// 等退避期满或网络恢复，先到者为准。返回 `true` 表示收到关机。
    pub(super) async fn wait_for_retry(
        &self,
        delay: DurationMillis,
        shutdown: &mut watch::Receiver<bool>,
    ) -> bool {
        let deadline = Instant::now() + Duration::from_millis(delay.value());
        // 退避本来就很短，或服务器能连上（失败另有原因）：照常等。
        if Duration::from_millis(delay.value()) <= self.interval || self.reachable().await {
            return wait_until(deadline, shutdown).await;
        }
        loop {
            let tick = deadline
                .saturating_duration_since(Instant::now())
                .min(self.interval);
            if tick.is_zero() {
                return false;
            }
            tokio::select! {
                () = sleep(tick) => {}
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow_and_update() {
                        return true;
                    }
                }
            }
            if self.reachable().await {
                tracing::info!("网络恢复，提前重连");
                return false;
            }
        }
    }
}

async fn wait_until(deadline: Instant, shutdown: &mut watch::Receiver<bool>) -> bool {
    if *shutdown.borrow() {
        return true;
    }
    tokio::select! {
        () = tokio::time::sleep_until(deadline) => false,
        changed = shutdown.changed() => changed.is_err() || *shutdown.borrow_and_update(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use agent_room_domain::time::DurationMillis;
    use axum::{Router, extract::State, http::StatusCode, routing::get};
    use tokio::{sync::watch, time::Instant};

    use super::{ConnectivityProbe, PROBE_TIMEOUT};

    /// 模拟一个时好时坏的 Matrix 服务器：`up` 为假时返回 503。
    async fn homeserver(up: Arc<AtomicBool>) -> String {
        let app = Router::new()
            .route(
                "/_matrix/client/versions",
                get(|State(up): State<Arc<AtomicBool>>| async move {
                    if up.load(Ordering::SeqCst) {
                        StatusCode::OK
                    } else {
                        StatusCode::SERVICE_UNAVAILABLE
                    }
                }),
            )
            .with_state(up);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("可监听");
        let address = listener.local_addr().expect("有地址");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("测试服务器运行");
        });
        format!("http://{address}")
    }

    #[tokio::test]
    async fn 断网时网络一恢复就提前结束退避() {
        let up = Arc::new(AtomicBool::new(false));
        let probe = ConnectivityProbe::with_timing(
            &homeserver(up.clone()).await,
            Duration::from_millis(50),
            PROBE_TIMEOUT,
        )
        .expect("地址有效");
        let (_keep, mut shutdown) = watch::channel(false);
        let started = Instant::now();
        let recover = tokio::spawn({
            let up = up.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                up.store(true, Ordering::SeqCst);
            }
        });
        let stopped = probe
            .wait_for_retry(
                DurationMillis::new(10_000).expect("时长有效"),
                &mut shutdown,
            )
            .await;
        recover.await.expect("恢复任务完成");
        assert!(!stopped);
        let waited = started.elapsed();
        assert!(waited >= Duration::from_millis(300), "{waited:?}");
        assert!(waited < Duration::from_secs(5), "{waited:?}");
    }

    #[tokio::test]
    async fn 服务器本来就能连上时照常等满退避() {
        let up = Arc::new(AtomicBool::new(true));
        let probe = ConnectivityProbe::with_timing(
            &homeserver(up).await,
            Duration::from_millis(50),
            PROBE_TIMEOUT,
        )
        .expect("地址有效");
        let (_keep, mut shutdown) = watch::channel(false);
        let started = Instant::now();
        let stopped = probe
            .wait_for_retry(DurationMillis::new(400).expect("时长有效"), &mut shutdown)
            .await;
        assert!(!stopped);
        assert!(started.elapsed() >= Duration::from_millis(400));
    }

    #[tokio::test]
    async fn 等待中收到关机立即返回() {
        let up = Arc::new(AtomicBool::new(false));
        let probe = ConnectivityProbe::with_timing(
            &homeserver(up).await,
            Duration::from_millis(50),
            PROBE_TIMEOUT,
        )
        .expect("地址有效");
        let (stop, mut shutdown) = watch::channel(false);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            let _ = stop.send(true);
        });
        let started = Instant::now();
        assert!(
            probe
                .wait_for_retry(
                    DurationMillis::new(10_000).expect("时长有效"),
                    &mut shutdown
                )
                .await
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn 带路径前缀的服务器地址也拼到版本接口() {
        let probe = ConnectivityProbe::new("https://matrix.example.test/prefix").expect("地址有效");
        assert_eq!(
            probe.url.as_str(),
            "https://matrix.example.test/prefix/_matrix/client/versions"
        );
        let root = ConnectivityProbe::new("https://matrix.example.test").expect("地址有效");
        assert_eq!(
            root.url.as_str(),
            "https://matrix.example.test/_matrix/client/versions"
        );
    }
}
