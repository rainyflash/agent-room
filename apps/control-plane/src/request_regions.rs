//! 按粗地区汇总（`specs/measurement/design.md` 第 3 步）：每个地区每天有多少账户在用、每种账户的
//! 请求有多少次，前端报的快慢也按地区分。
//!
//! 来源 IP 只在这里查一下属于哪个地区，查完就丢：不写进库、日志和指标标签。见过的账户只在内存里
//! 存加盐的摘要（盐每次启动重新随机），往外只报个数，UTC 零点清空，重启丢了就从头数。

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    hash::{BuildHasher as _, RandomState},
    path::Path,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use agent_room_application::ports::{CountryLookup, NetworkAgentActivity, NoCountryLookup};
use agent_room_domain::{
    ids::{NetworkAgentId, PrincipalId},
    regions::Region,
};
use agent_room_ip_country_adapter::{CountryDatabaseError, MaxMindCountryLookup};
use axum::{
    body::Body,
    extract::{Request, State},
    http::HeaderMap,
    middleware::Next,
    response::Response,
};
use opentelemetry::metrics::ObservableGauge;

use crate::{client_address::forwarded_address, telemetry_metrics::TelemetryMetrics};

/// 一天最多记这么多个（地区、账户种类、账户）。到了就不再往里加，个数停在那里：防的是有人换着
/// 令牌刷请求把内存撑大。
const MAX_SEEN_PER_DAY: usize = 100_000;

const SECONDS_PER_DAY: u64 = 86_400;

/// 谁在用。网络 Agent 多半从聊天助手平台的机房来，和人分开算才不会把人的分布拉歪。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AccountKind {
    /// 网页或桌面端的账户登录。
    Person,
    /// 本机 Agent：Bridge 拿这台电脑的设备令牌来的，按电脑所属的账户算。
    LocalAgent,
    /// 只凭网络接入的 Agent，按网络 Agent 算。
    NetworkAgent,
}

impl AccountKind {
    pub(crate) const ALL: [Self; 3] = [Self::Person, Self::LocalAgent, Self::NetworkAgent];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::LocalAgent => "local_agent",
            Self::NetworkAgent => "network_agent",
        }
    }
}

/// 这次请求的粗地区，放在请求扩展里，给前端报快慢的接口用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RequestRegion(pub(crate) Region);

/// 当天见过的账户。
struct ActiveAccounts {
    salt: RandomState,
    today: Mutex<SeenToday>,
}

#[derive(Default)]
struct SeenToday {
    day: u64,
    seen: HashMap<(Region, AccountKind), HashSet<u64>>,
    total: usize,
}

impl ActiveAccounts {
    fn new() -> Self {
        Self {
            salt: RandomState::new(),
            today: Mutex::new(SeenToday::default()),
        }
    }

    fn see(&self, day: u64, region: Region, kind: AccountKind, account: &[u8]) {
        let digest = self.salt.hash_one(account);
        let mut today = self.today.lock().unwrap_or_else(PoisonError::into_inner);
        if today.day != day {
            *today = SeenToday {
                day,
                ..SeenToday::default()
            };
        }
        if today.total >= MAX_SEEN_PER_DAY {
            return;
        }
        if today.seen.entry((region, kind)).or_default().insert(digest) {
            today.total += 1;
        }
    }

    /// 每个地区、每种账户今天见过几个，没见过的报 0；到了第二天还没人来也报 0。
    fn counts(&self, day: u64) -> Vec<(Region, AccountKind, u64)> {
        let today = self.today.lock().unwrap_or_else(PoisonError::into_inner);
        Region::ALL
            .into_iter()
            .flat_map(|region| AccountKind::ALL.map(|kind| (region, kind)))
            .map(|(region, kind)| {
                let count = if today.day == day {
                    today
                        .seen
                        .get(&(region, kind))
                        .map_or(0, |seen| u64::try_from(seen.len()).unwrap_or(u64::MAX))
                } else {
                    0
                };
                (region, kind, count)
            })
            .collect()
    }
}

fn utc_day() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() / SECONDS_PER_DAY)
}

