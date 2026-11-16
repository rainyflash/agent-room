use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use agent_room_a2a_adapter::{DnsResolver, NetworkTargetResult};
use agent_room_application::ports::{
    Clock, PortFuture, RepositoryLinkReader, RepositoryReadFailure,
};
use agent_room_domain::{
    project_rooms::{GithubRepository, RepositoryFile},
    room_listing::RoomShareCode,
    time::UtcMillis,
};
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::TcpListener,
};

use super::{
    GithubLinkReader, MAX_FILE_BYTES, PAUSE_MILLIS, PinnedRawTransport, READS_PER_MINUTE,
    RawEndpoint, RawResponse, RawTransport, RawTransportFailure, contains_link,
};

const SITE: &str = "https://agentroom.chat";
/// 2026-11-16 00:00:30Z：一分钟的中间。
const START: i64 = 1_763_251_230_000;

struct TestClock(AtomicI64);

impl TestClock {
    fn at(millis: i64) -> Arc<Self> {
        Arc::new(Self(AtomicI64::new(millis)))
    }

    fn advance(&self, millis: i64) {
        self.0.fetch_add(millis, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(self.0.load(Ordering::SeqCst)).unwrap()
    }
}

/// 按地址回事先放好的回答，记下读过哪些地址。没放的回 404。
#[derive(Default)]
struct ScriptedTransport {
    answers: Mutex<Vec<(String, Result<RawResponse, RawTransportFailure>)>>,
    requested: Mutex<Vec<String>>,
}

impl ScriptedTransport {
    fn answer(&self, file: &str, response: Result<RawResponse, RawTransportFailure>) {
        self.answers
            .lock()
            .unwrap()
            .push((file.to_owned(), response));
    }

    fn body(&self, file: &str, text: &str) {
        self.answer(
            file,
            Ok(RawResponse::Body {
                bytes: text.as_bytes().to_vec(),
                truncated: false,
            }),
        );
    }

    fn requested(&self) -> Vec<String> {
        self.requested.lock().unwrap().clone()
    }
}

impl RawTransport for ScriptedTransport {
    fn get<'a>(
        &'a self,
        url: &'a str,
        maximum_bytes: usize,
    ) -> PortFuture<'a, Result<RawResponse, RawTransportFailure>> {
        assert_eq!(maximum_bytes, MAX_FILE_BYTES);
        self.requested.lock().unwrap().push(url.to_owned());
        let answer = self
            .answers
            .lock()
            .unwrap()
            .iter()
            .find(|(file, _)| url.ends_with(file.as_str()))
            .map_or(Ok(RawResponse::NotFound), |(_, answer)| answer.clone());
        Box::pin(async move { answer })
    }
}

fn reader(transport: &Arc<ScriptedTransport>, clock: &Arc<TestClock>) -> GithubLinkReader {
    GithubLinkReader::new(
        RawEndpoint::github(),
        transport.clone(),
        SITE.into(),
        clock.clone(),
    )
}

fn code(value: &str) -> RoomShareCode {
    RoomShareCode::parse(value).unwrap()
}

fn repository() -> GithubRepository {
    GithubRepository::parse("rainyflash/agent-room").unwrap()
}

