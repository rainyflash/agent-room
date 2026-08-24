use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use agent_room_application::ports::PortFuture;
use agent_room_domain::{agent_cards::AgentCardSourceUrl, time::DurationMillis};
use futures_util::StreamExt as _;
use reqwest::{
    Client,
    header::{ACCEPT, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE},
    redirect::Policy,
};
use thiserror::Error;

const DEFAULT_CACHE_LIFETIME_MILLIS: u64 = 300_000;
const MINIMUM_CACHE_LIFETIME_MILLIS: u64 = 1_000;
const MAXIMUM_CACHE_LIFETIME_MILLIS: u64 = 3_600_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkTargetFailureKind {
    InvalidTarget,
    ResolutionFailed,
    BlockedAddress,
    ConnectFailed,
    InvalidResponse,
    ResponseTooLarge,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("安全 HTTPS 文档操作 {operation} 失败：{kind:?}")]
pub struct NetworkTargetFailure {
    operation: &'static str,
    kind: NetworkTargetFailureKind,
}

impl NetworkTargetFailure {
    const fn new(operation: &'static str, kind: NetworkTargetFailureKind) -> Self {
        Self { operation, kind }
    }

    pub const fn operation(self) -> &'static str {
        self.operation
    }

    pub const fn kind(self) -> NetworkTargetFailureKind {
        self.kind
    }
}

pub type NetworkTargetResult<T> = Result<T, NetworkTargetFailure>;

pub trait DnsResolver: Send + Sync {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> PortFuture<'a, NetworkTargetResult<Vec<SocketAddr>>>;
}

#[derive(Debug, Clone, Copy)]
pub struct SystemDnsResolver;

impl DnsResolver for SystemDnsResolver {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> PortFuture<'a, NetworkTargetResult<Vec<SocketAddr>>> {
        Box::pin(async move {
            tokio::net::lookup_host((host, port))
                .await
                .map(Iterator::collect)
                .map_err(|_| {
                    NetworkTargetFailure::new(
                        "a2a.http.resolve",
                        NetworkTargetFailureKind::ResolutionFailed,
                    )
                })
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonDocument {
    body: Vec<u8>,
    cache_lifetime: DurationMillis,
}

impl JsonDocument {
    pub fn new(body: Vec<u8>, cache_lifetime: DurationMillis) -> Self {
        Self {
            body,
            cache_lifetime,
        }
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub const fn cache_lifetime(&self) -> DurationMillis {
        self.cache_lifetime
    }
}

pub trait HttpsDocumentClient: Send + Sync {
    fn get_json<'a>(
        &'a self,
        source_url: &'a AgentCardSourceUrl,
        maximum_bytes: usize,
    ) -> PortFuture<'a, NetworkTargetResult<JsonDocument>>;
}

#[derive(Debug, Clone, Copy)]
pub struct PinnedHttpsClientConfiguration {
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
}

impl Default for PinnedHttpsClientConfiguration {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(3),
            request_timeout: Duration::from_secs(8),
        }
    }
}

pub struct PinnedHttpsClient {
    resolver: Arc<dyn DnsResolver>,
    configuration: PinnedHttpsClientConfiguration,
}

impl PinnedHttpsClient {
    pub fn new(
        resolver: Arc<dyn DnsResolver>,
        configuration: PinnedHttpsClientConfiguration,
    ) -> Self {
        Self {
            resolver,
            configuration,
        }
    }