/// 查地区、数账户。
#[derive(Clone)]
pub(crate) struct RegionUsage {
    inner: Arc<RegionUsageInner>,
}

struct RegionUsageInner {
    lookup: Arc<dyn CountryLookup>,
    accounts: Arc<ActiveAccounts>,
    metrics: TelemetryMetrics,
    _active_accounts: ObservableGauge<u64>,
}

impl RegionUsage {
    pub(crate) fn new(lookup: Arc<dyn CountryLookup>, metrics: TelemetryMetrics) -> Self {
        let accounts = Arc::new(ActiveAccounts::new());
        let observed = accounts.clone();
        let active_accounts = TelemetryMetrics::observe_active_accounts(move |observe| {
            for (region, kind, count) in observed.counts(utc_day()) {
                observe(region.slug(), kind.label(), count);
            }
        });
        Self {
            inner: Arc::new(RegionUsageInner {
                lookup,
                accounts,
                metrics,
                _active_accounts: active_accounts,
            }),
        }
    }

    /// 没有 IP 库，地区一律算未知。
    #[cfg(test)]
    pub(crate) fn unknown() -> Self {
        Self::new(Arc::new(NoCountryLookup), TelemetryMetrics::new())
    }

    /// 不管什么地址都在这个国家。
    #[cfg(test)]
    pub(crate) fn everywhere_in(country: &'static str) -> Self {
        struct Everywhere(&'static str);

        impl CountryLookup for Everywhere {
            fn country(
                &self,
                _address: std::net::IpAddr,
            ) -> Option<agent_room_domain::regions::CountryCode> {
                agent_room_domain::regions::CountryCode::parse(self.0)
            }
        }

        Self::new(Arc::new(Everywhere(country)), TelemetryMetrics::new())
    }

    /// 今天这个地区见过几个这种账户。
    #[cfg(test)]
    pub(crate) fn seen_today(&self, region: Region, kind: AccountKind) -> u64 {
        self.inner
            .accounts
            .counts(utc_day())
            .into_iter()
            .find(|(seen_region, seen_kind, _)| *seen_region == region && *seen_kind == kind)
            .map_or(0, |(_, _, count)| count)
    }

    /// 打开镜像里的 IP 库；没有或打不开时只在启动时记一次，地区一律算未知。
    pub(crate) fn open(database: &Path, metrics: TelemetryMetrics) -> Self {
        let lookup: Arc<dyn CountryLookup> = match MaxMindCountryLookup::open(database) {
            Ok(lookup) => {
                tracing::info!(
                    build_epoch = lookup.build_epoch(),
                    "已载入 IP 库，按粗地区汇总"
                );
                Arc::new(lookup)
            }
            Err(CountryDatabaseError::Missing) => {
                tracing::warn!("没有 IP 库，地区一律算未知");
                Arc::new(NoCountryLookup)
            }
            Err(error) => {
                tracing::warn!(error = %error, "IP 库打不开，地区一律算未知");
                Arc::new(NoCountryLookup)
            }
        };
        Self::new(lookup, metrics)
    }

    fn region(&self, headers: &HeaderMap) -> Region {
        forwarded_address(headers)
            .and_then(|address| self.inner.lookup.country(address))
            .map_or(Region::Unknown, Region::of_country)
    }

    fn record(&self, region: Region, kind: AccountKind, account: &[u8]) {
        self.inner.accounts.see(utc_day(), region, kind, account);
        self.inner
            .metrics
            .record_account_request(region.slug(), kind.label());
    }
}

/// 这次请求是谁发的。认证通过的那一刻记一次，同一个请求只记第一次。
#[derive(Clone)]
pub(crate) struct RequestAccount {
    usage: RegionUsage,
    region: Region,
    noted: Arc<AtomicBool>,
}

impl RequestAccount {
    fn note(&self, kind: AccountKind, account: &[u8]) {
        if !self.noted.swap(true, Ordering::AcqRel) {
            self.usage.record(self.region, kind, account);
        }
    }
}

tokio::task_local! {
    static CURRENT: RequestAccount;
}

fn note(kind: AccountKind, account: &[u8]) {
    // 不在请求里（后台任务、测试直接调用例）时不记。
    let _ = CURRENT.try_with(|current| current.note(kind, account));
}

/// 人的登录认证通过。
pub(crate) fn note_person(principal: PrincipalId) {
    note(AccountKind::Person, principal.as_uuid().as_bytes());
}

/// Bridge 的设备请求认证通过，按电脑所属的账户算。
pub(crate) fn note_local_agent(account: PrincipalId) {
    note(AccountKind::LocalAgent, account.as_uuid().as_bytes());
}

/// 网络 Agent 凭令牌认了出来；由网络 Agent 的用例告诉我们。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct NetworkAgentRegions;

impl NetworkAgentActivity for NetworkAgentRegions {
    fn authenticated(&self, agent: NetworkAgentId) {
        note(AccountKind::NetworkAgent, agent.as_uuid().as_bytes());
    }
}

/// 远程 MCP 的工具在 rmcp 另起的任务里跑，那里没有这次请求的任务局部变量：从请求扩展里取出来
/// 再接上。
pub(crate) async fn within<F: Future>(account: Option<RequestAccount>, future: F) -> F::Output {
    match account {
        Some(account) => CURRENT.scope(account, future).await,
        None => future.await,
    }
}

pub(crate) async fn attach(
    State(usage): State<RegionUsage>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let region = usage.region(request.headers());
    let account = RequestAccount {
        usage,
        region,
        noted: Arc::default(),
    };
    request.extensions_mut().insert(RequestRegion(region));
    request.extensions_mut().insert(account.clone());
    CURRENT.scope(account, next.run(request)).await
}

#[cfg(test)]
mod tests {
    use std::{net::IpAddr, sync::Arc};

