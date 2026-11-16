//! 读 GitHub 找认领的那一行（`specs/redesign/project-rooms.md` 的“服务器怎么读”，3P-2b）。
//!
//! 控制面里专门的一个客户端，只做这一件事：
//! - 只连 `https://raw.githubusercontent.com`，主机写死；只有单元测试和无头验收（3P-9）换成本机回环地址上
//!   的假 GitHub（[`RawEndpoint::loopback`]，配置只认回环地址）。
//! - 连之前看解析出的地址，内网、回环、链路本地的一律不连（防 DNS 被改指到内网），连上以后再看对方
//!   的地址是不是刚才看过的那几个。
//! - 不跟跳转：仓库改了名、搬了家，raw 回跳转，当成没找到、说“仓库改过名”。
//! - 只读默认分支（`HEAD`）的 `AGENTS.md`、`README.md`，owner、repo 按 GitHub 的规矩验过、百分号编码以后
//!   才拼进路径。每个文件最多读 512 KB，读够就断开；整次 5 秒超时。
//! - 只找 `<站点>/r/<码>` 这一串，后面不能紧跟字母或数字；不解析 Markdown，不存文件内容。
//! - 全站每分钟最多读 60 次，满了排到下一分钟；GitHub 回 429 或 403 时全体停 10 分钟。次数记在这个进程
//!   的内存里（生产只有一个控制面）。
//!
//! 访客打开 `/gh/` 页、看徽章都不走这里：只有认领的人核对、后台核对和房主再核对一次才读。

use std::{
    fmt::Write as _,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_room_a2a_adapter::{DnsResolver, is_public_ip};
use agent_room_application::ports::{
    Clock, PortFuture, RepositoryLinkReader, RepositoryReadFailure, RepositoryScan,
};
use agent_room_domain::{
    project_rooms::{GithubRepository, RepositoryFile},
    room_listing::RoomShareCode,
    time::UtcMillis,
};
use futures_util::StreamExt as _;
use reqwest::{Client, redirect::Policy};

/// GitHub 放仓库原始文件的主机。
const RAW_HOST: &str = "raw.githubusercontent.com";
/// 每个文件最多读这么多，读够就断开。
pub(crate) const MAX_FILE_BYTES: usize = 512 * 1_024;
/// 一次核对（最多两个文件）一共等这么久。
const SCAN_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// 全站每分钟最多读几次。
pub(crate) const READS_PER_MINUTE: u32 = 60;
const MINUTE_MILLIS: i64 = 60_000;
/// GitHub 回 429、403 以后全体停多久。
pub(crate) const PAUSE_MILLIS: i64 = 10 * 60_000;

/// 读哪里。生产只有 [`RawEndpoint::github`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawEndpoint {
    scheme: &'static str,
    host: String,
    port: u16,
    /// 连之前看地址是不是公网的。只有测试里连本机的假 GitHub 时关掉。
    check_addresses: bool,
}

impl RawEndpoint {
    pub(crate) fn github() -> Self {
        Self {
            scheme: "https",
            host: RAW_HOST.to_owned(),
            port: 443,
            check_addresses: true,
        }
    }

    /// 测试和无头验收里的假 GitHub：本机回环地址、明文 HTTP、不看地址（回环地址本来就过不了那一关）。
    /// 不是回环地址就不给：只能连到本机。
    pub(crate) fn loopback(address: SocketAddr) -> Option<Self> {
        address.ip().is_loopback().then(|| Self {
            scheme: "http",
            host: match address {
                SocketAddr::V4(address) => address.ip().to_string(),
                SocketAddr::V6(address) => format!("[{}]", address.ip()),
            },
            port: address.port(),
            check_addresses: false,
        })
    }

    /// 这个仓库默认分支上这个文件的地址。名字验过只剩不用编码的字符，照样百分号编码一遍。
    fn file_url(&self, repository: &GithubRepository, file: RepositoryFile) -> String {
        let default_port = matches!((self.scheme, self.port), ("https", 443) | ("http", 80));
        let authority = if default_port {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        };
        format!(
            "{}://{authority}/{}/{}/HEAD/{}",
            self.scheme,
            percent_encode(repository.owner()),
            percent_encode(repository.name()),
            file.as_str()
        )
    }
}

/// 一个文件读成什么样。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RawResponse {
    /// 读到了，`truncated` 是读到上限断开了。
    Body {
        bytes: Vec<u8>,
        truncated: bool,
    },
    NotFound,
    /// 回了跳转（3xx）。不跟。
    Redirect,
    /// 429、403：GitHub 限了次数。
    Limited,
    /// 别的状态码。
    Failed,
}

/// 连不上，或者地址不对没连。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RawTransportFailure {
    Blocked,
    Unreachable,
}