    async fn get_json_internal(
        &self,
        source_url: &AgentCardSourceUrl,
        maximum_bytes: usize,
    ) -> NetworkTargetResult<JsonDocument> {
        let operation = "a2a.http.get_json";
        if maximum_bytes == 0 {
            return Err(NetworkTargetFailure::new(
                operation,
                NetworkTargetFailureKind::Internal,
            ));
        }
        let parsed = url::Url::parse(source_url.as_str()).map_err(|_| {
            NetworkTargetFailure::new(operation, NetworkTargetFailureKind::InvalidTarget)
        })?;
        let host = parsed.host_str().ok_or_else(|| {
            NetworkTargetFailure::new(operation, NetworkTargetFailureKind::InvalidTarget)
        })?;
        let port = parsed.port_or_known_default().ok_or_else(|| {
            NetworkTargetFailure::new(operation, NetworkTargetFailureKind::InvalidTarget)
        })?;
        let resolved = self.resolver.resolve(host, port).await?;
        let approved = validate_resolved_addresses(&resolved)?;
        let client = build_pinned_client(host, &approved, self.configuration)?;
        let response = client
            .get(parsed)
            .header(ACCEPT, "application/a2a+json, application/json")
            .send()
            .await
            .map_err(|_| {
                NetworkTargetFailure::new(operation, NetworkTargetFailureKind::ConnectFailed)
            })?;
        let status = response.status();
        if !status.is_success() {
            let kind = if status.is_server_error() {
                NetworkTargetFailureKind::ConnectFailed
            } else {
                NetworkTargetFailureKind::InvalidResponse
            };
            return Err(NetworkTargetFailure::new(operation, kind));
        }
        let remote = response.remote_addr().ok_or_else(|| {
            NetworkTargetFailure::new(operation, NetworkTargetFailureKind::Internal)
        })?;
        if !approved.iter().any(|address| address.ip() == remote.ip()) {
            return Err(NetworkTargetFailure::new(
                operation,
                NetworkTargetFailureKind::BlockedAddress,
            ));
        }
        validate_content_type(response.headers().get(CONTENT_TYPE))?;
        validate_content_length(response.headers().get(CONTENT_LENGTH), maximum_bytes)?;
        let cache_lifetime = cache_lifetime(response.headers().get(CACHE_CONTROL));
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| {
                NetworkTargetFailure::new(operation, NetworkTargetFailureKind::ConnectFailed)
            })?;
            let next_length = body.len().checked_add(chunk.len()).ok_or_else(|| {
                NetworkTargetFailure::new(operation, NetworkTargetFailureKind::ResponseTooLarge)
            })?;
            if next_length > maximum_bytes {
                return Err(NetworkTargetFailure::new(
                    operation,
                    NetworkTargetFailureKind::ResponseTooLarge,
                ));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(JsonDocument::new(body, cache_lifetime))
    }
}

impl HttpsDocumentClient for PinnedHttpsClient {
    fn get_json<'a>(
        &'a self,
        source_url: &'a AgentCardSourceUrl,
        maximum_bytes: usize,
    ) -> PortFuture<'a, NetworkTargetResult<JsonDocument>> {
        Box::pin(async move { self.get_json_internal(source_url, maximum_bytes).await })
    }
}

fn build_pinned_client(
    host: &str,
    addresses: &[SocketAddr],
    configuration: PinnedHttpsClientConfiguration,
) -> NetworkTargetResult<Client> {
    Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(Policy::none())
        .connect_timeout(configuration.connect_timeout)
        .timeout(configuration.request_timeout)
        .resolve_to_addrs(host, addresses)
        .user_agent("AgentRoom-A2A/0.1")
        .build()
        .map_err(|_| {
            NetworkTargetFailure::new("a2a.http.build_client", NetworkTargetFailureKind::Internal)
        })
}

fn validate_resolved_addresses(addresses: &[SocketAddr]) -> NetworkTargetResult<Vec<SocketAddr>> {
    if addresses.is_empty() {
        return Err(NetworkTargetFailure::new(
            "a2a.http.validate_addresses",
            NetworkTargetFailureKind::ResolutionFailed,
        ));
    }
    let unique = addresses.iter().copied().collect::<BTreeSet<_>>();
    if unique.iter().any(|address| !is_public_ip(address.ip())) {
        return Err(NetworkTargetFailure::new(
            "a2a.http.validate_addresses",
            NetworkTargetFailureKind::BlockedAddress,
        ));
    }
    Ok(unique.into_iter().collect())
}

fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_ipv4(address),
        IpAddr::V6(address) => is_public_ipv6(address),
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let [a, b, c, d] = address.octets();
    !(a == 0
        || a == 10
        || a == 127
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || (a == 255 && b == 255 && c == 255 && d == 255))
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_public_ipv4(mapped);
    }
    let segments = address.segments();
    let is_global_unicast = (segments[0] & 0xe000) == 0x2000;
    let is_documentation = segments[0] == 0x2001 && segments[1] == 0x0db8;
    is_global_unicast && !is_documentation
}

fn validate_content_type(value: Option<&reqwest::header::HeaderValue>) -> NetworkTargetResult<()> {
    let media_type = value
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .map(str::to_ascii_lowercase);
    if matches!(
        media_type.as_deref(),
        Some("application/json" | "application/a2a+json")
    ) {
        Ok(())
    } else {
        Err(NetworkTargetFailure::new(
            "a2a.http.validate_content_type",
            NetworkTargetFailureKind::InvalidResponse,
        ))
    }
}

fn validate_content_length(
    value: Option<&reqwest::header::HeaderValue>,
    maximum_bytes: usize,
) -> NetworkTargetResult<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let length = value
        .to_str()
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| {
            NetworkTargetFailure::new(
                "a2a.http.validate_content_length",
                NetworkTargetFailureKind::InvalidResponse,
            )
        })?;
    let maximum_bytes = u64::try_from(maximum_bytes).map_err(|_| {
        NetworkTargetFailure::new(
            "a2a.http.validate_content_length",
            NetworkTargetFailureKind::Internal,
        )
    })?;
    if length > maximum_bytes {
        Err(NetworkTargetFailure::new(
            "a2a.http.validate_content_length",
            NetworkTargetFailureKind::ResponseTooLarge,
        ))
    } else {
        Ok(())
    }
}

