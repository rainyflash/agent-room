use serde_json::json;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
    task::JoinHandle,
};
use url::Url;

use super::{
    ControlPlaneProxy, ProxyResponseHead, decode_request, request_frame, request_headers,
    response_headers, webview_origin,
};

fn proxy(base: &str) -> ControlPlaneProxy {
    ControlPlaneProxy::new(Url::parse(base).expect("测试根地址有效")).expect("可建出代理")
}

fn frame(head: &serde_json::Value, body: &[u8]) -> Vec<u8> {
    let head = serde_json::to_vec(head).expect("请求头可序列化");
    let mut frame = u32::try_from(head.len())
        .expect("请求头不超过 4 GiB")
        .to_be_bytes()
        .to_vec();
    frame.extend_from_slice(&head);
    frame.extend_from_slice(body);
    frame
}

fn split_response(frame: &[u8]) -> (ProxyResponseHead, Vec<u8>) {
    let (length, rest) = frame.split_first_chunk::<4>().expect("回答至少有长度");
    let length = usize::try_from(u32::from_be_bytes(*length)).expect("长度可转换");
    let (head, body) = rest.split_at(length);
    (
        serde_json::from_slice(head).expect("回答头是 JSON"),
        body.to_vec(),
    )
}

/// 只接一个连接的 HTTP 服务：把收到的原始请求交回，再回一段写好的回答。
async fn serve_once(response: &'static str) -> (Url, JoinHandle<String>) {
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
            if read == 0 || request_complete(&received) {
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
    let base = Url::parse(&format!("http://{address}/")).expect("本机根地址有效");
    (base, handle)
}

fn request_complete(received: &[u8]) -> bool {
    let text = String::from_utf8_lossy(received);
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return false;
    };
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    body.len() >= length
}

fn header_lines(request: &str) -> Vec<String> {
    request
        .split("\r\n\r\n")
        .next()
        .unwrap_or_default()
        .lines()
        .skip(1)
        .map(str::to_ascii_lowercase)
        .collect()
}

#[tokio::test]
async fn 代发时带上登录和窗口_origin_回答原样交回且不交出_cookie() {
    let (base, server) = serve_once(concat!(
        "HTTP/1.1 201 Created\r\n",
        "Content-Type: application/json\r\n",
        "Set-Cookie: __Secure-agent-room-desktop-session=rotated; Secure; HttpOnly\r\n",
        "X-Correlation-Id: corr-1\r\n",
        "Content-Length: 11\r\n",
        "\r\n",
        "{\"ok\":true}",
    ))
    .await;
    let request = frame(
        &json!({
            "method": "POST",
            "path": "v1/things?cursor=a%2Fb",
            "headers": [
                ["Content-Type", "application/json"],
                ["Idempotency-Key", "submission-1"],
                ["Cookie", "forged=1"],
                ["Origin", "https://evil.example"],
                ["Sec-Fetch-Mode", "cors"],
            ],
        }),
        b"{\"a\":1}",
    );

    let response = proxy(base.as_str())
        .forward(&request, "tauri://localhost", Some("desktop-secret"))
        .await
        .expect("代发成功");

    let raw = server.await.expect("服务端正常结束");
    assert!(
        raw.starts_with("POST /v1/things?cursor=a%2Fb HTTP/1.1\r\n"),
        "{raw}"
    );
    let headers = header_lines(&raw);
    assert!(
        headers.contains(&"cookie: __secure-agent-room-desktop-session=desktop-secret".to_owned())
    );
    assert!(headers.contains(&"origin: tauri://localhost".to_owned()));
    assert!(headers.contains(&"content-type: application/json".to_owned()));
    assert!(headers.contains(&"idempotency-key: submission-1".to_owned()));
    assert!(headers.contains(&"accept-encoding: identity".to_owned()));
    assert!(
        !headers
            .iter()
            .any(|line| line.contains("forged") || line.contains("evil"))
    );
    assert!(
        !headers
            .iter()
            .any(|line| line.starts_with("sec-fetch-mode"))
    );
    assert!(raw.ends_with("{\"a\":1}"));

    let (head, body) = split_response(&response);
    assert_eq!(head.status, 201);
    assert!(
        head.headers
            .contains(&("content-type".to_owned(), "application/json".to_owned()))
    );
    assert!(
        head.headers
            .contains(&("x-correlation-id".to_owned(), "corr-1".to_owned()))
    );
    assert!(
        !head
            .headers
            .iter()
            .any(|(name, _)| name == "set-cookie" || name == "content-length")
    );
    assert_eq!(body, b"{\"ok\":true}");
}

