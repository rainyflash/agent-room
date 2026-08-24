use std::{collections::BTreeMap, io, sync::Arc, time::Duration};

use agent_room_bridge_core::ipc::{
    FoundationIpcScopePolicy, IpcHandshakeAgreement, IpcHandshakeFailureKind,
    IpcHandshakeNegotiator, IpcInstallationId, IpcProtocolVersion, IpcScope,
};
use agent_room_bridge_ipc::{
    IpcBridgeState, IpcChallenge, IpcChallengeProof, IpcErrorCategory, IpcFrame, IpcFrameCodec,
    IpcMethod, IpcProtocolFailureKind, IpcResponse, IpcSharedSecret, IpcVersion,
    client_offer_from_frame, server_agreement_frame, verify_challenge_proof,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use interprocess::local_socket::{
    ListenerOptions,
    tokio::{Listener, prelude::*},
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::watch,
    task::JoinSet,
    time::timeout,
};
use uuid::Uuid;

use crate::runtime_files::BridgeRuntimePaths;

const IPC_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const IPC_SECRET_BYTES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BridgeStatusSnapshot {
    pub(crate) state: IpcBridgeState,
    pub(crate) started_at_unix_ms: i64,
}

pub(crate) trait BridgeStatusReader: Send + Sync {
    fn read_status(&self) -> BridgeStatusSnapshot;
}

#[derive(Debug, Clone)]
struct BridgeIpcEndpoint {
    platform_name: String,
}

impl BridgeIpcEndpoint {
    fn from_installation(paths: &BridgeRuntimePaths, installation_id: &IpcInstallationId) -> Self {
        #[cfg(windows)]
        let platform_name = {
            let _ = paths;
            format!("agent-room-bridge-{}.sock", installation_id.as_str())
        };
        #[cfg(unix)]
        let platform_name = paths
            .runtime_root()
            .join(format!("bridge-{}.sock", installation_id.as_str()))
            .to_string_lossy()
            .into_owned();
        #[cfg(not(any(windows, unix)))]
        let platform_name = {
            let _ = paths;
            format!("agent-room-bridge-{}.sock", installation_id.as_str())
        };
        Self { platform_name }
    }

    #[cfg(windows)]
    fn to_name(&self) -> io::Result<interprocess::local_socket::Name<'_>> {
        use interprocess::local_socket::{GenericNamespaced, ToNsName as _};

        self.platform_name
            .as_str()
            .to_ns_name::<GenericNamespaced>()
    }

    #[cfg(unix)]
    fn to_name(&self) -> io::Result<interprocess::local_socket::Name<'_>> {
        use interprocess::local_socket::{GenericFilePath, ToFsName as _};

        self.platform_name.as_str().to_fs_name::<GenericFilePath>()
    }
}

pub(crate) struct BridgeIpcServer {
    listener: Listener,
    installation_id: IpcInstallationId,
    shared_secret: IpcSharedSecret,
    server_instance_id: Uuid,
    status_reader: Arc<dyn BridgeStatusReader>,
}

impl BridgeIpcServer {
    /// 创建仅当前登录会话可访问的本地 IPC 服务。
    ///
    /// # Errors
    ///
    /// 端点名称、平台 ACL 或本地监听器创建失败时返回稳定错误。
    pub(crate) fn bind(
        paths: &BridgeRuntimePaths,
        installation_id: IpcInstallationId,
        shared_secret: IpcSharedSecret,
        status_reader: Arc<dyn BridgeStatusReader>,
    ) -> BridgeIpcResult<Self> {
        let endpoint = BridgeIpcEndpoint::from_installation(paths, &installation_id);
        let options = private_listener_options(&endpoint)?;
        let listener = options
            .create_tokio()
            .map_err(|_| BridgeIpcFailure::new(BridgeIpcFailureKind::Bind))?;
        Ok(Self {
            listener,
            installation_id,
            shared_secret,
            server_instance_id: Uuid::now_v7(),
            status_reader,
        })
    }