    use agent_room_application::ports::{CountryLookup, NetworkAgentActivity as _};
    use agent_room_domain::{
        ids::{NetworkAgentId, PrincipalId},
        regions::{CountryCode, Region},
    };
    use axum::{
        Extension, Router,
        body::Body,
        http::{Request, StatusCode},
        middleware,
        routing::get,
    };
    use tower::ServiceExt as _;
    use uuid::Uuid;

    use super::{
        AccountKind, ActiveAccounts, MAX_SEEN_PER_DAY, NetworkAgentRegions, RegionUsage,
        RequestAccount, RequestRegion, attach, note_person, utc_day, within,
    };
    use crate::telemetry_metrics::TelemetryMetrics;

    /// 文档地址段 203.0.113.0/24 在德国，别的都查不出来。
    struct DocumentationInGermany;

    impl CountryLookup for DocumentationInGermany {
        fn country(&self, address: IpAddr) -> Option<CountryCode> {
            match address {
                IpAddr::V4(v4) if v4.octets()[..3] == [203, 0, 113] => CountryCode::parse("DE"),
                _ => None,
            }
        }
    }

    async fn send(router: &Router, path: &str, forwarded_for: Option<&str>) -> StatusCode {
        let mut request = Request::builder().uri(path);
        if let Some(address) = forwarded_for {
            request = request.header("x-forwarded-for", address);
        }
        router
            .clone()
            .oneshot(request.body(Body::empty()).expect("请求有效"))
            .await
            .expect("测试路由不会出错")
            .status()
    }

    fn router(usage: &RegionUsage) -> Router {
        let principal = PrincipalId::from_uuid(Uuid::now_v7());
        let counted = NetworkAgentId::from_uuid(Uuid::now_v7());
        let uncounted = NetworkAgentId::from_uuid(Uuid::now_v7());
        Router::new()
            .route(
                "/person",
                get(move |Extension(RequestRegion(region))| async move {
                    note_person(principal);
                    note_person(principal);
                    if region == Region::Europe {
                        StatusCode::OK
                    } else {
                        StatusCode::ACCEPTED
                    }
                }),
            )
            .route("/anonymous", get(|| async { StatusCode::OK }))
            .route(
                "/spawned",
                get(
                    move |Extension(account): Extension<RequestAccount>| async move {
                        tokio::spawn(within(Some(account), async move {
                            NetworkAgentRegions.authenticated(counted);
                        }))
                        .await
                        .expect("任务跑完");
                        tokio::spawn(async move { NetworkAgentRegions.authenticated(uncounted) })
                            .await
                            .expect("任务跑完");
                        StatusCode::OK
                    },
                ),
            )
            .layer(middleware::from_fn_with_state(usage.clone(), attach))
    }