#[test]
fn 只认站点加码_后面不能紧跟字母或数字() {
    let code = code("K7p3Qx");
    for found in [
        "Agent Room: this project's room is https://agentroom.chat/r/K7p3Qx. Agents join",
        "[![Agent Room](https://agentroom.chat/gh/o/r/badge.svg)](https://agentroom.chat/r/K7p3Qx)",
        "https://agentroom.chat/r/K7p3Qx",
        "see https://agentroom.chat/r/K7p3Qx\n",
        "https://agentroom.chat/r/K7p3Qx/live",
        "https://agentroom.chat/r/K7p3Qx-",
        "first https://agentroom.chat/r/K7p3Qxy then https://agentroom.chat/r/K7p3Qx",
    ] {
        assert!(contains_link(found.as_bytes(), SITE, &code), "{found}");
    }
    for missing in [
        "https://agentroom.chat/r/K7p3Qxy",
        "https://agentroom.chat/r/K7p3Qx9",
        "https://agentroom.chat/r/k7p3qx",
        "https://agentroom.chat/gh/K7p3Qx",
        "http://agentroom.chat/r/K7p3Qx",
        "https://agentroom.chat.example/r/K7p3Qx",
        "agentroom.chat/r/K7p3Qx",
        "",
    ] {
        assert!(!contains_link(missing.as_bytes(), SITE, &code), "{missing}");
    }
    // 读到上限时可能断在一个字的中间：按字节找，不当成坏文件。
    let mut cut = "说明 https://agentroom.chat/r/K7p3Qx ".as_bytes().to_vec();
    cut.extend_from_slice(&"中".as_bytes()[..2]);
    assert!(contains_link(&cut, SITE, &code));
}

#[tokio::test]
async fn 先读_agents_找到了就不读_readme_地址照规矩拼() {
    let transport = Arc::new(ScriptedTransport::default());
    transport.body("AGENTS.md", "room https://agentroom.chat/r/K7p3Qx");
    let clock = TestClock::at(START);
    let reader = reader(&transport, &clock);

    let scan = reader
        .scan(&repository(), &[code("K7p3Qx")])
        .await
        .expect("读得到");

    assert_eq!(
        scan.file_of(&code("K7p3Qx")),
        Some(RepositoryFile::AgentsMd)
    );
    assert!(!scan.truncated);
    assert_eq!(
        transport.requested(),
        ["https://raw.githubusercontent.com/rainyflash/agent-room/HEAD/AGENTS.md"]
    );
}

#[tokio::test]
async fn 两个码都要找时_agents_里只有一个就接着读_readme() {
    let transport = Arc::new(ScriptedTransport::default());
    transport.body("AGENTS.md", "https://agentroom.chat/r/NewNew");
    transport.body("README.md", "[badge](https://agentroom.chat/r/OldOld)");
    let clock = TestClock::at(START);
    let reader = reader(&transport, &clock);

    let scan = reader
        .scan(&repository(), &[code("NewNew"), code("OldOld")])
        .await
        .expect("读得到");

    assert_eq!(
        scan.file_of(&code("NewNew")),
        Some(RepositoryFile::AgentsMd)
    );
    assert_eq!(
        scan.file_of(&code("OldOld")),
        Some(RepositoryFile::ReadmeMd)
    );
    assert_eq!(transport.requested().len(), 2);
}

#[tokio::test]
async fn 没有文件不算出错_跳转当成改过名_太大的照实说() {
    let clock = TestClock::at(START);
    let missing = Arc::new(ScriptedTransport::default());
    let scan = reader(&missing, &clock)
        .scan(&repository(), &[code("K7p3Qx")])
        .await
        .expect("两个文件都没有也读完了");
    assert!(scan.found.is_empty());
    assert_eq!(missing.requested().len(), 2);

    let renamed = Arc::new(ScriptedTransport::default());
    renamed.answer("AGENTS.md", Ok(RawResponse::Redirect));
    assert_eq!(
        reader(&renamed, &clock)
            .scan(&repository(), &[code("K7p3Qx")])
            .await,
        Err(RepositoryReadFailure::Renamed)
    );
    assert_eq!(renamed.requested().len(), 1, "跳转以后不再读别的");

    let large = Arc::new(ScriptedTransport::default());
    large.answer(
        "README.md",
        Ok(RawResponse::Body {
            bytes: vec![b'x'; 16],
            truncated: true,
        }),
    );
    let scan = reader(&large, &clock)
        .scan(&repository(), &[code("K7p3Qx")])
        .await
        .expect("读得到开头");
    assert!(scan.found.is_empty());
    assert!(scan.truncated);

    let broken = Arc::new(ScriptedTransport::default());
    broken.answer("AGENTS.md", Err(RawTransportFailure::Blocked));
    assert_eq!(
        reader(&broken, &clock)
            .scan(&repository(), &[code("K7p3Qx")])
            .await,
        Err(RepositoryReadFailure::Unavailable { retry_at: None })
    );
}