    /// 接收并隔离多个本地调用方，直到收到关闭信号。
    ///
    /// # Errors
    ///
    /// 监听器失效或连接任务崩溃时返回错误；单个恶意客户端只终止自身会话。
    pub(crate) async fn run(self, mut shutdown: watch::Receiver<bool>) -> BridgeIpcResult<()> {
        let context = Arc::new(BridgeIpcContext {
            installation_id: self.installation_id,
            shared_secret: self.shared_secret,
            server_instance_id: self.server_instance_id,
            status_reader: self.status_reader,
        });
        let mut connections = JoinSet::new();

        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow_and_update() {
                        break;
                    }
                }
                accepted = self.listener.accept() => {
                    let stream = accepted
                        .map_err(|_| BridgeIpcFailure::new(BridgeIpcFailureKind::Accept))?;
                    let context = context.clone();
                    connections.spawn(async move { handle_connection(stream, &context).await });
                }
                completed = connections.join_next(), if !connections.is_empty() => {
                    inspect_connection_result(completed.as_ref())?;
                }
            }
        }

        connections.abort_all();
        while let Some(completed) = connections.join_next().await {
            match completed {
                Err(error) if error.is_cancelled() => {}
                other => inspect_connection_result(Some(&other))?,
            }
        }
        Ok(())
    }
}

fn inspect_connection_result(
    completed: Option<&Result<BridgeIpcResult<()>, tokio::task::JoinError>>,
) -> BridgeIpcResult<()> {
    match completed {
        Some(Ok(Ok(()))) | None => Ok(()),
        Some(Ok(Err(failure))) => {
            tracing::warn!(
                event = "bridge_ipc_session_closed",
                code = failure.code(),
                "本地 IPC 会话被拒绝或中断"
            );
            Ok(())
        }
        Some(Err(_)) => Err(BridgeIpcFailure::new(BridgeIpcFailureKind::Internal)),
    }
}

struct BridgeIpcContext {
    installation_id: IpcInstallationId,
    shared_secret: IpcSharedSecret,
    server_instance_id: Uuid,
    status_reader: Arc<dyn BridgeStatusReader>,
}

async fn handle_connection<S>(mut stream: S, context: &BridgeIpcContext) -> BridgeIpcResult<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let agreement = timeout(
        IPC_HANDSHAKE_TIMEOUT,
        authenticate_connection(&mut stream, context),
    )
    .await
    .map_err(|_| BridgeIpcFailure::new(BridgeIpcFailureKind::Timeout))??;
    handle_requests(&mut stream, context, &agreement).await
}

async fn authenticate_connection<S>(
    stream: &mut S,
    context: &BridgeIpcContext,
) -> BridgeIpcResult<IpcHandshakeAgreement>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let hello = IpcFrameCodec::read(&mut *stream)
        .await
        .map_err(BridgeIpcFailure::protocol)?;
    let (installation_id, offer) = match client_offer_from_frame(&hello) {
        Ok(value) => value,
        Err(failure) => {
            send_handshake_error(&mut *stream, failure.kind()).await?;
            return Err(BridgeIpcFailure::new(BridgeIpcFailureKind::Handshake));
        }
    };
    if installation_id != context.installation_id {
        send_error(
            &mut *stream,
            None,
            "bridge.ipc.installation_rejected",
            IpcErrorCategory::Authentication,
            false,
        )
        .await?;
        return Err(BridgeIpcFailure::new(BridgeIpcFailureKind::Authentication));
    }

    let negotiator =
        IpcHandshakeNegotiator::new([IpcProtocolVersion::V1_0], FoundationIpcScopePolicy)
            .map_err(|_| BridgeIpcFailure::new(BridgeIpcFailureKind::Internal))?;
    let agreement = match negotiator.negotiate(&offer) {
        Ok(value) => value,
        Err(failure) => {
            send_handshake_error(&mut *stream, failure.kind()).await?;
            return Err(BridgeIpcFailure::new(BridgeIpcFailureKind::Handshake));
        }
    };

    let challenge_id = Uuid::now_v7();
    let challenge = random_challenge()?;
    IpcFrameCodec::write(
        &mut *stream,
        &IpcFrame::ServerChallenge {
            challenge_id,
            challenge: URL_SAFE_NO_PAD.encode(challenge.as_bytes()),
            selected_version: IpcVersion::from(agreement.selected_version()),
        },
    )
    .await
    .map_err(BridgeIpcFailure::protocol)?;

    let proof_frame = IpcFrameCodec::read(&mut *stream)
        .await
        .map_err(BridgeIpcFailure::protocol)?;
    let proof = match decode_proof(&proof_frame, challenge_id) {
        Ok(value) => value,
        Err(failure) => {
            send_error(
                &mut *stream,
                None,
                "bridge.ipc.authentication_rejected",
                IpcErrorCategory::Authentication,
                false,
            )
            .await?;
            return Err(failure);
        }
    };
    if !verify_challenge_proof(
        &context.shared_secret,
        challenge_id,
        challenge,
        &installation_id,
        &offer,
        &agreement,
        proof,
    ) {
        send_error(
            &mut *stream,
            None,
            "bridge.ipc.authentication_rejected",
            IpcErrorCategory::Authentication,
            false,
        )
        .await?;
        return Err(BridgeIpcFailure::new(BridgeIpcFailureKind::Authentication));
    }

    IpcFrameCodec::write(
        &mut *stream,
        &server_agreement_frame(context.server_instance_id, &agreement),
    )
    .await
    .map_err(BridgeIpcFailure::protocol)?;

    Ok(agreement)
}

