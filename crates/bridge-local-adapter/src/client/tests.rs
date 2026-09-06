use agent_room_bridge_core::ipc::IpcInstallationId;
use agent_room_bridge_ipc::{
    IpcClientCredentials, IpcCloseHostSessionRequest, IpcFrame, IpcFrameCodec, IpcHostSessionState,
    IpcHostSessionSummary, IpcScopeName, IpcSharedSecret, IpcVersion,
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
                selected_version: IpcVersion { major: 3, minor: 0 },
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
                selected_version: IpcVersion { major: 3, minor: 0 },
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