#[tokio::test]
async fn 没登录时不带_cookie_get_不带正文_跳转原样交回() {
    let (base, server) = serve_once(concat!(
        "HTTP/1.1 302 Found\r\n",
        "Location: https://elsewhere.example/\r\n",
        "Content-Length: 0\r\n",
        "\r\n",
    ))
    .await;
    let request = frame(
        &json!({ "method": "GET", "path": "auth/session", "headers": [["Accept", "application/json"]] }),
        b"ignored",
    );

    let response = proxy(base.as_str())
        .forward(&request, "http://tauri.localhost", None)
        .await
        .expect("代发成功");

    let raw = server.await.expect("服务端正常结束");
    assert!(raw.starts_with("GET /auth/session HTTP/1.1\r\n"), "{raw}");
    let headers = header_lines(&raw);
    assert!(!headers.iter().any(|line| line.starts_with("cookie")));
    assert!(headers.contains(&"origin: http://tauri.localhost".to_owned()));
    assert!(!raw.contains("ignored"));
    let (head, body) = split_response(&response);
    assert_eq!(head.status, 302);
    assert!(head.headers.contains(&(
        "location".to_owned(),
        "https://elsewhere.example/".to_owned()
    )));
    assert!(body.is_empty());
}

#[tokio::test]
async fn 控制面连不上时是可重试的失败() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("可绑定回环端口");
    let address = listener.local_addr().expect("可读本机地址");
    drop(listener);
    let request = frame(
        &json!({ "method": "GET", "path": "health/ready", "headers": [] }),
        b"",
    );

    let failure = proxy(&format!("http://{address}/"))
        .forward(&request, "tauri://localhost", None)
        .await
        .expect_err("端口没人听");

    assert_eq!(failure.code(), "desktop.control_plane.unavailable");
    assert!(failure.retryable());
}

#[test]
fn 只认控制面根地址下面的相对路径() {
    let root = proxy("https://api.example.test");
    assert_eq!(
        root.endpoint("auth/session")
            .expect("相对路径有效")
            .as_str(),
        "https://api.example.test/auth/session"
    );
    assert_eq!(
        root.endpoint("rooms?cursor=a%2Fb")
            .expect("带查询有效")
            .as_str(),
        "https://api.example.test/rooms?cursor=a%2Fb"
    );
    let prefixed = proxy("https://app.example.test/_agent-room/api");
    assert_eq!(
        prefixed
            .endpoint("auth/session")
            .expect("前缀下的路径有效")
            .as_str(),
        "https://app.example.test/_agent-room/api/auth/session"
    );

    for invalid in [
        "",
        "/auth/session",
        "//evil.example/x",
        "https://evil.example/x",
        "evil.example:8080/x",
        "../x",
        "a/../../x",
        "%2e%2e/x",
        "a/%2E%2E/x",
        "./x",
        "a\\b",
        "a#b",
        "a b",
        "a\nb",
    ] {
        assert!(prefixed.endpoint(invalid).is_err(), "应拒绝 {invalid:?}");
        assert!(root.endpoint(invalid).is_err(), "应拒绝 {invalid:?}");
    }
}