#[tokio::test]
async fn 每分钟最多读_60_次_满了排到下一分钟() {
    let transport = Arc::new(ScriptedTransport::default());
    let clock = TestClock::at(START);
    let reader = reader(&transport, &clock);
    let codes = [code("K7p3Qx")];

    for _ in 0..READS_PER_MINUTE / 2 {
        reader.scan(&repository(), &codes).await.expect("没超");
    }
    let next_minute = UtcMillis::new(START - START % 60_000 + 60_000).unwrap();
    assert_eq!(reader.next_read_at(), Some(next_minute));
    assert_eq!(
        reader.scan(&repository(), &codes).await,
        Err(RepositoryReadFailure::Unavailable {
            retry_at: Some(next_minute)
        })
    );
    assert_eq!(transport.requested().len(), 60, "满了就不再去读");

    clock.advance(30_000);
    assert_eq!(reader.next_read_at(), None);
    reader
        .scan(&repository(), &codes)
        .await
        .expect("下一分钟又能读");
}

#[tokio::test]
async fn github_回_429_或_403_全体停十分钟() {
    {
        let transport = Arc::new(ScriptedTransport::default());
        transport.answer("AGENTS.md", Ok(RawResponse::Limited));
        let clock = TestClock::at(START);
        let reader = reader(&transport, &clock);
        let paused_until = UtcMillis::new(START + PAUSE_MILLIS).unwrap();

        assert_eq!(
            reader.scan(&repository(), &[code("K7p3Qx")]).await,
            Err(RepositoryReadFailure::Unavailable {
                retry_at: Some(paused_until)
            })
        );
        clock.advance(PAUSE_MILLIS - 1);
        assert_eq!(reader.next_read_at(), Some(paused_until));
        assert_eq!(
            reader.scan(&repository(), &[code("NewNew")]).await,
            Err(RepositoryReadFailure::Unavailable {
                retry_at: Some(paused_until)
            })
        );
        assert_eq!(transport.requested().len(), 1, "停着的时候一次也不读");
        clock.advance(1);
        assert_eq!(reader.next_read_at(), None);
    }
}

/// 一直不回答。
struct HangingTransport(AtomicUsize);

impl RawTransport for HangingTransport {
    fn get<'a>(
        &'a self,
        _url: &'a str,
        _maximum_bytes: usize,
    ) -> PortFuture<'a, Result<RawResponse, RawTransportFailure>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            tokio::time::sleep(Duration::from_mins(1)).await;
            Ok(RawResponse::NotFound)
        })
    }
}

#[tokio::test(start_paused = true)]
async fn 整次核对五秒超时() {
    let transport = Arc::new(HangingTransport(AtomicUsize::new(0)));
    let reader = GithubLinkReader::new(
        RawEndpoint::github(),
        transport.clone(),
        SITE.into(),
        TestClock::at(START),
    );
    let started = tokio::time::Instant::now();

    let scanned = reader.scan(&repository(), &[code("K7p3Qx")]).await;

    assert_eq!(
        scanned,
        Err(RepositoryReadFailure::Unavailable { retry_at: None })
    );
    assert_eq!(started.elapsed(), Duration::from_secs(5));
    assert_eq!(transport.0.load(Ordering::SeqCst), 1);
}

struct FixedResolver(Vec<SocketAddr>, AtomicUsize);

impl DnsResolver for FixedResolver {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        _port: u16,
    ) -> PortFuture<'a, NetworkTargetResult<Vec<SocketAddr>>> {
        self.1.fetch_add(1, Ordering::SeqCst);
        let addresses = self.0.clone();
        let host = host.to_owned();
        Box::pin(async move {
            assert!(host == "raw.githubusercontent.com" || host == "127.0.0.1");
            Ok(addresses)
        })
    }
}