async fn handle_requests<S>(
    stream: &mut S,
    context: &BridgeIpcContext,
    agreement: &IpcHandshakeAgreement,
) -> BridgeIpcResult<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        let frame = match IpcFrameCodec::read(&mut *stream).await {
            Ok(frame) => frame,
            Err(failure) if failure.kind() == IpcProtocolFailureKind::Io => return Ok(()),
            Err(failure) => return Err(BridgeIpcFailure::protocol(failure)),
        };
        let IpcFrame::Request {
            correlation_id,
            method,
        } = frame
        else {
            send_error(
                &mut *stream,
                None,
                "bridge.ipc.request_invalid",
                IpcErrorCategory::Validation,
                false,
            )
            .await?;
            return Err(BridgeIpcFailure::new(BridgeIpcFailureKind::Protocol));
        };

        match method {
            IpcMethod::BridgeStatus
                if agreement
                    .granted_scopes()
                    .contains(&IpcScope::BridgeStatusRead) =>
            {
                let status = context.status_reader.read_status();
                IpcFrameCodec::write(
                    &mut *stream,
                    &IpcFrame::Response {
                        correlation_id,
                        result: IpcResponse::BridgeStatus {
                            state: status.state,
                            started_at_unix_ms: status.started_at_unix_ms,
                        },
                    },
                )
                .await
                .map_err(BridgeIpcFailure::protocol)?;
                tracing::debug!(
                    event = "bridge_ipc_request",
                    method = "bridge_status",
                    result = "ok",
                    "本地 IPC 请求完成"
                );
            }
            IpcMethod::BridgeStatus => {
                send_error(
                    &mut *stream,
                    Some(correlation_id),
                    "bridge.ipc.scope_denied",
                    IpcErrorCategory::Authorization,
                    false,
                )
                .await?;
            }
        }
    }
}

fn random_challenge() -> BridgeIpcResult<IpcChallenge> {
    let mut bytes = [0_u8; IPC_SECRET_BYTES];
    getrandom::fill(&mut bytes)
        .map_err(|_| BridgeIpcFailure::new(BridgeIpcFailureKind::Entropy))?;
    Ok(IpcChallenge::new(bytes))
}

fn decode_proof(
    frame: &IpcFrame,
    expected_challenge_id: Uuid,
) -> BridgeIpcResult<IpcChallengeProof> {
    let IpcFrame::ClientProof {
        challenge_id,
        proof,
    } = frame
    else {
        return Err(BridgeIpcFailure::new(BridgeIpcFailureKind::Authentication));
    };
    if *challenge_id != expected_challenge_id {
        return Err(BridgeIpcFailure::new(BridgeIpcFailureKind::Authentication));
    }
    let bytes: [u8; IPC_SECRET_BYTES] = URL_SAFE_NO_PAD
        .decode(proof)
        .map_err(|_| BridgeIpcFailure::new(BridgeIpcFailureKind::Authentication))?
        .try_into()
        .map_err(|_| BridgeIpcFailure::new(BridgeIpcFailureKind::Authentication))?;
    Ok(IpcChallengeProof::new(bytes))
}

