//! 这台电脑的授权跟着账户登录走（`specs/desktop-sign-in/design.md`）。
//!
//! 托管的 Bridge 还没授权时，原生层用桌面登录向控制面要一份一次性的电脑授权，经标准输入交给它，
//! 不用再去浏览器批准设备码。授权只在原生层和 Bridge 之间走：网页拿不到，代发通道也不转发这个地址
//! （`is_native_only_route`）。

use std::time::Duration;

use reqwest::{Client, StatusCode, header};
use serde::Deserialize;
use url::Url;

use crate::control_plane_proxy::{DESKTOP_SESSION_COOKIE, directory_url};

pub(crate) const COMPUTER_GRANT_ROUTE: &str = "auth/desktop/computer-grant";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_GRANT_LENGTH: usize = 512;
/// 控制面说带去的登录过期了或被撤销了。
const INVALID_SESSION_CODE: &str = "authentication.invalid_session";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComputerGrantFailure {
    /// 网络不通、控制面暂时不可用：过一会儿再要。
    Transient,
    /// 控制面说带去的登录无效：删掉本机这份，等人重新登录。
    SessionRejected,
    /// 别的拒绝（比如旧版控制面没有这个地址）：改走设备码。
    Refused,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComputerGrantResponse {
    grant: String,
}

#[derive(Deserialize)]
struct ErrorCode {
    code: String,
}

#[derive(Clone)]
pub(crate) struct ComputerGrantClient {
    endpoint: Url,
    http: Client,
}

impl ComputerGrantClient {
    pub(crate) fn new(control_plane_url: Url) -> Option<Self> {
        let endpoint = directory_url(control_plane_url)
            .join(COMPUTER_GRANT_ROUTE)
            .ok()?;
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .ok()?;
        Some(Self { endpoint, http })
    }

    /// 带上桌面登录和窗口的 Origin 向控制面要一份电脑授权。
    pub(crate) async fn request(
        &self,
        origin: &str,
        session_secret: &str,
    ) -> Result<ComputerGrant, ComputerGrantFailure> {
        let mut cookie =
            header::HeaderValue::from_str(&format!("{DESKTOP_SESSION_COOKIE}={session_secret}"))
                .map_err(|_| ComputerGrantFailure::Refused)?;
        cookie.set_sensitive(true);
        let response = self
            .http
            .post(self.endpoint.clone())
            .header(header::COOKIE, cookie)
            .header(header::ORIGIN, origin)
            .send()
            .await
            .map_err(|_| ComputerGrantFailure::Transient)?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|_| ComputerGrantFailure::Transient)?;
        if status.is_success() {
            return serde_json::from_slice::<ComputerGrantResponse>(&body)
                .ok()
                .map(|response| response.grant)
                .filter(|grant| valid_grant(grant))
                .map(ComputerGrant)
                .ok_or(ComputerGrantFailure::Refused);
        }
        Err(classify_failure(status, &body))
    }
}

fn classify_failure(status: StatusCode, body: &[u8]) -> ComputerGrantFailure {
    if status == StatusCode::UNAUTHORIZED
        && serde_json::from_slice::<ErrorCode>(body)
            .is_ok_and(|error| error.code == INVALID_SESSION_CODE)
    {
        return ComputerGrantFailure::SessionRejected;
    }
    if status.is_server_error()
        || status == StatusCode::TOO_MANY_REQUESTS
        || status == StatusCode::REQUEST_TIMEOUT
    {
        return ComputerGrantFailure::Transient;
    }
    ComputerGrantFailure::Refused
}

/// 要到的电脑授权。调试输出里不出现它。
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ComputerGrant(String);

impl ComputerGrant {
    /// 交给 Bridge 的那一行（Bridge `supervisor_input.rs` 的 `device-grant <授权>`）。
    pub(crate) fn bridge_line(&self) -> String {
        format!(
            "device-grant {}
",
            self.0
        )
    }
}

impl std::fmt::Debug for ComputerGrant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ComputerGrant([已脱敏])")
    }
}

/// 请 Bridge 改走设备码的那一行。
pub(crate) const DEVICE_CODE_LINE: &str = "device-code
";
const FIRST_RETRY_DELAY: Duration = Duration::from_secs(2);
const MAX_RETRY_DELAY: Duration = Duration::from_mins(1);
/// Bridge 拿授权注册没成、过一会儿换一份再试就行的错误码；别的改走设备码。
const BRIDGE_RETRYABLE_CODES: [&str; 2] = [
    "bridge.control_plane_unavailable",
    "bridge.registration_outcome_unknown",
];

