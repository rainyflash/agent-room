use agent_room_bridge_core::ipc::{IpcInstallationId, IpcProtocolVersion};
use agent_room_bridge_ipc::{
    IpcClientCredentials, IpcCloseHostSessionRequest, IpcFrame, IpcFrameCodec, IpcHostSessionState,
    IpcHostSessionSummary, IpcScopeName, IpcSharedSecret,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use tokio::{
    io::{DuplexStream, duplex},
    task::JoinHandle,
    time::{Instant, sleep},
};

use super::*;

const SESSION_ID: &str = "019d2c44-1dc4-7a5b-9e32-2f3c1d4b5a60";

#[test]
fn 客户端给凭据故障保留可修复语义() {
    let missing = LocalBridgeClientFailure::credential(IpcCredentialFailure::new(
        IpcCredentialFailureKind::Missing,
    ));
    assert_eq!(
        missing.kind(),
        LocalBridgeClientFailureKind::CredentialsMissing
    );
    assert_eq!(missing.code(), "bridge.ipc.credentials_missing");
    assert!(!missing.retryable());
}

fn close_method() -> IpcMethod {
    IpcMethod::CloseHostSession(IpcCloseHostSessionRequest {
        session_id: SESSION_ID.to_owned(),
    })
}

fn closed_response() -> IpcResponse {
    IpcResponse::HostSession {
        session: IpcHostSessionSummary {
            session_id: SESSION_ID.to_owned(),
            state: IpcHostSessionState::Closed,
            agent_id: None,
            error_code: None,
        },
    }
}

async fn delayed_server(
    method: IpcMethod,
    delay: Duration,
) -> (IpcClientSession<DuplexStream>, JoinHandle<()>) {
    let scope = method.required_scope();
    let (client_stream, mut server_stream) = duplex(16 * 1024);
    let server = tokio::spawn(async move {
        assert!(matches!(
            IpcFrameCodec::read(&mut server_stream).await.unwrap(),
            IpcFrame::ClientHello { .. }
        ));
        IpcFrameCodec::write(
            &mut server_stream,
            &IpcFrame::ServerChallenge {
                challenge_id: SESSION_ID.parse().unwrap(),
                challenge: URL_SAFE_NO_PAD.encode([9_u8; 32]),
                selected_version: IpcProtocolVersion::V4_6.into(),
                granted_scopes: vec![IpcScopeName::from(scope)],
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            IpcFrameCodec::read(&mut server_stream).await.unwrap(),
            IpcFrame::ClientProof { .. }
        ));
        IpcFrameCodec::write(
            &mut server_stream,
            &IpcFrame::ServerReady {
                server_instance_id: SESSION_ID.parse().unwrap(),
                selected_version: IpcProtocolVersion::V4_6.into(),
                granted_scopes: vec![IpcScopeName::from(scope)],
            },
        )
        .await
        .unwrap();
        let IpcFrame::Request {
            correlation_id,
            method: received,
        } = IpcFrameCodec::read(&mut server_stream).await.unwrap()
        else {
            panic!("已鉴权连接应收到请求");
        };
        assert_eq!(received, method);
        sleep(delay).await;
        IpcFrameCodec::write(
            &mut server_stream,
            &IpcFrame::Response {
                correlation_id,
                result: closed_response(),
            },
        )
        .await
        .unwrap();
    });
    let credentials = IpcClientCredentials::new(
        IpcInstallationId::new("deadline_test").unwrap(),
        IpcSharedSecret::new([7; 32]),
    );
    let client = IpcClientSession::authenticate(
        client_stream,
        &credentials,
        IpcCallerKind::McpServer,
        [scope],
    )
    .await
    .unwrap();
    (client, server)
}

#[tokio::test(start_paused = true)]
async fn 关闭等待长轮询排空后返回原句柄的关闭确认() {
    let method = close_method();
    let (mut session, server) = delayed_server(method.clone(), Duration::from_secs(35)).await;
    let started = Instant::now();
    let response = LocalBridgeClient::system(PathBuf::new())
        .request(&mut session, method)
        .await
        .unwrap();
    assert_eq!(response, closed_response());
    assert_eq!(started.elapsed(), Duration::from_secs(35));
    server.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn 普通请求仍在十五秒返回可重试超时() {
    let method = IpcMethod::BridgeStatus;
    let (mut session, server) = delayed_server(method.clone(), Duration::from_secs(35)).await;
    let started = Instant::now();
    let failure = LocalBridgeClient::system(PathBuf::new())
        .request(&mut session, method)
        .await
        .unwrap_err();
    assert_eq!(failure.code(), "bridge.ipc.timeout");
    assert!(failure.retryable());
    assert_eq!(started.elapsed(), Duration::from_secs(15));
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[tokio::test(start_paused = true)]
async fn 关闭卡住仍返回超时而不伪造成功() {
    let method = close_method();
    let (mut session, server) = delayed_server(method.clone(), Duration::from_mins(3)).await;
    let started = Instant::now();
    let failure = LocalBridgeClient::system(PathBuf::new())
        .request(&mut session, method)
        .await
        .unwrap_err();
    assert_eq!(failure.code(), "bridge.ipc.timeout");
    assert!(failure.retryable());
    assert_eq!(started.elapsed(), Duration::from_mins(2));
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[cfg(unix)]
#[tokio::test]
async fn unix_有_bridge_监听才读取凭据() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct 计数凭据(AtomicUsize);

    impl IpcCredentialSource for 计数凭据 {
        fn load(&self) -> Result<IpcClientCredentials, IpcCredentialFailure> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(IpcCredentialFailure::new(
                IpcCredentialFailureKind::Unavailable,
            ))
        }
    }

    let runtime_root = tempfile::tempdir().unwrap();
    let credentials = Arc::new(计数凭据::default());
    let client = LocalBridgeClient {
        runtime_root: runtime_root.path().to_path_buf(),
        credentials: credentials.clone(),
        caller: IpcCallerKind::AgentCli,
        connect_timeout: DEFAULT_CONNECT_TIMEOUT,
        operation_timeout: DEFAULT_OPERATION_TIMEOUT,
    };

    let absent = client.invoke(IpcMethod::BridgeStatus).await.unwrap_err();
    assert_eq!(
        absent.kind(),
        LocalBridgeClientFailureKind::BridgeUnavailable
    );
    assert_eq!(credentials.0.load(Ordering::SeqCst), 0);

    let _listener =
        std::os::unix::net::UnixListener::bind(runtime_root.path().join("bridge.sock")).unwrap();
    let unreadable = client.invoke(IpcMethod::BridgeStatus).await.unwrap_err();
    assert_eq!(
        unreadable.kind(),
        LocalBridgeClientFailureKind::CredentialsUnavailable
    );
    assert_eq!(credentials.0.load(Ordering::SeqCst), 1);
}

/// 记着读了几次、作废了几次的凭据；`cached` 表示进程里还留着一份。
#[cfg(unix)]
struct 留着的凭据 {
    loads: std::sync::atomic::AtomicUsize,
    forgets: std::sync::atomic::AtomicUsize,
    cached: std::sync::atomic::AtomicBool,
}

#[cfg(unix)]
impl 留着的凭据 {
    fn new(cached: bool) -> Arc<Self> {
        Arc::new(Self {
            loads: 0.into(),
            forgets: 0.into(),
            cached: cached.into(),
        })
    }
}

#[cfg(unix)]
impl IpcCredentialSource for 留着的凭据 {
    fn load(&self) -> Result<IpcClientCredentials, IpcCredentialFailure> {
        self.loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(IpcClientCredentials::new(
            IpcInstallationId::new("cached_test").unwrap(),
            IpcSharedSecret::new([7; 32]),
        ))
    }

    fn forget_cached(&self) -> bool {
        self.forgets
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.cached.swap(false, std::sync::atomic::Ordering::SeqCst)
    }
}

/// 在 `bridge.sock` 上起一个假 Bridge：前 `rejected` 个连接在握手时说不认这份凭据，之后的照常
/// 完成握手、回答一个请求。返回它一共接了几个连接。
#[cfg(unix)]
fn rejecting_bridge(
    runtime_root: &std::path::Path,
    rejected: usize,
    scope: IpcScope,
) -> JoinHandle<usize> {
    let listener = tokio::net::UnixListener::bind(runtime_root.join("bridge.sock")).unwrap();
    tokio::spawn(async move {
        let mut accepted = 0;
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            accepted += 1;
            assert!(matches!(
                IpcFrameCodec::read(&mut stream).await.unwrap(),
                IpcFrame::ClientHello { .. }
            ));
            if accepted <= rejected {
                IpcFrameCodec::write(
                    &mut stream,
                    &IpcFrame::Error {
                        correlation_id: None,
                        code: "bridge.ipc.installation_rejected".to_owned(),
                        category: IpcErrorCategory::Authentication,
                        retryable: false,
                        details: BTreeMap::new(),
                    },
                )
                .await
                .unwrap();
                continue;
            }
            IpcFrameCodec::write(
                &mut stream,
                &IpcFrame::ServerChallenge {
                    challenge_id: SESSION_ID.parse().unwrap(),
                    challenge: URL_SAFE_NO_PAD.encode([9_u8; 32]),
                    selected_version: IpcProtocolVersion::V4_6.into(),
                    granted_scopes: vec![IpcScopeName::from(scope)],
                },
            )
            .await
            .unwrap();
            assert!(matches!(
                IpcFrameCodec::read(&mut stream).await.unwrap(),
                IpcFrame::ClientProof { .. }
            ));
            IpcFrameCodec::write(
                &mut stream,
                &IpcFrame::ServerReady {
                    server_instance_id: SESSION_ID.parse().unwrap(),
                    selected_version: IpcProtocolVersion::V4_6.into(),
                    granted_scopes: vec![IpcScopeName::from(scope)],
                },
            )
            .await
            .unwrap();
            let IpcFrame::Request { correlation_id, .. } =
                IpcFrameCodec::read(&mut stream).await.unwrap()
            else {
                panic!("已鉴权连接应收到请求");
            };
            IpcFrameCodec::write(
                &mut stream,
                &IpcFrame::Response {
                    correlation_id,
                    result: closed_response(),
                },
            )
            .await
            .unwrap();
            return accepted;
        }
    })
}

#[cfg(unix)]
fn client_with(
    runtime_root: &std::path::Path, credentials: Arc<留着的凭据>
) -> LocalBridgeClient {
    LocalBridgeClient {
        runtime_root: runtime_root.to_path_buf(),
        credentials,
        caller: IpcCallerKind::McpServer,
        connect_timeout: DEFAULT_CONNECT_TIMEOUT,
        operation_timeout: DEFAULT_OPERATION_TIMEOUT,
    }
}

#[cfg(unix)]
#[tokio::test]
async fn unix_bridge_不认留着的凭据时作废重读再连一次() {
    use std::sync::atomic::Ordering;

    let runtime_root = tempfile::tempdir().unwrap();
    let bridge = rejecting_bridge(runtime_root.path(), 1, close_method().required_scope());
    let credentials = 留着的凭据::new(true);

    let response = client_with(runtime_root.path(), credentials.clone())
        .invoke(close_method())
        .await
        .unwrap();

    assert_eq!(response, closed_response());
    assert_eq!(bridge.await.unwrap(), 2);
    assert_eq!(credentials.forgets.load(Ordering::SeqCst), 1);
    assert_eq!(credentials.loads.load(Ordering::SeqCst), 2);
}

#[cfg(unix)]
#[tokio::test]
async fn unix_没有留着的凭据时被拒不再重连() {
    use std::sync::atomic::Ordering;

    let runtime_root = tempfile::tempdir().unwrap();
    let bridge = rejecting_bridge(
        runtime_root.path(),
        usize::MAX,
        close_method().required_scope(),
    );
    let credentials = 留着的凭据::new(false);

    let failure = client_with(runtime_root.path(), credentials.clone())
        .invoke(close_method())
        .await
        .unwrap_err();

    assert_eq!(failure.code(), "bridge.ipc.installation_rejected");
    assert_eq!(credentials.forgets.load(Ordering::SeqCst), 1);
    assert_eq!(credentials.loads.load(Ordering::SeqCst), 1);
    bridge.abort();
}