#[test]
fn 请求格式不对就拒绝() {
    let valid_head = json!({ "method": "GET", "path": "x", "headers": [] });
    let valid = frame(&valid_head, b"body");
    let (head, body) = decode_request(&valid).expect("格式正确");
    assert_eq!(head.method, "GET");
    assert_eq!(head.path, "x");
    assert_eq!(body, b"body");

    let mut too_long = valid.clone();
    too_long[..4].copy_from_slice(&u32::MAX.to_be_bytes());
    let unknown_field = frame(
        &json!({ "method": "GET", "path": "x", "headers": [], "url": "https://evil.example" }),
        b"",
    );
    for invalid in [
        &b"\0\0"[..],
        &too_long,
        &unknown_field,
        &frame(&json!("x"), b""),
    ] {
        assert_eq!(
            decode_request(invalid).expect_err("应拒绝").code(),
            "desktop.control_plane.request_invalid"
        );
    }
    let not_allowed = frame(
        &json!({ "method": "CONNECT", "path": "x", "headers": [] }),
        b"",
    );
    let (head, _) = decode_request(&not_allowed).expect("格式正确");
    assert!(
        proxy("https://api.example.test")
            .build_request(&head, b"", "tauri://localhost", None)
            .is_err()
    );
}

#[test]
fn 请求头只转前端该管的() {
    let headers = request_headers(&[
        ("Content-Type".to_owned(), "application/json".to_owned()),
        ("Accept".to_owned(), "application/json".to_owned()),
        ("Idempotency-Key".to_owned(), "key".to_owned()),
        ("Cookie".to_owned(), "a=b".to_owned()),
        ("Authorization".to_owned(), "Bearer x".to_owned()),
        ("Host".to_owned(), "evil.example".to_owned()),
        ("Content-Length".to_owned(), "9".to_owned()),
        ("Sec-Fetch-Site".to_owned(), "cross-site".to_owned()),
        ("Proxy-Foo".to_owned(), "bar".to_owned()),
    ])
    .expect("请求头有效");
    let mut names = headers
        .keys()
        .map(reqwest::header::HeaderName::as_str)
        .collect::<Vec<_>>();
    names.sort_unstable();
    assert_eq!(names, ["accept", "content-type", "idempotency-key"]);

    assert!(request_headers(&[("Bad Name".to_owned(), "x".to_owned())]).is_err());
    assert!(request_headers(&[("X-Test".to_owned(), "line\nbreak".to_owned())]).is_err());
    let too_many = (0..65)
        .map(|index| (format!("x-{index}"), "v".to_owned()))
        .collect::<Vec<_>>();
    assert!(request_headers(&too_many).is_err());
}

#[test]
fn 回答头不交出_cookie_和传输细节() {
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in [
        ("content-type", "application/json"),
        ("retry-after", "5"),
        ("set-cookie", "a=b"),
        ("content-length", "2"),
        ("content-encoding", "gzip"),
        ("transfer-encoding", "chunked"),
    ] {
        headers.append(
            reqwest::header::HeaderName::from_static(name),
            reqwest::header::HeaderValue::from_static(value),
        );
    }
    assert_eq!(
        response_headers(&headers),
        [
            ("content-type".to_owned(), "application/json".to_owned()),
            ("retry-after".to_owned(), "5".to_owned()),
        ]
    );
}

#[test]
fn 窗口_origin_按平台的样子写() {
    let origin = |url: &str| webview_origin(&Url::parse(url).expect("测试 URL 有效"));
    assert_eq!(
        origin("tauri://localhost/connect").as_deref(),
        Some("tauri://localhost")
    );
    assert_eq!(
        origin("http://tauri.localhost/").as_deref(),
        Some("http://tauri.localhost")
    );
    assert_eq!(
        origin("http://localhost:1420/x").as_deref(),
        Some("http://localhost:1420")
    );
    assert_eq!(origin("data:text/plain,x"), None);
}

#[test]
fn 收得下原始字节和退回消息通道时的数字数组() {
    let raw = tauri::ipc::InvokeBody::Raw(vec![0, 1, 255]);
    assert_eq!(request_frame(&raw).as_deref(), Some(&[0_u8, 1, 255][..]));
    let numbers = tauri::ipc::InvokeBody::Json(json!([0, 1, 255]));
    assert_eq!(
        request_frame(&numbers).as_deref(),
        Some(&[0_u8, 1, 255][..])
    );
    assert!(request_frame(&tauri::ipc::InvokeBody::Json(json!([256]))).is_none());
    assert!(request_frame(&tauri::ipc::InvokeBody::Json(json!({ "path": "x" }))).is_none());
}