fn cache_lifetime(value: Option<&reqwest::header::HeaderValue>) -> DurationMillis {
    let milliseconds = value
        .and_then(|value| value.to_str().ok())
        .and_then(parse_max_age_seconds)
        .and_then(|seconds| seconds.checked_mul(1_000))
        .unwrap_or(DEFAULT_CACHE_LIFETIME_MILLIS)
        .clamp(MINIMUM_CACHE_LIFETIME_MILLIS, MAXIMUM_CACHE_LIFETIME_MILLIS);
    DurationMillis::new(milliseconds).expect("缓存时限常量必须大于零")
}

fn parse_max_age_seconds(value: &str) -> Option<u64> {
    value.split(',').map(str::trim).find_map(|directive| {
        let (name, value) = directive.split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("max-age")
            .then(|| value.trim().trim_matches('"').parse().ok())
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use std::{
        net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
        sync::Arc,
    };

    use agent_room_application::ports::PortFuture;
    use agent_room_domain::agent_cards::AgentCardSourceUrl;
    use reqwest::header::HeaderValue;

    use super::{
        DnsResolver, HttpsDocumentClient, NetworkTargetFailureKind, NetworkTargetResult,
        PinnedHttpsClient, PinnedHttpsClientConfiguration, cache_lifetime, is_public_ip,
        validate_content_length, validate_content_type, validate_resolved_addresses,
    };

    struct FixedResolver {
        addresses: Vec<SocketAddr>,
    }

    impl DnsResolver for FixedResolver {
        fn resolve<'a>(
            &'a self,
            host: &'a str,
            port: u16,
        ) -> PortFuture<'a, NetworkTargetResult<Vec<SocketAddr>>> {
            assert_eq!(host, "agent.example");
            assert_eq!(port, 443);
            let addresses = self.addresses.clone();
            Box::pin(async move { Ok(addresses) })
        }
    }

    #[test]
    fn 只接受公网地址并拒绝混合_dns_答案() {
        assert!(is_public_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(is_public_ip(IpAddr::V6(
            "2606:4700:4700::1111".parse().expect("测试 IPv6 有效")
        )));
        for blocked in [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            IpAddr::V6("fc00::1".parse().expect("测试 IPv6 有效")),
            IpAddr::V6("2001:db8::1".parse().expect("测试 IPv6 有效")),
        ] {
            assert!(!is_public_ip(blocked), "地址 {blocked} 必须被拒绝");
        }

        let mixed = [
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 443),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 443),
        ];
        let failure = validate_resolved_addresses(&mixed).expect_err("混合 DNS 答案必须整体拒绝");
        assert_eq!(failure.kind(), NetworkTargetFailureKind::BlockedAddress);
    }

    #[tokio::test]
    async fn dns_解析到元数据地址时在建立_http_连接前拒绝() {
        let client = PinnedHttpsClient::new(
            Arc::new(FixedResolver {
                addresses: vec![SocketAddr::new(
                    IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
                    443,
                )],
            }),
            PinnedHttpsClientConfiguration::default(),
        );
        let source =
            AgentCardSourceUrl::new("https://agent.example/.well-known/agent-card.json".to_owned())
                .expect("测试来源有效");

        let failure = client
            .get_json(&source, 65_536)
            .await
            .expect_err("云元数据地址必须在连接前拒绝");

        assert_eq!(failure.kind(), NetworkTargetFailureKind::BlockedAddress);
    }

    #[test]
    fn json_媒体类型和正文上限严格执行() {
        assert!(
            validate_content_type(Some(&HeaderValue::from_static(
                "application/a2a+json; charset=utf-8"
            )))
            .is_ok()
        );
        assert!(validate_content_type(Some(&HeaderValue::from_static("text/html"))).is_err());
        let failure = validate_content_length(Some(&HeaderValue::from_static("65537")), 65_536)
            .expect_err("超长正文必须拒绝");
        assert_eq!(failure.kind(), NetworkTargetFailureKind::ResponseTooLarge);
    }

    #[test]
    fn 缓存时限使用服务端_max_age_且受本地上限约束() {
        assert_eq!(
            cache_lifetime(Some(&HeaderValue::from_static("public, max-age=120"))).value(),
            120_000
        );
        assert_eq!(
            cache_lifetime(Some(&HeaderValue::from_static("max-age=0"))).value(),
            1_000
        );
        assert_eq!(
            cache_lifetime(Some(&HeaderValue::from_static("max-age=999999"))).value(),
            3_600_000
        );
    }
}
