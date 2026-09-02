use std::{io, net::Ipv4Addr, time::Duration};

use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
    time::{Instant, timeout},
};
use url::Url;

const HUMAN_SESSION_CALLBACK_PATH: &str = "/auth/callback";
const MATRIX_SESSION_CALLBACK_PREFIX: &str = "/matrix/callback/";
const MAX_REQUEST_HEAD_BYTES: usize = 16 * 1_024;
const SOCKET_IO_TIMEOUT: Duration = Duration::from_secs(5);

/// 只在当前认证事务期间存活的本机 HTTP 回调监听器。
///
/// 它绑定随机回环端口，不暴露到局域网，也不把长期凭据交给浏览器。
pub(crate) struct LoopbackCallbackListener {
    callback_url: Url,
    listener: TcpListener,
}

impl LoopbackCallbackListener {
    pub(crate) async fn bind_human_session() -> io::Result<Self> {
        Self::bind_path(HUMAN_SESSION_CALLBACK_PATH).await
    }

    pub(crate) async fn bind_matrix_session(transaction_id: &str) -> io::Result<Self> {
        if !(32..=128).contains(&transaction_id.len())
            || !transaction_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Matrix 回调事务标识无效",
            ));
        }
        Self::bind_path(&format!("{MATRIX_SESSION_CALLBACK_PREFIX}{transaction_id}")).await
    }

    async fn bind_path(callback_path: &str) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let port = listener.local_addr()?.port();
        let callback_url = Url::parse(&format!(
            "http://{}:{port}{callback_path}",
            Ipv4Addr::LOCALHOST
        ))
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        Ok(Self {
            callback_url,
            listener,
        })
    }

    pub(crate) fn callback_url(&self) -> &Url {
        &self.callback_url
    }

    /// 等待首个结构有效的回环请求；探测流量会收到拒绝响应并被忽略。
    pub(crate) async fn wait(
        self,
        lifetime: Duration,
    ) -> Result<LoopbackCallbackRequest, LoopbackCallbackFailure> {
        let deadline = Instant::now() + lifetime;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(LoopbackCallbackFailure::Timeout);
            }
            let accepted = timeout(remaining, self.listener.accept())
                .await
                .map_err(|_| LoopbackCallbackFailure::Timeout)?
                .map_err(|_| LoopbackCallbackFailure::Unavailable)?;
            let (mut stream, peer) = accepted;
            if !peer.ip().is_loopback() {
                let _ = write_response(&mut stream, CallbackResponse::Rejected).await;
                continue;
            }
            match read_callback_url(&mut stream, &self.callback_url).await {
                Ok(callback_url) => {
                    return Ok(LoopbackCallbackRequest {
                        callback_url,
                        stream,
                    });
                }
                Err(()) => {
                    let _ = write_response(&mut stream, CallbackResponse::Rejected).await;
                }
            }
        }
    }
}

pub(crate) struct LoopbackCallbackRequest {
    callback_url: Url,
    stream: TcpStream,
}

impl LoopbackCallbackRequest {
    pub(crate) fn callback_url(&self) -> &Url {
        &self.callback_url
    }