async fn send_handshake_error<S>(
    stream: &mut S,
    kind: IpcHandshakeFailureKind,
) -> BridgeIpcResult<()>
where
    S: AsyncWrite + Unpin,
{
    let (code, category) = match kind {
        IpcHandshakeFailureKind::InvalidConfiguration => (
            "bridge.ipc.configuration_invalid",
            IpcErrorCategory::Internal,
        ),
        IpcHandshakeFailureKind::InvalidOffer => {
            ("bridge.ipc.hello_invalid", IpcErrorCategory::Validation)
        }
        IpcHandshakeFailureKind::AuthenticationRejected => (
            "bridge.ipc.authentication_rejected",
            IpcErrorCategory::Authentication,
        ),
        IpcHandshakeFailureKind::IncompatibleVersion => (
            "bridge.ipc.version_incompatible",
            IpcErrorCategory::IncompatibleVersion,
        ),
        IpcHandshakeFailureKind::ScopeDenied => {
            ("bridge.ipc.scope_denied", IpcErrorCategory::Authorization)
        }
    };
    send_error(stream, None, code, category, false).await
}

async fn send_error<S>(
    stream: &mut S,
    correlation_id: Option<Uuid>,
    code: &str,
    category: IpcErrorCategory,
    retryable: bool,
) -> BridgeIpcResult<()>
where
    S: AsyncWrite + Unpin,
{
    IpcFrameCodec::write(
        stream,
        &IpcFrame::Error {
            correlation_id,
            code: code.to_owned(),
            category,
            retryable,
            details: BTreeMap::new(),
        },
    )
    .await
    .map_err(BridgeIpcFailure::protocol)
}

#[cfg(windows)]
fn private_listener_options(endpoint: &BridgeIpcEndpoint) -> BridgeIpcResult<ListenerOptions<'_>> {
    use interprocess::os::windows::local_socket::ListenerOptionsExt as _;
    use interprocess::os::windows::security_descriptor::SecurityDescriptor;
    use widestring::U16CString;
    use win_security_identifier::{GetCurrentSid as _, SecurityIdentifier};

    let session_sid = SecurityIdentifier::get_current_logon_sid()
        .map_err(|_| BridgeIpcFailure::new(BridgeIpcFailureKind::AccessControl))?
        .map_or_else(SecurityIdentifier::get_current_user_sid, Ok)
        .map_err(|_| BridgeIpcFailure::new(BridgeIpcFailureKind::AccessControl))?;
    let sddl = private_windows_sddl(&session_sid.to_string());
    let wide_sddl = U16CString::from_str(&sddl)
        .map_err(|_| BridgeIpcFailure::new(BridgeIpcFailureKind::AccessControl))?;
    let descriptor = SecurityDescriptor::deserialize(&wide_sddl)
        .map_err(|_| BridgeIpcFailure::new(BridgeIpcFailureKind::AccessControl))?;
    Ok(ListenerOptions::new()
        .name(
            endpoint
                .to_name()
                .map_err(|_| BridgeIpcFailure::new(BridgeIpcFailureKind::InvalidEndpoint))?,
        )
        .reclaim_name(true)
        .try_overwrite(false)
        .security_descriptor(descriptor))
}

#[cfg(windows)]
fn private_windows_sddl(session_sid: &str) -> String {
    format!("D:P(A;;GA;;;{session_sid})(A;;GA;;;SY)")
}

#[cfg(unix)]
fn private_listener_options(endpoint: &BridgeIpcEndpoint) -> BridgeIpcResult<ListenerOptions<'_>> {
    use interprocess::local_socket::ListenerOptionsExt as _;

    Ok(ListenerOptions::new()
        .name(
            endpoint
                .to_name()
                .map_err(|_| BridgeIpcFailure::new(BridgeIpcFailureKind::InvalidEndpoint))?,
        )
        .reclaim_name(true)
        .try_overwrite(false)
        .mode(0o600))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BridgeIpcFailureKind {
    InvalidEndpoint,
    AccessControl,
    Bind,
    Accept,
    Protocol,
    Handshake,
    Authentication,
    Entropy,
    Timeout,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BridgeIpcFailure {
    kind: BridgeIpcFailureKind,
}

impl BridgeIpcFailure {
    const fn new(kind: BridgeIpcFailureKind) -> Self {
        Self { kind }
    }

    const fn protocol(_failure: agent_room_bridge_ipc::IpcProtocolFailure) -> Self {
        Self::new(BridgeIpcFailureKind::Protocol)
    }

    pub(crate) const fn kind(self) -> BridgeIpcFailureKind {
        self.kind
    }

    const fn code(self) -> &'static str {
        match self.kind {
            BridgeIpcFailureKind::InvalidEndpoint => "bridge.ipc.endpoint_invalid",
            BridgeIpcFailureKind::AccessControl => "bridge.ipc.access_control_failed",
            BridgeIpcFailureKind::Bind => "bridge.ipc.bind_failed",
            BridgeIpcFailureKind::Accept => "bridge.ipc.accept_failed",
            BridgeIpcFailureKind::Protocol => "bridge.ipc.protocol_failed",
            BridgeIpcFailureKind::Handshake => "bridge.ipc.handshake_failed",
            BridgeIpcFailureKind::Authentication => "bridge.ipc.authentication_rejected",
            BridgeIpcFailureKind::Entropy => "bridge.ipc.entropy_unavailable",
            BridgeIpcFailureKind::Timeout => "bridge.ipc.handshake_timeout",
            BridgeIpcFailureKind::Internal => "bridge.ipc.internal",
        }
    }
}