/// 读一个地址。测试里换成假的，不出网。
pub(crate) trait RawTransport: Send + Sync {
    fn get<'a>(
        &'a self,
        url: &'a str,
        maximum_bytes: usize,
    ) -> PortFuture<'a, Result<RawResponse, RawTransportFailure>>;
}

/// 真的那一个：每次先解析、看地址，再把连接钉在看过的地址上。
pub(crate) struct PinnedRawTransport {
    endpoint: RawEndpoint,
    resolver: Arc<dyn DnsResolver>,
}

impl PinnedRawTransport {
    pub(crate) fn new(endpoint: RawEndpoint, resolver: Arc<dyn DnsResolver>) -> Self {
        Self { endpoint, resolver }
    }

    async fn get_internal(
        &self,
        url: &str,
        maximum_bytes: usize,
    ) -> Result<RawResponse, RawTransportFailure> {
        let resolved = self
            .resolver
            .resolve(&self.endpoint.host, self.endpoint.port)
            .await
            .map_err(|_| RawTransportFailure::Unreachable)?;
        if resolved.is_empty() {
            return Err(RawTransportFailure::Unreachable);
        }
        if self.endpoint.check_addresses
            && resolved.iter().any(|address| !is_public_ip(address.ip()))
        {
            return Err(RawTransportFailure::Blocked);
        }
        let client = self.client(&resolved)?;
        let response = client
            .get(url)
            .send()
            .await
            .map_err(|_| RawTransportFailure::Unreachable)?;
        if self.endpoint.check_addresses {
            let remote = response.remote_addr().ok_or(RawTransportFailure::Blocked)?;
            if !resolved.iter().any(|address| address.ip() == remote.ip()) {
                return Err(RawTransportFailure::Blocked);
            }
        }
        let status = response.status();
        if status.is_redirection() {
            return Ok(RawResponse::Redirect);
        }
        match status.as_u16() {
            404 => return Ok(RawResponse::NotFound),
            403 | 429 => return Ok(RawResponse::Limited),
            _ if !status.is_success() => return Ok(RawResponse::Failed),
            _ => {}
        }
        read_capped(response.bytes_stream(), maximum_bytes).await
    }

    fn client(&self, addresses: &[SocketAddr]) -> Result<Client, RawTransportFailure> {
        Client::builder()
            .https_only(self.endpoint.scheme == "https")
            .no_proxy()
            .redirect(Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(SCAN_TIMEOUT)
            .resolve_to_addrs(&self.endpoint.host, addresses)
            .user_agent("AgentRoom-ProjectRooms/0.1")
            .build()
            .map_err(|_| RawTransportFailure::Unreachable)
    }
}

impl RawTransport for PinnedRawTransport {
    fn get<'a>(
        &'a self,
        url: &'a str,
        maximum_bytes: usize,
    ) -> PortFuture<'a, Result<RawResponse, RawTransportFailure>> {
        Box::pin(self.get_internal(url, maximum_bytes))
    }
}

/// 读到上限就停：剩下的不读，响应一丢连接就断了。刚好读满上限也算截断：分不出后面还有没有，宁可说
/// “文件太大”。
async fn read_capped<S, B, E>(
    mut stream: S,
    maximum_bytes: usize,
) -> Result<RawResponse, RawTransportFailure>
where
    S: futures_util::Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
{
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| RawTransportFailure::Unreachable)?;
        let chunk = chunk.as_ref();
        let room = maximum_bytes.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if chunk.len() >= room {
            return Ok(RawResponse::Body {
                bytes,
                truncated: true,
            });
        }
    }
    Ok(RawResponse::Body {
        bytes,
        truncated: false,
    })
}

/// 读的次数：每分钟最多 60 次，GitHub 限了次数时全体停 10 分钟。
#[derive(Debug, Default)]
struct ReadBudget {
    minute: i64,
    used: u32,
    paused_until: Option<UtcMillis>,
}

impl ReadBudget {
    /// 现在能读就记一次；不能读回什么时候能再读。
    fn acquire(&mut self, now: UtcMillis) -> Result<(), UtcMillis> {
        if let Some(retry_at) = self.next_read_at(now) {
            return Err(retry_at);
        }
        self.used += 1;
        Ok(())
    }

    fn next_read_at(&mut self, now: UtcMillis) -> Option<UtcMillis> {
        if let Some(paused_until) = self.paused_until {
            if now < paused_until {
                return Some(paused_until);
            }
            self.paused_until = None;
        }
        let minute = now.value().div_euclid(MINUTE_MILLIS);
        if minute != self.minute {
            self.minute = minute;
            self.used = 0;
        }
        (self.used >= READS_PER_MINUTE).then(|| {
            UtcMillis::new(minute.saturating_add(1).saturating_mul(MINUTE_MILLIS)).unwrap_or(now)
        })
    }