/// 托管的 Bridge 等电脑授权时，原生层接下来做什么。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum GrantStep {
    /// 什么也不做：在等登录、等回答，或者 Bridge 已经不等了。
    Wait,
    /// 把授权写给 Bridge。
    Deliver(ComputerGrant),
    /// 过一会儿再要一份。
    RetryAfter(Duration),
    /// 控制面说登录无效：删掉本机这份，等人重新登录。
    ForgetSession,
    /// 请 Bridge 改走设备码。
    DeviceCode,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum GrantPhase {
    #[default]
    Idle,
    /// 有一次请求正在路上；`signed_in` 表示请求发出后应用里又刚登录好。
    InFlight { signed_in: bool },
    /// 正在等定好的重试到点。
    WaitingRetry,
}

/// 一代托管 Bridge 等电脑授权的进度。换一代子进程就重来。
#[derive(Debug, Default)]
pub(crate) struct GrantWait {
    /// Bridge 报了 `device_grant_required`，还没拿到能用的授权，也没改走设备码。
    wanted: bool,
    phase: GrantPhase,
    failures: u32,
}

impl GrantWait {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn bridge_asked(&mut self) {
        self.wanted = true;
        self.failures = 0;
    }

    /// 现在该不该去要一份：Bridge 在等，手上没有正在路上的请求，也没有在等重试到点。
    pub(crate) const fn should_request(&self) -> bool {
        self.wanted && matches!(self.phase, GrantPhase::Idle)
    }

    pub(crate) const fn requesting(&mut self) {
        self.phase = GrantPhase::InFlight { signed_in: false };
    }

    /// 应用里刚登录好：打断退避等待，如果旧请求还在路上就记下来，等它回来立刻重试。
    pub(crate) const fn signed_in(&mut self) {
        self.failures = 0;
        self.phase = match self.phase {
            GrantPhase::InFlight { .. } => GrantPhase::InFlight { signed_in: true },
            GrantPhase::Idle | GrantPhase::WaitingRetry => GrantPhase::Idle,
        };
    }

    pub(crate) const fn retry_due(&mut self) {
        if matches!(self.phase, GrantPhase::WaitingRetry) {
            self.phase = GrantPhase::Idle;
        }
    }

    pub(crate) const fn bridge_authorized(&mut self) {
        self.wanted = false;
        self.phase = GrantPhase::Idle;
    }

    pub(crate) fn resolved(
        &mut self,
        result: Result<ComputerGrant, ComputerGrantFailure>,
    ) -> GrantStep {
        let signed_in = matches!(self.phase, GrantPhase::InFlight { signed_in: true });
        self.phase = GrantPhase::Idle;
        if !self.wanted {
            return GrantStep::Wait;
        }
        match result {
            Ok(grant) => {
                self.wanted = false;
                GrantStep::Deliver(grant)
            }
            Err(ComputerGrantFailure::Transient) => {
                if signed_in {
                    self.phase = GrantPhase::WaitingRetry;
                    GrantStep::RetryAfter(Duration::ZERO)
                } else {
                    self.retry_later()
                }
            }
            // 请求在路上时人又登录了一次：被拒的是之前那份登录，刚登录的那份马上试。
            // 没有的话只删本机这份、等人登录，别拿同一份登录接着要（删没成时会一直空转）。
            Err(ComputerGrantFailure::SessionRejected) if signed_in => {
                self.phase = GrantPhase::WaitingRetry;
                GrantStep::RetryAfter(Duration::ZERO)
            }
            Err(ComputerGrantFailure::SessionRejected) => GrantStep::ForgetSession,
            Err(ComputerGrantFailure::Refused) => {
                self.wanted = false;
                GrantStep::DeviceCode
            }
        }
    }

    /// Bridge 拿授权注册没成（`device_grant_failed`）。
    pub(crate) fn bridge_rejected(&mut self, code: &str) -> GrantStep {
        if BRIDGE_RETRYABLE_CODES.contains(&code) {
            self.wanted = true;
            return self.retry_later();
        }
        self.wanted = false;
        GrantStep::DeviceCode
    }

    fn retry_later(&mut self) -> GrantStep {
        self.phase = GrantPhase::WaitingRetry;
        self.failures = self.failures.saturating_add(1);
        let delay = FIRST_RETRY_DELAY.saturating_mul(1 << self.failures.saturating_sub(1).min(6));
        GrantStep::RetryAfter(delay.min(MAX_RETRY_DELAY))
    }
}

/// 授权整行写进 Bridge 的标准输入：不许有空白和控制字符。
fn valid_grant(grant: &str) -> bool {
    !grant.is_empty()
        && grant.len() <= MAX_GRANT_LENGTH
        && !grant
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
}