pub(crate) type BridgeIpcResult<T> = Result<T, BridgeIpcFailure>;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use agent_room_bridge_core::ipc::{
        FoundationIpcScopePolicy, IpcCallerKind, IpcHandshakeNegotiator, IpcHandshakeOffer,
        IpcInstallationId, IpcProtocolVersion, IpcScope,
    };
    use agent_room_bridge_ipc::{
        IpcCaller, IpcChallenge, IpcErrorCategory, IpcFrame, IpcFrameCodec, IpcMethod, IpcResponse,
        IpcScopeName, IpcSharedSecret, IpcVersion, create_challenge_proof,
    };
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use tempfile::tempdir;
    use tokio::io::duplex;
    use uuid::Uuid;

    use crate::runtime_files::BridgeRuntimePaths;

    use super::{
        BridgeIpcContext, BridgeIpcFailureKind, BridgeIpcServer, BridgeStatusReader,
        BridgeStatusSnapshot, handle_connection,
    };

    struct 固定状态;

    impl BridgeStatusReader for 固定状态 {
        fn read_status(&self) -> BridgeStatusSnapshot {
            BridgeStatusSnapshot {
                state: agent_room_bridge_ipc::IpcBridgeState::Ready,
                started_at_unix_ms: 1_000,
            }
        }
    }

    #[tokio::test]
    async fn 完成挑战后只能调用已授权的闭合方法() {
        let installation_id = IpcInstallationId::new("install_1").expect("安装标识有效");
        let secret = IpcSharedSecret::new([7; 32]);
        let context = BridgeIpcContext {
            installation_id: installation_id.clone(),
            shared_secret: secret.clone(),
            server_instance_id: Uuid::from_u128(99),
            status_reader: Arc::new(固定状态),
        };
        let (mut client, server) = duplex(8 * 1_024);
        let server_task = tokio::spawn(async move { handle_connection(server, &context).await });
        let offer = IpcHandshakeOffer::new(
            IpcCallerKind::DiagnosticCli,
            [IpcProtocolVersion::V1_0],
            [IpcScope::BridgeStatusRead],
        )
        .expect("测试提议有效");
        IpcFrameCodec::write(
            &mut client,
            &IpcFrame::ClientHello {
                installation_id: installation_id.as_str().to_owned(),
                caller: IpcCaller::DiagnosticCli,
                supported_versions: vec![IpcVersion { major: 1, minor: 0 }],
                requested_scopes: vec![IpcScopeName::BridgeStatusRead],
            },
        )
        .await
        .expect("客户端问候可发送");
        let (challenge_id, challenge) = read_challenge(&mut client).await;
        let agreement =
            IpcHandshakeNegotiator::new([IpcProtocolVersion::V1_0], FoundationIpcScopePolicy)
                .expect("测试协商器有效")
                .negotiate(&offer)
                .expect("测试提议可协商");
        let proof = create_challenge_proof(
            &secret,
            challenge_id,
            challenge,
            &installation_id,
            &offer,
            &agreement,
        )
        .expect("测试证明可创建");
        IpcFrameCodec::write(
            &mut client,
            &IpcFrame::ClientProof {
                challenge_id,
                proof: URL_SAFE_NO_PAD.encode(proof.as_bytes()),
            },
        )
        .await
        .expect("客户端证明可发送");
        assert!(matches!(
            IpcFrameCodec::read(&mut client)
                .await
                .expect("服务端就绪帧可读取"),
            IpcFrame::ServerReady { .. }
        ));

        let correlation_id = Uuid::from_u128(1);
        IpcFrameCodec::write(
            &mut client,
            &IpcFrame::Request {
                correlation_id,
                method: IpcMethod::BridgeStatus,
            },
        )
        .await
        .expect("状态请求可发送");
        assert_eq!(
            IpcFrameCodec::read(&mut client)
                .await
                .expect("状态响应可读取"),
            IpcFrame::Response {
                correlation_id,
                result: IpcResponse::BridgeStatus {
                    state: agent_room_bridge_ipc::IpcBridgeState::Ready,
                    started_at_unix_ms: 1_000,
                },
            }
        );
        drop(client);
        server_task
            .await
            .expect("服务端任务未崩溃")
            .expect("客户端正常断开");
    }

    #[tokio::test]
    async fn 错误挑战证明在任何请求分派前拒绝() {
        let installation_id = IpcInstallationId::new("install_2").expect("安装标识有效");
        let context = BridgeIpcContext {
            installation_id: installation_id.clone(),
            shared_secret: IpcSharedSecret::new([7; 32]),
            server_instance_id: Uuid::from_u128(99),
            status_reader: Arc::new(固定状态),
        };
        let (mut client, server) = duplex(8 * 1_024);
        let server_task = tokio::spawn(async move { handle_connection(server, &context).await });
        IpcFrameCodec::write(
            &mut client,
            &IpcFrame::ClientHello {
                installation_id: installation_id.as_str().to_owned(),
                caller: IpcCaller::CodexPlugin,
                supported_versions: vec![IpcVersion { major: 1, minor: 0 }],
                requested_scopes: vec![IpcScopeName::BridgeStatusRead],
            },
        )
        .await
        .expect("客户端问候可发送");
        let (challenge_id, _) = read_challenge(&mut client).await;
        IpcFrameCodec::write(
            &mut client,
            &IpcFrame::ClientProof {
                challenge_id,
                proof: URL_SAFE_NO_PAD.encode([8_u8; 32]),
            },
        )
        .await
        .expect("错误证明可发送");

        assert!(matches!(
            IpcFrameCodec::read(&mut client)
                .await
                .expect("拒绝帧可读取"),
            IpcFrame::Error {
                category: IpcErrorCategory::Authentication,
                ..
            }
        ));
        assert_eq!(
            server_task
                .await
                .expect("服务端任务未崩溃")
                .expect_err("错误证明必须失败")
                .kind(),
            BridgeIpcFailureKind::Authentication
        );
    }

    #[tokio::test]
    async fn 当前平台私有端点可以创建且持有独占名称() {
        let temporary = tempdir().expect("测试目录可创建");
        let paths = BridgeRuntimePaths::new(temporary.path().join("bridge"));
        paths.prepare().expect("运行目录可准备");
        let installation_id =
            IpcInstallationId::new(format!("test-{}", Uuid::now_v7())).expect("测试安装标识有效");
        let first = BridgeIpcServer::bind(
            &paths,
            installation_id.clone(),
            IpcSharedSecret::new([1; 32]),
            Arc::new(固定状态),
        )
        .expect("首个私有端点可创建");

        let Err(second) = BridgeIpcServer::bind(
            &paths,
            installation_id,
            IpcSharedSecret::new([1; 32]),
            Arc::new(固定状态),
        ) else {
            panic!("相同端点不能被第二个服务占用");
        };

        assert_eq!(second.kind(), BridgeIpcFailureKind::Bind);
        drop(first);
    }

    #[cfg(windows)]
    #[test]
    fn windows_acl_不包含广域主体() {
        let sddl = super::private_windows_sddl("S-1-5-5-1-2");

        assert!(sddl.contains("S-1-5-5-1-2"));
        assert!(sddl.contains(";;;SY"));
        assert!(!sddl.contains(";;;WD"));
        assert!(!sddl.contains(";;;AN"));
        assert!(!sddl.contains(";;;AU"));
        assert!(!sddl.contains(";;;BU"));
    }

    async fn read_challenge<S>(stream: &mut S) -> (Uuid, IpcChallenge)
    where
        S: tokio::io::AsyncRead + Unpin,
    {
        let IpcFrame::ServerChallenge {
            challenge_id,
            challenge,
            ..
        } = IpcFrameCodec::read(stream).await.expect("挑战帧可读取")
        else {
            panic!("必须返回挑战帧");
        };
        let bytes: [u8; 32] = URL_SAFE_NO_PAD
            .decode(challenge)
            .expect("挑战编码有效")
            .try_into()
            .expect("挑战长度有效");
        (challenge_id, IpcChallenge::new(bytes))
    }
}