    pub(crate) async fn respond(mut self, authenticated: bool) -> io::Result<()> {
        let response = if authenticated {
            CallbackResponse::Authenticated
        } else {
            CallbackResponse::Rejected
        };
        write_response(&mut self.stream, response).await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoopbackCallbackFailure {
    Timeout,
    Unavailable,
}

async fn read_callback_url(stream: &mut TcpStream, callback_base: &Url) -> Result<Url, ()> {
    let mut request = Vec::with_capacity(2_048);
    let mut chunk = [0_u8; 2_048];
    loop {
        if request.len() >= MAX_REQUEST_HEAD_BYTES {
            return Err(());
        }
        let read = timeout(SOCKET_IO_TIMEOUT, stream.read(&mut chunk))
            .await
            .map_err(|_| ())?
            .map_err(|_| ())?;
        if read == 0 {
            return Err(());
        }
        request.extend_from_slice(&chunk[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let request = std::str::from_utf8(&request).map_err(|_| ())?;
    let mut lines = request.split("\r\n");
    let request_line = lines.next().ok_or(())?;
    let mut parts = request_line.split_ascii_whitespace();
    let method = parts.next().ok_or(())?;
    let target = parts.next().ok_or(())?;
    let version = parts.next().ok_or(())?;
    if parts.next().is_some()
        || method != "GET"
        || !matches!(version, "HTTP/1.0" | "HTTP/1.1")
        || !target.starts_with(callback_base.path())
        || target.starts_with("//")
        || target.contains('#')
    {
        return Err(());
    }
    let expected_host = callback_base
        .host_str()
        .zip(callback_base.port())
        .map(|(host, port)| format!("{host}:{port}"))
        .ok_or(())?;
    let host_matches = lines
        .filter_map(|line| line.split_once(':'))
        .any(|(name, value)| {
            name.eq_ignore_ascii_case("host") && value.trim().eq_ignore_ascii_case(&expected_host)
        });
    if !host_matches {
        return Err(());
    }
    let callback = callback_base.join(target).map_err(|_| ())?;
    if callback.scheme() != "http"
        || callback.host_str() != callback_base.host_str()
        || callback.port() != callback_base.port()
        || callback.path() != callback_base.path()
        || callback.fragment().is_some()
        || callback.query().is_none()
    {
        return Err(());
    }
    Ok(callback)
}

#[derive(Clone, Copy)]
enum CallbackResponse {
    Authenticated,
    Rejected,
}

async fn write_response(stream: &mut TcpStream, response: CallbackResponse) -> io::Result<()> {
    let (status, title, message) = match response {
        CallbackResponse::Authenticated => (
            "200 OK",
            "Agent Room connected",
            "Agent Room 已连接。现在可以关闭此页面并返回桌面应用。",
        ),
        CallbackResponse::Rejected => (
            "400 Bad Request",
            "Agent Room authentication failed",
            "认证回调无效或已过期。请返回 Agent Room 桌面应用重试。",
        ),
    };
    let body = format!(
        "<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{title}</title><body><main><h1>{title}</h1><p>{message}</p></main></body></html>"
    );
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; base-uri 'none'; frame-ancestors 'none'\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    );
    timeout(SOCKET_IO_TIMEOUT, async {
        stream.write_all(head.as_bytes()).await?;
        stream.write_all(body.as_bytes()).await?;
        stream.shutdown().await
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "本机认证响应超时"))?
}

#[cfg(test)]
mod tests {
    use super::LoopbackCallbackListener;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    #[tokio::test]
    async fn 回环监听器只接受自身端口上的闭合回调() {
        let listener = LoopbackCallbackListener::bind_human_session()
            .await
            .expect("回环端口可绑定");
        let callback_base = listener.callback_url().clone();
        let state = "abcdefghijklmnopqrstuvwxyzABCDEF";
        let client = tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect((
                callback_base.host_str().expect("回调主机存在"),
                callback_base.port().expect("回调端口存在"),
            ))
            .await
            .expect("测试客户端可连接");
            let request = format!(
                "GET /auth/callback?code=one-time-code&state={state} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
                callback_base.host_str().expect("回调主机存在"),
                callback_base.port().expect("回调端口存在")
            );
            stream
                .write_all(request.as_bytes())
                .await
                .expect("测试请求可发送");
            let mut response = String::new();
            stream
                .read_to_string(&mut response)
                .await
                .expect("测试响应可读取");
            response
        });
        let callback = listener
            .wait(std::time::Duration::from_secs(2))
            .await
            .expect("闭合回调可接收");
        assert_eq!(callback.callback_url().query_pairs().count(), 2);
        callback.respond(true).await.expect("成功响应可写入");
        assert!(
            client
                .await
                .expect("客户端任务完成")
                .starts_with("HTTP/1.1 200 OK")
        );
    }

    #[tokio::test]
    async fn matrix_回调使用不可猜测事务路径并拒绝相邻路径() {
        let transaction_id = "abcdefghijklmnopqrstuvwxyzABCDEF0123456789_-";
        let listener = LoopbackCallbackListener::bind_matrix_session(transaction_id)
            .await
            .expect("Matrix 回环端口可绑定");
        assert_eq!(
            listener.callback_url().path(),
            format!("/matrix/callback/{transaction_id}")
        );
        assert!(
            LoopbackCallbackListener::bind_matrix_session("unsafe/path")
                .await
                .is_err()
        );
    }
}