    fn pause(&mut self, now: UtcMillis) -> UtcMillis {
        let until = UtcMillis::new(now.value().saturating_add(PAUSE_MILLIS)).unwrap_or(now);
        self.paused_until = Some(until);
        until
    }
}

/// 项目房间读 GitHub 的那一个：拼地址、数次数、找那一行。
pub(crate) struct GithubLinkReader {
    endpoint: RawEndpoint,
    transport: Arc<dyn RawTransport>,
    /// 分享链接所在的网站（`https://<裸域>`），那一行写的是 `<它>/r/<码>`。
    site_origin: Arc<str>,
    budget: Mutex<ReadBudget>,
    clock: Arc<dyn Clock>,
}

impl GithubLinkReader {
    pub(crate) fn new(
        endpoint: RawEndpoint,
        transport: Arc<dyn RawTransport>,
        site_origin: Arc<str>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            endpoint,
            transport,
            site_origin,
            budget: Mutex::new(ReadBudget::default()),
            clock,
        }
    }

    async fn scan_files(
        &self,
        repository: &GithubRepository,
        codes: &[RoomShareCode],
    ) -> Result<RepositoryScan, RepositoryReadFailure> {
        let mut scan = RepositoryScan::default();
        for file in RepositoryFile::READ_ORDER {
            if codes.iter().all(|code| scan.file_of(code).is_some()) {
                break;
            }
            let now = self.clock.now();
            if let Err(retry_at) = self.budget().acquire(now) {
                return Err(RepositoryReadFailure::Unavailable {
                    retry_at: Some(retry_at),
                });
            }
            let url = self.endpoint.file_url(repository, file);
            match self.transport.get(&url, MAX_FILE_BYTES).await {
                Ok(RawResponse::Body { bytes, truncated }) => {
                    scan.truncated |= truncated;
                    for code in codes {
                        if scan.file_of(code).is_none()
                            && contains_link(&bytes, &self.site_origin, code)
                        {
                            scan.found.push((code.clone(), file));
                        }
                    }
                }
                Ok(RawResponse::NotFound) => {}
                Ok(RawResponse::Redirect) => return Err(RepositoryReadFailure::Renamed),
                Ok(RawResponse::Limited) => {
                    let retry_at = self.budget().pause(self.clock.now());
                    tracing::warn!(
                        operation = "project_room.github.read",
                        "GitHub 限了读的次数，全体停 10 分钟再读"
                    );
                    return Err(RepositoryReadFailure::Unavailable {
                        retry_at: Some(retry_at),
                    });
                }
                Ok(RawResponse::Failed) | Err(RawTransportFailure::Unreachable) => {
                    return Err(RepositoryReadFailure::Unavailable { retry_at: None });
                }
                Err(RawTransportFailure::Blocked) => {
                    tracing::warn!(
                        operation = "project_room.github.read",
                        "GitHub 的地址解析到了不该连的地方，没有连"
                    );
                    return Err(RepositoryReadFailure::Unavailable { retry_at: None });
                }
            }
        }
        Ok(scan)
    }

    fn budget(&self) -> std::sync::MutexGuard<'_, ReadBudget> {
        self.budget
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl RepositoryLinkReader for GithubLinkReader {
    fn scan<'a>(
        &'a self,
        repository: &'a GithubRepository,
        codes: &'a [RoomShareCode],
    ) -> PortFuture<'a, Result<RepositoryScan, RepositoryReadFailure>> {
        Box::pin(async move {
            tokio::time::timeout(SCAN_TIMEOUT, self.scan_files(repository, codes))
                .await
                .unwrap_or(Err(RepositoryReadFailure::Unavailable { retry_at: None }))
        })
    }

    fn next_read_at(&self) -> Option<UtcMillis> {
        self.budget().next_read_at(self.clock.now())
    }
}

/// 文件里有没有 `<站点>/r/<码>`，后面不紧跟字母或数字。按字节找：读到上限时可能断在一个字的中间。
pub(crate) fn contains_link(body: &[u8], site_origin: &str, code: &RoomShareCode) -> bool {
    let needle = format!("{site_origin}/r/{}", code.as_str());
    let needle = needle.as_bytes();
    if needle.is_empty() || body.len() < needle.len() {
        return false;
    }
    body.windows(needle.len())
        .enumerate()
        .any(|(start, window)| {
            window == needle
                && body
                    .get(start + needle.len())
                    .is_none_or(|next| !next.is_ascii_alphanumeric())
        })
}

/// 路径里的一段：不是 RFC 3986 不用编码的字符就编码。
fn percent_encode(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(test)]
mod tests;