/// 代发通道不转发的地址：电脑授权只给原生层自己要。比较前去掉查询、末尾的斜杠和重复的斜杠，
/// 解开百分号编码，不分大小写，免得换个写法就绕过去。
pub(crate) fn is_native_only_route(path: &str) -> bool {
    let route = path.split('?').next().unwrap_or_default();
    let decoded = percent_decoded(route).to_ascii_lowercase();
    let normalized = decoded
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    normalized == COMPUTER_GRANT_ROUTE
}

fn percent_decoded(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(byte) = bytes
                .get(index + 1..index + 3)
                .and_then(|hex| std::str::from_utf8(hex).ok())
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            decoded.push(byte);
            index += 3;
            continue;
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

#[cfg(test)]
mod tests {
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpListener,
        task::JoinHandle,
    };

    use super::*;

    /// 只接一个连接的 HTTP 服务：把收到的原始请求交回，再回一段写好的回答。
    async fn serve_once(response: String) -> (Url, JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("可绑定回环端口");
        let address = listener.local_addr().expect("可读本机地址");
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("收到连接");
            let mut received = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = stream.read(&mut buffer).await.expect("可读请求");
                received.extend_from_slice(&buffer[..read]);
                if read == 0 || received.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            stream
                .write_all(response.as_bytes())
                .await
                .expect("可写回答");
            stream.shutdown().await.expect("可关闭连接");
            String::from_utf8(received).expect("请求是 UTF-8")
        });
        (
            Url::parse(&format!("http://{address}/api")).expect("本机根地址有效"),
            handle,
        )
    }

    fn reply(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    #[tokio::test]
    async fn 带上桌面登录和窗口_origin_要到电脑授权() {
        let (base, received) = serve_once(reply(
            "200 OK",
            r#"{"grant":"computer-grant","expiresAtUnixMs":1}"#,
        ))
        .await;
        let client = ComputerGrantClient::new(base).expect("可建出客户端");

        let grant = client
            .request("tauri://localhost", "session-secret")
            .await
            .expect("要到授权");

        assert_eq!(
            grant.bridge_line(),
            "device-grant computer-grant
"
        );
        assert!(!format!("{grant:?}").contains("computer-grant"));
        let request = received.await.expect("服务收到请求").to_ascii_lowercase();
        assert!(
            request.starts_with("post /api/auth/desktop/computer-grant "),
            "{request}"
        );
        assert!(request.contains(&format!(
            "cookie: {}=session-secret",
            DESKTOP_SESSION_COOKIE.to_ascii_lowercase()
        )));
        assert!(request.contains("origin: tauri://localhost"));
    }

    #[tokio::test]
    async fn 回答按能不能再试分开() {
        for (status, body, expected) in [
            (
                "401 Unauthorized",
                r#"{"code":"authentication.invalid_session"}"#,
                ComputerGrantFailure::SessionRejected,
            ),
            (
                "503 Service Unavailable",
                r#"{"code":"device.dependency_unavailable"}"#,
                ComputerGrantFailure::Transient,
            ),
            (
                "429 Too Many Requests",
                "{}",
                ComputerGrantFailure::Transient,
            ),
            ("404 Not Found", "{}", ComputerGrantFailure::Refused),
            (
                "403 Forbidden",
                r#"{"code":"authentication.desktop_session_required"}"#,
                ComputerGrantFailure::Refused,
            ),
            (
                "200 OK",
                r#"{"grant":"has space"}"#,
                ComputerGrantFailure::Refused,
            ),
        ] {
            let (base, _) = serve_once(reply(status, body)).await;
            let failure = ComputerGrantClient::new(base)
                .expect("可建出客户端")
                .request("tauri://localhost", "session-secret")
                .await
                .expect_err("要不到授权");
            assert_eq!(failure, expected, "{status} {body}");
        }
    }

    #[tokio::test]
    async fn 控制面连不上时过一会儿再要() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("可绑定回环端口");
        let address = listener.local_addr().expect("可读本机地址");
        drop(listener);
        let failure =
            ComputerGrantClient::new(Url::parse(&format!("http://{address}/")).expect("地址有效"))
                .expect("可建出客户端")
                .request("tauri://localhost", "session-secret")
                .await
                .expect_err("端口没人听");
        assert_eq!(failure, ComputerGrantFailure::Transient);
    }

    fn grant() -> ComputerGrant {
        ComputerGrant("computer-grant".to_owned())
    }

    #[test]
    fn 要到授权就交给_bridge_只交一次() {
        let mut wait = GrantWait::default();
        assert!(!wait.should_request(), "Bridge 还没要");
        wait.bridge_asked();
        assert!(wait.should_request());
        wait.requesting();
        assert!(!wait.should_request(), "一次只要一份");

        assert_eq!(wait.resolved(Ok(grant())), GrantStep::Deliver(grant()));
        assert!(!wait.should_request());
        assert_eq!(
            wait.resolved(Ok(grant())),
            GrantStep::Wait,
            "交过了就不再交"
        );
    }

    #[test]
    fn 连不上就退避再要_登录无效就等重新登录_被拒就改走设备码() {
        let mut wait = GrantWait::default();
        wait.bridge_asked();
        wait.requesting();
        let delays: Vec<_> = (0..8)
            .map(|_| {
                let step = wait.resolved(Err(ComputerGrantFailure::Transient));
                assert!(!wait.should_request(), "定好的重试还没到");
                wait.retry_due();
                assert!(wait.should_request());
                wait.requesting();
                step
            })
            .collect();
        assert_eq!(
            delays,
            [2, 4, 8, 16, 32, 60, 60, 60]
                .map(|seconds| GrantStep::RetryAfter(Duration::from_secs(seconds)))
        );

        assert_eq!(
            wait.resolved(Err(ComputerGrantFailure::SessionRejected)),
            GrantStep::ForgetSession
        );
        assert!(wait.should_request(), "重新登录以后接着要");
        wait.requesting();
        assert_eq!(
            wait.resolved(Err(ComputerGrantFailure::Refused)),
            GrantStep::DeviceCode
        );
        assert!(!wait.should_request());
    }

    #[test]
    fn bridge_注册没成时_暂时的换一份再试_别的改走设备码() {
        let mut wait = GrantWait::default();
        wait.bridge_asked();
        wait.requesting();
        assert_eq!(wait.resolved(Ok(grant())), GrantStep::Deliver(grant()));
        assert_eq!(
            wait.bridge_rejected("bridge.control_plane_unavailable"),
            GrantStep::RetryAfter(Duration::from_secs(2))
        );
        wait.retry_due();
        assert!(wait.should_request());
        wait.requesting();
        assert_eq!(wait.resolved(Ok(grant())), GrantStep::Deliver(grant()));
        assert_eq!(
            wait.bridge_rejected("bridge.identity_assertion_invalid"),
            GrantStep::DeviceCode
        );
        assert!(!wait.should_request());
    }

    #[test]
    fn 退避期间或请求在路上时重新登录会立刻重试() {
        let mut wait = GrantWait::default();
        wait.bridge_asked();
        wait.requesting();
        assert_eq!(
            wait.resolved(Err(ComputerGrantFailure::Transient)),
            GrantStep::RetryAfter(Duration::from_secs(2))
        );
        assert!(!wait.should_request());

        wait.signed_in();
        assert!(wait.should_request(), "退避期间登录好应立即允许请求");
        wait.requesting();
        wait.retry_due();
        assert!(
            !wait.should_request(),
            "旧的退避定时器到点不能打断正在路上的新请求"
        );

        wait.signed_in();
        assert_eq!(
            wait.resolved(Err(ComputerGrantFailure::Transient)),
            GrantStep::RetryAfter(Duration::ZERO),
            "请求在路上时登录好，旧请求暂时失败后应立即重试"
        );

        wait.retry_due();
        wait.requesting();
        wait.signed_in();
        assert_eq!(
            wait.resolved(Err(ComputerGrantFailure::SessionRejected)),
            GrantStep::RetryAfter(Duration::ZERO),
            "被拒的是登录之前那份，刚登录的那份马上试"
        );
        wait.retry_due();
        wait.requesting();
        assert_eq!(
            wait.resolved(Err(ComputerGrantFailure::SessionRejected)),
            GrantStep::ForgetSession,
            "没有新登录就等人登录，不接着要"
        );
    }

    #[test]
    fn 代发通道不转发电脑授权_换个写法也不行() {
        for blocked in [
            "auth/desktop/computer-grant",
            "auth/desktop/computer-grant/",
            "auth//desktop/computer-grant",
            "AUTH/Desktop/Computer-Grant",
            "auth/desktop/computer%2Dgrant",
            "auth/desktop/computer%2dgrant?x=1",
            "auth%2Fdesktop%2Fcomputer-grant",
        ] {
            assert!(is_native_only_route(blocked), "应拦下 {blocked:?}");
        }
        for allowed in [
            "auth/desktop/matrix-session",
            "auth/session",
            "auth/devices",
            "auth/desktop/computer-grants",
        ] {
            assert!(!is_native_only_route(allowed), "不该拦 {allowed:?}");
        }
    }
}
