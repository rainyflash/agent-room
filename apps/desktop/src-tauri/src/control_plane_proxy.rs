//! 桌面端发往控制面的请求由原生层代发。
//!
//! macOS 的 `WKWebView` 默认挡跨站 Cookie：页面在 `tauri://localhost`，控制面在另一个站点，
//! 注入 `WebView` 的登录 Cookie 不会随请求带上，浏览器里登录完，界面回来还是“没登录”。
//! 所以 `WebView` 不再直接请求控制面：前端把请求整个交给 `desktop_control_plane_request`，
//! 这里带上系统凭据库里的登录和窗口自己的 Origin 发出去，再把回答原样交回。登录 Secret
//! 一直留在原生层，前端拿不到。
//!
//! 请求和回答都是一段字节：开头 4 字节大端长度，接着这么长的 JSON 头，剩下的是正文。

use std::{borrow::Cow, time::Duration};

use reqwest::{
    Client, Method,
    header::{self, HeaderMap, HeaderName, HeaderValue},
    redirect,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

/// 前端每个请求自己有更短的超时；这里只防原生请求一直挂着。
const REQUEST_TIMEOUT: Duration = Duration::from_mins(2);
const MAX_HEAD_BYTES: usize = 64 * 1024;
const MAX_REQUEST_HEADERS: usize = 64;
pub(crate) const DESKTOP_SESSION_COOKIE: &str = "__Secure-agent-room-desktop-session";

/// 这些请求头由这里决定，前端交来的一律不转：登录和 Origin 由原生层带，其余是连接层的事。
const REQUEST_HEADERS_SET_HERE: &[&str] = &[
    "accept-encoding",
    "authorization",
    "connection",
    "content-length",
    "cookie",
    "cookie2",
    "expect",
    "host",
    "keep-alive",
    "origin",
    "proxy-authorization",
    "proxy-connection",
    "referer",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// 这些回答头不交回前端：Cookie 留在原生层，长度和传输方式由这一段字节自己说明。
const RESPONSE_HEADERS_KEPT_HERE: &[&str] = &[
    "connection",
    "content-encoding",
    "content-length",
    "keep-alive",
    "proxy-connection",
    "set-cookie",
    "set-cookie2",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProxyRequestHead {
    method: String,
    /// 相对控制面根地址的路径，可带查询，比如 `auth/session`。
    path: String,
    headers: Vec<(String, String)>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProxyResponseHead {
    status: u16,
    headers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ControlPlaneProxyFailure {
    code: &'static str,
    retryable: bool,
}

impl ControlPlaneProxyFailure {
    const fn new(code: &'static str, retryable: bool) -> Self {
        Self { code, retryable }
    }

    pub(crate) const fn code(self) -> &'static str {
        self.code
    }

    pub(crate) const fn retryable(self) -> bool {
        self.retryable
    }
}

#[derive(Clone)]
pub(crate) struct ControlPlaneProxy {
    base: Url,
    http: Client,
}

impl ControlPlaneProxy {
    pub(crate) fn new(control_plane_url: Url) -> Result<Self, ControlPlaneProxyFailure> {
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            // 跳转原样交回前端，不替它跟到别处去。
            .redirect(redirect::Policy::none())
            .build()
            .map_err(|_| {
                ControlPlaneProxyFailure::new("desktop.control_plane.http_invalid", false)
            })?;
        Ok(Self {
            base: directory_url(control_plane_url),
            http,
        })
    }

    /// 解开前端交来的一段请求，带上登录发给控制面，把回答编成同样格式的一段字节。
    pub(crate) async fn forward(
        &self,
        frame: &[u8],
        origin: &str,
        session_secret: Option<&str>,
    ) -> Result<Vec<u8>, ControlPlaneProxyFailure> {
        let (head, body) = decode_request(frame)?;
        let request = self.build_request(&head, body, origin, session_secret)?;
        let response = self
            .http
            .execute(request)
            .await
            .map_err(|_| unavailable())?;
        let head = ProxyResponseHead {
            status: response.status().as_u16(),
            headers: response_headers(response.headers()),
        };
        let body = response.bytes().await.map_err(|_| unavailable())?;
        encode_frame(&head, &body)
    }

    fn build_request(
        &self,
        head: &ProxyRequestHead,
        body: &[u8],
        origin: &str,
        session_secret: Option<&str>,
    ) -> Result<reqwest::Request, ControlPlaneProxyFailure> {
        let method = proxy_method(&head.method)?;
        let url = self.endpoint(&head.path)?;
        let mut headers = request_headers(&head.headers)?;
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_str(origin).map_err(|_| invalid_request())?,
        );
        headers.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("identity"),
        );
        if let Some(secret) = session_secret {
            let mut cookie = HeaderValue::from_str(&format!("{DESKTOP_SESSION_COOKIE}={secret}"))
                .map_err(|_| invalid_request())?;
            cookie.set_sensitive(true);
            headers.insert(header::COOKIE, cookie);
        }
        let mut request = self.http.request(method.clone(), url).headers(headers);
        if method != Method::GET && method != Method::HEAD {
            request = request.body(body.to_vec());
        }
        request.build().map_err(|_| invalid_request())
    }

    /// 只认控制面根地址下面的相对路径：前端不能借这条路把登录带去别的地方。
    fn endpoint(&self, path: &str) -> Result<Url, ControlPlaneProxyFailure> {
        let route = path.split('?').next().unwrap_or_default();
        if path.is_empty()
            || path.starts_with('/')
            || path.contains(['\\', '#'])
            || path
                .chars()
                .any(|character| character.is_control() || character == ' ')
            || route.split('/').any(|segment| {
                matches!(
                    segment.to_ascii_lowercase().as_str(),
                    "." | ".." | "%2e" | "%2e%2e" | ".%2e" | "%2e."
                )
            })
        {
            return Err(invalid_request());
        }
        let url = self.base.join(path).map_err(|_| invalid_request())?;
        if !url.as_str().starts_with(self.base.as_str()) {
            return Err(invalid_request());
        }
        Ok(url)
    }
}

/// 前端交来的那段字节。走 Tauri 的自定义协议时是原始字节；协议被挡、退回消息通道时是数字数组。
pub(crate) fn request_frame(body: &tauri::ipc::InvokeBody) -> Option<Cow<'_, [u8]>> {
    match body {
        tauri::ipc::InvokeBody::Raw(bytes) => Some(Cow::Borrowed(bytes.as_slice())),
        tauri::ipc::InvokeBody::Json(Value::Array(values)) => values
            .iter()
            .map(|value| value.as_u64().and_then(|byte| u8::try_from(byte).ok()))
            .collect::<Option<Vec<_>>>()
            .map(Cow::Owned),
        tauri::ipc::InvokeBody::Json(_) => None,
    }
}

/// 窗口自己的 Origin。`tauri://localhost` 是不透明来源，不能用 `Url::origin()`，那样得到 `null`。
pub(crate) fn webview_origin(url: &Url) -> Option<String> {
    let host = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    })
}