    fn today(usage: &RegionUsage, region: Region, kind: AccountKind) -> u64 {
        count(&usage.inner.accounts, utc_day(), region, kind)
    }

    #[tokio::test]
    async fn 认证通过才算_一个请求只算一次_地区按来源地址查() {
        let usage = RegionUsage::new(Arc::new(DocumentationInGermany), TelemetryMetrics::new());
        let router = router(&usage);

        assert_eq!(
            send(&router, "/person", Some("192.0.2.1, 203.0.113.9")).await,
            StatusCode::OK
        );
        assert_eq!(send(&router, "/person", None).await, StatusCode::ACCEPTED);
        assert_eq!(
            send(&router, "/anonymous", Some("203.0.113.10")).await,
            StatusCode::OK
        );
        assert_eq!(
            send(&router, "/spawned", Some("203.0.113.11")).await,
            StatusCode::OK
        );

        assert_eq!(today(&usage, Region::Europe, AccountKind::Person), 1);
        assert_eq!(today(&usage, Region::Unknown, AccountKind::Person), 1);
        assert_eq!(
            today(&usage, Region::Europe, AccountKind::NetworkAgent),
            1,
            "另起的任务接上了这次请求才算"
        );
        let total: u64 = usage
            .inner
            .accounts
            .counts(utc_day())
            .into_iter()
            .map(|(_, _, count)| count)
            .sum();
        assert_eq!(total, 3, "没认证的请求不算");
    }

    fn count(accounts: &ActiveAccounts, day: u64, region: Region, kind: AccountKind) -> u64 {
        accounts
            .counts(day)
            .into_iter()
            .find(|(r, k, _)| *r == region && *k == kind)
            .map(|(_, _, count)| count)
            .expect("每个地区每种账户都报")
    }

    #[test]
    fn 同一天同一个账户只算一次_人和_agent_分开算() {
        let accounts = ActiveAccounts::new();
        for _ in 0..3 {
            accounts.see(10, Region::Europe, AccountKind::Person, b"alice");
        }
        accounts.see(10, Region::Europe, AccountKind::Person, b"bob");
        accounts.see(10, Region::Europe, AccountKind::LocalAgent, b"alice");
        accounts.see(10, Region::MainlandChina, AccountKind::Person, b"alice");
        assert_eq!(count(&accounts, 10, Region::Europe, AccountKind::Person), 2);
        assert_eq!(
            count(&accounts, 10, Region::Europe, AccountKind::LocalAgent),
            1
        );
        assert_eq!(
            count(&accounts, 10, Region::MainlandChina, AccountKind::Person),
            1
        );
        assert_eq!(
            count(&accounts, 10, Region::Europe, AccountKind::NetworkAgent),
            0
        );
        assert_eq!(accounts.counts(10).len(), 33, "标签组合固定");
    }

    #[test]
    fn 过了_utc_零点从头数_没人来也报_0() {
        let accounts = ActiveAccounts::new();
        accounts.see(10, Region::Europe, AccountKind::Person, b"alice");
        assert_eq!(count(&accounts, 11, Region::Europe, AccountKind::Person), 0);
        accounts.see(11, Region::Europe, AccountKind::Person, b"bob");
        accounts.see(11, Region::Europe, AccountKind::Person, b"alice");
        assert_eq!(count(&accounts, 11, Region::Europe, AccountKind::Person), 2);
    }

    #[test]
    fn 一天记满以后个数停住() {
        let accounts = ActiveAccounts::new();
        for index in 0..=MAX_SEEN_PER_DAY {
            accounts.see(
                10,
                Region::Unknown,
                AccountKind::NetworkAgent,
                &index.to_be_bytes(),
            );
        }
        assert_eq!(
            count(&accounts, 10, Region::Unknown, AccountKind::NetworkAgent),
            u64::try_from(MAX_SEEN_PER_DAY).unwrap()
        );
    }
}