#[tokio::test]
async fn 解析到内网_回环_链路本地的一律不连() {
    for blocked in [
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7)),
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
        IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
        "::1".parse().unwrap(),
        "fe80::1".parse().unwrap(),
    ] {
        let resolver = Arc::new(FixedResolver(
            vec![
                SocketAddr::new(IpAddr::V4(Ipv4Addr::new(185, 199, 108, 133)), 443),
                SocketAddr::new(blocked, 443),
            ],
            AtomicUsize::new(0),
        ));
        let transport = PinnedRawTransport::new(RawEndpoint::github(), resolver.clone());

        let result = transport
            .get(
                "https://raw.githubusercontent.com/o/r/HEAD/AGENTS.md",
                MAX_FILE_BYTES,
            )
            .await;

        assert_eq!(result, Err(RawTransportFailure::Blocked), "{blocked}");
        assert_eq!(resolver.1.load(Ordering::SeqCst), 1);
    }
}

/// 本机一个只回一种答案的 HTTP 服务：回答写在 `response` 里，读完请求头就写。
async fn local_server(response: Vec<u8>) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let response = response.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0_u8; 1_024];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    match stream.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => request.extend_from_slice(&buffer[..read]),
                    }
                }
                // 客户端读够以后断开，后面写不进去是正常的。
                let _ = stream.write_all(&response).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    address
}

fn local_transport(address: SocketAddr) -> PinnedRawTransport {
    PinnedRawTransport::new(
        RawEndpoint::plain_http_for_tests("127.0.0.1", address.port()),
        Arc::new(FixedResolver(vec![address], AtomicUsize::new(0))),
    )
}

#[tokio::test]
async fn 真的连接_不跟跳转_读够_512_kb_就断开() {
    let redirect = local_server(
        b"HTTP/1.1 301 Moved Permanently\r\nLocation: http://127.0.0.1:9/elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .to_vec(),
    )
    .await;
    let url = format!("http://127.0.0.1:{}/o/r/HEAD/AGENTS.md", redirect.port());
    assert_eq!(
        local_transport(redirect).get(&url, MAX_FILE_BYTES).await,
        Ok(RawResponse::Redirect)
    );

    let size = 2 * MAX_FILE_BYTES;
    let mut large = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n"
    )
    .into_bytes();
    large.extend(std::iter::repeat_n(b'a', size));
    let large = local_server(large).await;
    let url = format!("http://127.0.0.1:{}/o/r/HEAD/README.md", large.port());
    match local_transport(large).get(&url, MAX_FILE_BYTES).await {
        Ok(RawResponse::Body { bytes, truncated }) => {
            assert_eq!(bytes.len(), MAX_FILE_BYTES);
            assert!(truncated);
        }
        other => panic!("应该只读到上限：{other:?}"),
    }

    let small = local_server(
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello".to_vec(),
    )
    .await;
    let url = format!("http://127.0.0.1:{}/o/r/HEAD/README.md", small.port());
    assert_eq!(
        local_transport(small).get(&url, MAX_FILE_BYTES).await,
        Ok(RawResponse::Body {
            bytes: b"hello".to_vec(),
            truncated: false
        })
    );

    for (status, expected) in [
        ("404 Not Found", RawResponse::NotFound),
        ("429 Too Many Requests", RawResponse::Limited),
        ("403 Forbidden", RawResponse::Limited),
        ("500 Internal Server Error", RawResponse::Failed),
    ] {
        let server = local_server(
            format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .into_bytes(),
        )
        .await;
        let url = format!("http://127.0.0.1:{}/o/r/HEAD/AGENTS.md", server.port());
        assert_eq!(
            local_transport(server).get(&url, MAX_FILE_BYTES).await,
            Ok(expected),
            "{status}"
        );
    }
}

#[test]
fn 生产上只连_raw_githubusercontent_com() {
    let endpoint = RawEndpoint::github();
    assert_eq!(
        endpoint.file_url(
            &GithubRepository::parse("Owner-1/repo.name_x").unwrap(),
            RepositoryFile::ReadmeMd
        ),
        "https://raw.githubusercontent.com/Owner-1/repo.name_x/HEAD/README.md"
    );
}