fn decode_request(frame: &[u8]) -> Result<(ProxyRequestHead, &[u8]), ControlPlaneProxyFailure> {
    let (length, rest) = frame.split_first_chunk::<4>().ok_or_else(invalid_request)?;
    let length = usize::try_from(u32::from_be_bytes(*length)).map_err(|_| invalid_request())?;
    if length > MAX_HEAD_BYTES || length > rest.len() {
        return Err(invalid_request());
    }
    let (head, body) = rest.split_at(length);
    let head = serde_json::from_slice(head).map_err(|_| invalid_request())?;
    Ok((head, body))
}

fn encode_frame(
    head: &ProxyResponseHead,
    body: &[u8],
) -> Result<Vec<u8>, ControlPlaneProxyFailure> {
    let head = serde_json::to_vec(head).map_err(|_| unavailable())?;
    let length = u32::try_from(head.len()).map_err(|_| unavailable())?;
    let mut frame = Vec::with_capacity(4 + head.len() + body.len());
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(&head);
    frame.extend_from_slice(body);
    Ok(frame)
}

fn proxy_method(method: &str) -> Result<Method, ControlPlaneProxyFailure> {
    match method {
        "GET" => Ok(Method::GET),
        "HEAD" => Ok(Method::HEAD),
        "POST" => Ok(Method::POST),
        "PUT" => Ok(Method::PUT),
        "PATCH" => Ok(Method::PATCH),
        "DELETE" => Ok(Method::DELETE),
        _ => Err(invalid_request()),
    }
}

fn request_headers(values: &[(String, String)]) -> Result<HeaderMap, ControlPlaneProxyFailure> {
    if values.len() > MAX_REQUEST_HEADERS {
        return Err(invalid_request());
    }
    let mut headers = HeaderMap::new();
    for (name, value) in values {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| invalid_request())?;
        if REQUEST_HEADERS_SET_HERE.contains(&name.as_str())
            || name.as_str().starts_with("sec-")
            || name.as_str().starts_with("proxy-")
        {
            continue;
        }
        let value = HeaderValue::from_str(value).map_err(|_| invalid_request())?;
        headers.append(name, value);
    }
    Ok(headers)
}

fn response_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(name, _)| !RESPONSE_HEADERS_KEPT_HERE.contains(&name.as_str()))
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.as_str().to_owned(), value.to_owned()))
        })
        .collect()
}

/// `join` 只在根地址以斜杠结尾时才把路径接在它后面，否则会换掉最后一段。
fn directory_url(mut url: Url) -> Url {
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    url.set_query(None);
    url.set_fragment(None);
    url
}

const fn invalid_request() -> ControlPlaneProxyFailure {
    ControlPlaneProxyFailure::new("desktop.control_plane.request_invalid", false)
}

const fn unavailable() -> ControlPlaneProxyFailure {
    ControlPlaneProxyFailure::new("desktop.control_plane.unavailable", true)
}

#[cfg(test)]
mod tests;
