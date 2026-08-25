use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use agent_room_bridge_core::ipc::{
    FoundationIpcScopePolicy, IpcHandshakeAgreement, IpcHandshakeFailureKind,
    IpcHandshakeNegotiator, IpcInstallationId, IpcProtocolVersion,
};
use agent_room_bridge_core::messages::{MessageTimelineQueryRepository, OpenMessageContentService};
use agent_room_bridge_ipc::{
    IpcBridgeState, IpcChallenge, IpcChallengeProof, IpcErrorCategory, IpcFrame, IpcFrameCodec,
    IpcMethod, IpcProtocolFailureKind, IpcResponse, IpcScopeName, IpcSharedSecret, IpcVersion,
    client_offer_from_frame, server_agreement_frame, verify_challenge_proof,
};
use agent_room_bridge_local_adapter::LocalIpcEndpoint;
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

mod agent_runtime;

use agent_runtime::AgentRuntimeIpcFacade;
pub(crate) use agent_runtime::{BridgeAgentRuntimeReader, BridgeAgentRuntimeSnapshot};

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

type BridgeIpcDispatchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<IpcResponse, BridgeIpcDispatchFailure>> + Send + 'a>>;

pub(crate) trait BridgeIpcRequestHandler: Send + Sync {
    fn dispatch(&self, method: IpcMethod) -> BridgeIpcDispatchFuture<'_>;
}

pub(crate) struct FoundationBridgeIpcRequestHandler {
    status_reader: Arc<dyn BridgeStatusReader>,
    agent_runtime: Option<AgentRuntimeIpcFacade>,
}

impl FoundationBridgeIpcRequestHandler {
    pub(crate) fn new(status_reader: Arc<dyn BridgeStatusReader>) -> Self {
        Self {
            status_reader,
            agent_runtime: None,
        }
    }

    pub(crate) fn with_agent_runtime(
        status_reader: Arc<dyn BridgeStatusReader>,
        agent_runtime_reader: Arc<dyn BridgeAgentRuntimeReader>,
        previews: Arc<dyn MessageTimelineQueryRepository>,
        content: Arc<OpenMessageContentService>,
    ) -> Self {
        Self {
            status_reader: status_reader.clone(),
            agent_runtime: Some(AgentRuntimeIpcFacade::new(
                status_reader,
                agent_runtime_reader,
                previews,
                content,
            )),
        }
    }

    fn agent_runtime(&self) -> Result<&AgentRuntimeIpcFacade, BridgeIpcDispatchFailure> {
        self.agent_runtime
            .as_ref()
            .ok_or_else(agent_runtime_unavailable)
    }
}

impl BridgeIpcRequestHandler for FoundationBridgeIpcRequestHandler {
    fn dispatch(&self, method: IpcMethod) -> BridgeIpcDispatchFuture<'_> {
        Box::pin(async move {
            match method {
                IpcMethod::BridgeStatus => {
                    let status = self.status_reader.read_status();
                    Ok(IpcResponse::BridgeStatus {
                        state: status.state,
                        started_at_unix_ms: status.started_at_unix_ms,
                    })
                }
                IpcMethod::GetSelf => self.agent_runtime()?.get_self(),
                IpcMethod::ListPreviews(request) => {
                    self.agent_runtime()?.list_previews(request).await
                }
                IpcMethod::PublishStatus(request) => {
                    self.agent_runtime()?.publish_status(request).await
                }
                IpcMethod::OpenContent(request) => {
                    self.agent_runtime()?.open_content(request).await
                }
                IpcMethod::SendMessage(request) => {
                    self.agent_runtime()?.send_message(request).await
                }
                IpcMethod::ConsumeHandoff(request) => {
                    self.agent_runtime()?.consume_handoff(request).await
                }
                IpcMethod::DeclineHandoff(request) => {
                    self.agent_runtime()?.decline_handoff(request).await
                }
                IpcMethod::GetPresence(_) => Err(agent_runtime_unavailable()),
            }
        })
    }
}

const fn agent_runtime_unavailable() -> BridgeIpcDispatchFailure {
    BridgeIpcDispatchFailure::new(
        "bridge.agent_runtime_unavailable",
        IpcErrorCategory::DependencyUnavailable,
        true,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BridgeIpcDispatchFailure {
    code: &'static str,
    category: IpcErrorCategory,
    retryable: bool,
}

impl BridgeIpcDispatchFailure {
    pub(crate) const fn new(
        code: &'static str,
        category: IpcErrorCategory,
        retryable: bool,
    ) -> Self {
        Self {
            code,
            category,
            retryable,
        }
    }
}

pub(crate) struct BridgeIpcServer {
    listener: Listener,
    installation_id: IpcInstallationId,
    shared_secret: IpcSharedSecret,
    server_instance_id: Uuid,
    request_handler: Arc<dyn BridgeIpcRequestHandler>,
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
        request_handler: Arc<dyn BridgeIpcRequestHandler>,
    ) -> BridgeIpcResult<Self> {
        let endpoint = LocalIpcEndpoint::from_installation(paths.runtime_root(), &installation_id);
        let options = private_listener_options(&endpoint)?;
        let listener = options
            .create_tokio()
            .map_err(|_| BridgeIpcFailure::new(BridgeIpcFailureKind::Bind))?;
        Ok(Self {
            listener,
            installation_id,
            shared_secret,
            server_instance_id: Uuid::now_v7(),
            request_handler,
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
            request_handler: self.request_handler,
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
    request_handler: Arc<dyn BridgeIpcRequestHandler>,
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
            granted_scopes: agreement
                .granted_scopes()
                .iter()
                .copied()
                .map(IpcScopeName::from)
                .collect(),
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

        let method_name = method.name();
        if let Err(failure) = method.validate() {
            send_error(
                &mut *stream,
                Some(correlation_id),
                failure.code(),
                IpcErrorCategory::Validation,
                false,
            )
            .await?;
            continue;
        }
        if !agreement
            .granted_scopes()
            .contains(&method.required_scope())
        {
            send_error(
                &mut *stream,
                Some(correlation_id),
                "bridge.ipc.scope_denied",
                IpcErrorCategory::Authorization,
                false,
            )
            .await?;
            continue;
        }

        match context.request_handler.dispatch(method).await {
            Ok(result) => {
                IpcFrameCodec::write(
                    &mut *stream,
                    &IpcFrame::Response {
                        correlation_id,
                        result,
                    },
                )
                .await
                .map_err(BridgeIpcFailure::protocol)?;
                tracing::debug!(
                    event = "bridge_ipc_request",
                    method = method_name,
                    result = "ok",
                    "本地 IPC 请求完成"
                );
            }
            Err(failure) => {
                send_error(
                    &mut *stream,
                    Some(correlation_id),
                    failure.code,
                    failure.category,
                    failure.retryable,
                )
                .await?;
                tracing::warn!(
                    event = "bridge_ipc_request",
                    method = method_name,
                    result = "error",
                    code = failure.code,
                    "本地 IPC 请求失败"
                );
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
fn private_listener_options(endpoint: &LocalIpcEndpoint) -> BridgeIpcResult<ListenerOptions<'_>> {
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
fn private_listener_options(endpoint: &LocalIpcEndpoint) -> BridgeIpcResult<ListenerOptions<'_>> {
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
    use std::sync::{Arc, Mutex};

    use agent_room_application::ports::{
        Clock, DeviceSignature, MatrixAcceptedEvent, MatrixEvent, MatrixEventId, MatrixResult,
        MatrixRoomId, MatrixStateEvent, PortFuture,
    };
    use agent_room_bridge_core::ipc::{
        FoundationIpcScopePolicy, IpcCallerKind, IpcHandshakeNegotiator, IpcHandshakeOffer,
        IpcInstallationId, IpcProtocolVersion, IpcScope,
    };
    use agent_room_bridge_core::{
        agent_identity::BridgeAgentIdentity,
        handoffs::{
            ConsumedHandoffContext, HandoffConsumptionOutcome, HandoffReceiptDelivery,
            HandoffReceptionFailure, HandoffResolutionOutcome,
        },
        messages::{
            DownloadedMessageContent, MessageContentBindRequest, MessageContentFailure,
            MessageContentGateway, MessageContentReadFailure, MessageContentReadFailureKind,
            MessageContentReadGateway, MessageContentReadRequest, MessageContentRecord,
            MessageContentRedactRequest, MessageContentSourceQuery, MessageContentUploadRequest,
            MessageEventPublisher, MessagePreviewPage, MessagePreviewQuery,
            MessagePublicationDependencies, MessagePublicationService, MessageTimelineQueryFailure,
            MessageTimelineQueryRepository, OpenMessageContentDependencies,
            OpenMessageContentService, ProjectedMessageActor, ProjectedMessagePreview,
        },
        ports::{
            AgentStatusStatePublisher, BridgeCredentialResult, DeviceSigningIdentity,
            StatusEventIdentifierFactory,
        },
        status::{
            AgentStatusLeasePolicy, AgentStatusPublicationDependencies,
            AgentStatusPublicationService, AgentStatusRoomTarget, HostAgentState,
        },
    };
    use agent_room_bridge_ipc::{
        IpcCaller, IpcChallenge, IpcErrorCategory, IpcFrame, IpcFrameCodec, IpcHandoffRequest,
        IpcHandoffStatus, IpcMessageProvenance, IpcMessageSensitivity, IpcMethod,
        IpcOpenContentRequest, IpcPublishStatusRequest, IpcResponse, IpcScopeName,
        IpcSendMessageRequest, IpcSharedSecret, IpcSubmissionState, IpcVersion, IpcWorkStatus,
        create_challenge_proof,
    };
    use agent_room_bridge_storage_adapter::SqliteMessageSubmissionRepository;
    use agent_room_domain::{
        agent_status::AgentStatusVisibility,
        content::{ContentByteLength, ContentMediaType, Sha256Digest},
        devices::DevicePublicSigningKey,
        handoff::{
            ContextHandoff, ContextHandoffFields, HandoffContentReference, HandoffPermission,
            HandoffPermissions, HandoffPurpose, HandoffSource, HandoffSourceActor,
            HandoffSourceEventId,
        },
        ids::{AgentId, AgentInstanceId, ContentId, HandoffId, MessageId, PrincipalId},
        messages::{
            MessageContentReference, MessagePreview, MessageProvenance, MessageRiskFlag,
            MessageRiskFlags, MessageSensitivity, MessageSummary, MessageTitle,
        },
        rooms::MatrixRoomReference,
        time::{DurationMillis, UtcMillis},
    };
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use tempfile::tempdir;
    use tokio::io::duplex;
    use uuid::Uuid;

    use crate::{agent_status::AgentStatusPublicationHandle, runtime_files::BridgeRuntimePaths};

    use super::{
        BridgeAgentRuntimeReader, BridgeAgentRuntimeSnapshot, BridgeIpcContext,
        BridgeIpcFailureKind, BridgeIpcRequestHandler, BridgeIpcServer, BridgeStatusReader,
        BridgeStatusSnapshot, FoundationBridgeIpcRequestHandler,
        agent_runtime::AgentHandoffRuntime, handle_connection,
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

    #[derive(Clone)]
    struct 固定Agent运行时(BridgeAgentRuntimeSnapshot);

    impl BridgeAgentRuntimeReader for 固定Agent运行时 {
        fn read_agent_runtime(&self) -> Option<BridgeAgentRuntimeSnapshot> {
            Some(self.0.clone())
        }
    }

    struct 固定交接运行时 {
        pending: ContextHandoff,
        body: Arc<[u8]>,
    }

    impl AgentHandoffRuntime for 固定交接运行时 {
        fn inspect_pending(
            &self,
            handoff_id: HandoffId,
        ) -> PortFuture<'_, Result<ContextHandoff, HandoffReceptionFailure>> {
            assert_eq!(handoff_id, self.pending.fields().id);
            let pending = self.pending.clone();
            Box::pin(async move { Ok(pending) })
        }

        fn consume(
            &self,
            handoff_id: HandoffId,
        ) -> PortFuture<'_, Result<HandoffConsumptionOutcome, HandoffReceptionFailure>> {
            assert_eq!(handoff_id, self.pending.fields().id);
            let mut consumed = self.pending.clone();
            consumed
                .consume(UtcMillis::new(1_100).expect("消费时间有效"))
                .expect("已送达交接可消费");
            let body = self.body.clone();
            Box::pin(async move {
                Ok(HandoffConsumptionOutcome::new(
                    ConsumedHandoffContext::new(consumed, body),
                    HandoffReceiptDelivery::Confirmed,
                ))
            })
        }

        fn decline(
            &self,
            handoff_id: HandoffId,
        ) -> PortFuture<'_, Result<HandoffResolutionOutcome, HandoffReceptionFailure>> {
            assert_eq!(handoff_id, self.pending.fields().id);
            Box::pin(async move {
                Ok(HandoffResolutionOutcome::new(
                    handoff_id,
                    agent_room_domain::handoff::HandoffStatus::Declined,
                    HandoffReceiptDelivery::Confirmed,
                ))
            })
        }
    }

    #[derive(Default)]
    struct 记录预览查询(Mutex<Vec<MessagePreviewQuery>>);

    impl MessageTimelineQueryRepository for 记录预览查询 {
        fn list_previews<'a>(
            &'a self,
            query: &'a MessagePreviewQuery,
        ) -> PortFuture<'a, Result<MessagePreviewPage, MessageTimelineQueryFailure>> {
            Box::pin(async move {
                self.0.lock().expect("查询记录锁可用").push(query.clone());
                Ok(MessagePreviewPage::new(Vec::new(), None))
            })
        }

        fn find_content_source<'a>(
            &'a self,
            _query: &'a MessageContentSourceQuery,
        ) -> PortFuture<'a, Result<Option<ProjectedMessagePreview>, MessageTimelineQueryFailure>>
        {
            Box::pin(async { Ok(None) })
        }
    }

    struct 固定正文投影(ProjectedMessagePreview);

    impl MessageTimelineQueryRepository for 固定正文投影 {
        fn list_previews<'a>(
            &'a self,
            _query: &'a MessagePreviewQuery,
        ) -> PortFuture<'a, Result<MessagePreviewPage, MessageTimelineQueryFailure>> {
            Box::pin(async { Ok(MessagePreviewPage::new(Vec::new(), None)) })
        }

        fn find_content_source<'a>(
            &'a self,
            query: &'a MessageContentSourceQuery,
        ) -> PortFuture<'a, Result<Option<ProjectedMessagePreview>, MessageTimelineQueryFailure>>
        {
            Box::pin(async move {
                Ok((query.room_id() == &self.0.room_id
                    && query.content_id() == self.0.content.content_id())
                .then(|| self.0.clone()))
            })
        }
    }

    struct 拒绝正文网关;

    impl MessageContentReadGateway for 拒绝正文网关 {
        fn open<'a>(
            &'a self,
            _request: &'a MessageContentReadRequest,
        ) -> PortFuture<'a, Result<DownloadedMessageContent, MessageContentReadFailure>> {
            Box::pin(async {
                Err(MessageContentReadFailure::new(
                    MessageContentReadFailureKind::NotFound,
                ))
            })
        }
    }

    struct 固定正文网关(DownloadedMessageContent);

    impl MessageContentReadGateway for 固定正文网关 {
        fn open<'a>(
            &'a self,
            _request: &'a MessageContentReadRequest,
        ) -> PortFuture<'a, Result<DownloadedMessageContent, MessageContentReadFailure>> {
            Box::pin(async { Ok(self.0.clone()) })
        }
    }

    struct 固定时钟;

    impl Clock for 固定时钟 {
        fn now(&self) -> UtcMillis {
            UtcMillis::new(1_000).expect("测试时间有效")
        }
    }

    struct 测试签名身份;

    impl DeviceSigningIdentity for 测试签名身份 {
        fn public_key(&self) -> BridgeCredentialResult<DevicePublicSigningKey> {
            Ok(DevicePublicSigningKey::new(vec![8; 32]).expect("测试公钥有效"))
        }

        fn sign(&self, _message: &[u8]) -> BridgeCredentialResult<DeviceSignature> {
            Ok(DeviceSignature::new(vec![9; 64]).expect("测试签名有效"))
        }
    }

    #[derive(Default)]
    struct 记录状态发布器(Mutex<Vec<MatrixStateEvent>>);

    impl AgentStatusStatePublisher for 记录状态发布器 {
        fn publish<'a>(
            &'a self,
            _room_id: &'a MatrixRoomId,
            event: &'a MatrixStateEvent,
        ) -> PortFuture<'a, MatrixResult<MatrixEventId>> {
            self.0.lock().expect("状态事件锁可用").push(event.clone());
            Box::pin(async {
                Ok(MatrixEventId::new("$status:matrix.test").expect("事件标识有效"))
            })
        }
    }

    struct 版本七状态标识;

    impl StatusEventIdentifierFactory for 版本七状态标识 {
        fn event_id(&self) -> Uuid {
            Uuid::now_v7()
        }

        fn correlation_id(&self) -> Uuid {
            Uuid::now_v7()
        }
    }

    #[derive(Default)]
    struct 记录消息发布器(Mutex<Vec<MatrixEvent>>);

    impl MessageEventPublisher for 记录消息发布器 {
        fn publish<'a>(
            &'a self,
            _room_id: &'a MatrixRoomId,
            event: &'a MatrixEvent,
        ) -> PortFuture<'a, MatrixResult<MatrixAcceptedEvent>> {
            self.0.lock().expect("消息事件锁可用").push(event.clone());
            Box::pin(async move {
                Ok(MatrixAcceptedEvent::new(
                    event.transaction_id().clone(),
                    MatrixEventId::new("$sent:matrix.test").expect("事件标识有效"),
                ))
            })
        }
    }

    #[derive(Default)]
    struct 记录正文写入 {
        uploads: Mutex<Vec<MessageContentUploadRequest>>,
        bindings: Mutex<Vec<MessageContentBindRequest>>,
    }

    impl MessageContentGateway for 记录正文写入 {
        fn upload<'a>(
            &'a self,
            request: &'a MessageContentUploadRequest,
        ) -> PortFuture<'a, Result<MessageContentRecord, MessageContentFailure>> {
            self.uploads
                .lock()
                .expect("正文上传锁可用")
                .push(request.clone());
            Box::pin(async move {
                Ok(MessageContentRecord {
                    content_id: ContentId::from_uuid(request.request_id.as_uuid()),
                    digest: request.digest,
                    byte_length: request.byte_length,
                    media_type: request.media_type.clone(),
                })
            })
        }

        fn bind<'a>(
            &'a self,
            request: &'a MessageContentBindRequest,
        ) -> PortFuture<'a, Result<(), MessageContentFailure>> {
            self.bindings
                .lock()
                .expect("正文绑定锁可用")
                .push(request.clone());
            Box::pin(async { Ok(()) })
        }

        fn redact<'a>(
            &'a self,
            _request: &'a MessageContentRedactRequest,
        ) -> PortFuture<'a, Result<(), MessageContentFailure>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[tokio::test]
    async fn agent_上线后自身摘要与默认大厅预览来自真实运行时() {
        let room_id = MatrixRoomId::new("!lobby:matrix.test").expect("房间标识有效");
        let identity = 测试_agent_身份();
        let previews = Arc::new(记录预览查询::default());
        let handler = FoundationBridgeIpcRequestHandler::with_agent_runtime(
            Arc::new(固定状态),
            Arc::new(固定Agent运行时(BridgeAgentRuntimeSnapshot::new(
                identity,
                "DEVICE-1",
                room_id.clone(),
                ["self.read", "previews.read"],
            ))),
            previews.clone(),
            空正文服务(previews.clone()),
        );

        let summary = handler
            .dispatch(IpcMethod::GetSelf)
            .await
            .expect("自身摘要可读");
        let IpcResponse::SelfSummary { summary } = summary else {
            panic!("必须返回自身摘要");
        };
        assert_eq!(summary.agent.display_name, "Codex Agent");
        assert_eq!(summary.matrix_device_id, "DEVICE-1");
        assert_eq!(summary.granted_capabilities, ["self.read", "previews.read"]);

        let page = handler
            .dispatch(IpcMethod::ListPreviews(
                agent_room_bridge_ipc::IpcListPreviewsRequest {
                    room_id: None,
                    before_event_id: None,
                    limit: 20,
                },
            ))
            .await
            .expect("默认大厅预览可读");
        assert!(matches!(
            page,
            IpcResponse::MessagePreviews {
                previews,
                next_cursor: None
            } if previews.is_empty()
        ));
        let queries = previews.0.lock().expect("查询记录锁可用");
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].room_id(), &room_id);
    }

    #[tokio::test]
    async fn 公共大厅状态发布只暴露粗粒度状态并返回真实租约() {
        let room_id = MatrixRoomId::new("!lobby:matrix.test").expect("房间标识有效");
        let identity = 测试_agent_身份();
        let publisher = Arc::new(记录状态发布器::default());
        let status = Arc::new(AgentStatusPublicationHandle::new(
            AgentStatusPublicationService::new(
                AgentStatusPublicationDependencies {
                    identity: identity.clone(),
                    signer: Arc::new(测试签名身份),
                    publisher: publisher.clone(),
                    identifiers: Arc::new(版本七状态标识),
                    clock: Arc::new(固定时钟),
                },
                AgentStatusLeasePolicy::new(
                    DurationMillis::new(300_000).expect("租约时长有效"),
                    DurationMillis::new(120_000).expect("续租间隔有效"),
                    DurationMillis::new(15_000).expect("续租抖动有效"),
                )
                .expect("租约策略有效"),
            ),
            AgentStatusRoomTarget::new(room_id.clone(), AgentStatusVisibility::Coarse),
            HostAgentState::Available,
        ));
        let previews = Arc::new(记录预览查询::default());
        let handler = FoundationBridgeIpcRequestHandler::with_agent_runtime(
            Arc::new(固定状态),
            Arc::new(固定Agent运行时(
                BridgeAgentRuntimeSnapshot::new(
                    identity,
                    "DEVICE-1",
                    room_id.clone(),
                    ["status.publish"],
                )
                .with_status(status),
            )),
            previews.clone(),
            空正文服务(previews),
        );

        let response = handler
            .dispatch(IpcMethod::PublishStatus(IpcPublishStatusRequest {
                room_id: room_id.as_str().to_owned(),
                status: IpcWorkStatus::Working,
                task_summary: Some("不得进入公共大厅的任务内容".to_owned()),
                progress_basis_points: Some(5_000),
            }))
            .await
            .expect("状态可发布");
        assert!(matches!(
            response,
            IpcResponse::PublishedStatus { publication }
                if publication.room_id == room_id.as_str()
                    && publication.status == IpcWorkStatus::Working
                    && publication.lease_expires_at_unix_ms == 301_000
        ));
        let events = publisher.0.lock().expect("状态事件锁可用");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].content()["status"], "working");
        assert_eq!(events[0].content()["visibility"], "coarse");
        assert!(events[0].content().get("taskSummary").is_none());
        assert!(events[0].content().get("progress").is_none());
    }

    #[tokio::test]
    async fn 正文打开只使用当前大厅的已验证投影并返回完整来源() {
        let room_id = MatrixRoomId::new("!lobby:matrix.test").expect("房间标识有效");
        let content_id = ContentId::from_uuid(
            Uuid::parse_str("01945c1e-7b5a-7c7f-8a28-2de53f56a9a5").expect("正文标识有效"),
        );
        let digest = Sha256Digest::from_bytes([
            0xd6, 0x61, 0xc3, 0xd9, 0x6d, 0x53, 0xeb, 0xc0, 0xca, 0x8a, 0x55, 0xaa, 0xe2, 0x4b,
            0x5d, 0xf4, 0xa4, 0xd1, 0xbf, 0x28, 0xd3, 0x73, 0x37, 0xb9, 0x82, 0xfe, 0x8e, 0xbf,
            0x54, 0x84, 0x6e, 0xeb,
        ]);
        let source = 测试正文投影(room_id.clone(), content_id, digest);
        let projections = Arc::new(固定正文投影(source));
        let content = Arc::new(OpenMessageContentService::new(
            OpenMessageContentDependencies {
                projections: projections.clone(),
                content: Arc::new(固定正文网关(DownloadedMessageContent {
                    bytes: Arc::from("正文".as_bytes()),
                    digest,
                    byte_length: ContentByteLength::new(6).expect("正文长度有效"),
                    media_type: ContentMediaType::new("text/plain").expect("媒体类型有效"),
                })),
            },
        ));
        let handler = FoundationBridgeIpcRequestHandler::with_agent_runtime(
            Arc::new(固定状态),
            Arc::new(固定Agent运行时(BridgeAgentRuntimeSnapshot::new(
                测试_agent_身份(),
                "DEVICE-1",
                room_id.clone(),
                ["content.read"],
            ))),
            projections,
            content,
        );

        let response = handler
            .dispatch(IpcMethod::OpenContent(IpcOpenContentRequest {
                content_id: content_id.to_string(),
            }))
            .await
            .expect("当前大厅正文可打开");

        assert!(matches!(
            response,
            IpcResponse::OpenedContent { content }
                if content.body == "正文"
                    && content.source_room_id == room_id.as_str()
                    && content.source_event_id == "$message:matrix.test"
                    && content.content.media_type == "text/plain"
                    && content.content.digest_sha256
                        == "d661c3d96d53ebc0ca8a55aae24b5df4a4d1bf28d37337b982fe8ebf54846eeb"
                    && content.risk_flags == ["external_link"]
        ));
    }

    #[tokio::test]
    async fn 消息发送贯穿正文上传签名发布绑定且相同提交号保持幂等() {
        let temporary = tempdir().expect("测试目录可创建");
        let room_id = MatrixRoomId::new("!lobby:matrix.test").expect("房间标识有效");
        let identity = 测试_agent_身份();
        let publisher = Arc::new(记录消息发布器::default());
        let content = Arc::new(记录正文写入::default());
        let submissions = Arc::new(
            SqliteMessageSubmissionRepository::open(temporary.path().join("messages.sqlite3"))
                .await
                .expect("提交仓库可打开"),
        );
        let publication = Arc::new(MessagePublicationService::new(
            MessagePublicationDependencies {
                identity: identity.clone(),
                signer: Arc::new(测试签名身份),
                publisher: publisher.clone(),
                content: content.clone(),
                submissions,
            },
        ));
        let previews = Arc::new(记录预览查询::default());
        let handler = FoundationBridgeIpcRequestHandler::with_agent_runtime(
            Arc::new(固定状态),
            Arc::new(固定Agent运行时(
                BridgeAgentRuntimeSnapshot::new(
                    identity,
                    "DEVICE-1",
                    room_id.clone(),
                    ["message.send"],
                )
                .with_message_publication(publication),
            )),
            previews.clone(),
            空正文服务(previews),
        );
        let submission_id = Uuid::now_v7().to_string();
        let request = IpcSendMessageRequest {
            submission_id: Some(submission_id.clone()),
            room_id: room_id.as_str().to_owned(),
            title: "构建完成".to_owned(),
            summary: "Bridge 已通过完整验证".to_owned(),
            body: "正文只在用户明确打开时读取。".to_owned(),
            media_type: "text/markdown".to_owned(),
            language: Some("zh-CN".to_owned()),
            sensitivity: IpcMessageSensitivity::Normal,
            risk_flags: vec!["untrusted_instructions".to_owned()],
            provenance: IpcMessageProvenance::AutonomousAgent,
            reply_to_message_id: None,
        };

        for _ in 0..2 {
            let response = handler
                .dispatch(IpcMethod::SendMessage(request.clone()))
                .await
                .expect("消息可幂等发送");
            assert!(matches!(
                response,
                IpcResponse::SentMessage { message }
                    if message.submission_id == submission_id
                        && message.state == IpcSubmissionState::Submitted
                        && message.event_id.as_deref() == Some("$sent:matrix.test")
            ));
        }

        let events = publisher.0.lock().expect("消息事件锁可用");
        assert_eq!(events.len(), 1);
        drop(events);
        let uploads = content.uploads.lock().expect("正文上传锁可用");
        assert_eq!(uploads.len(), 1);
        assert_eq!(
            uploads[0].body.as_ref(),
            "正文只在用户明确打开时读取。".as_bytes()
        );
        drop(uploads);
        assert_eq!(content.bindings.lock().expect("正文绑定锁可用").len(), 1);
    }

    #[tokio::test]
    async fn 一次性交接消费与拒绝通过运行时而正文只返回一次() {
        let room_id = MatrixRoomId::new("!lobby:matrix.test").expect("房间标识有效");
        let body = Arc::<[u8]>::from("正文".as_bytes());
        let digest = Sha256Digest::from_bytes([
            0xd6, 0x61, 0xc3, 0xd9, 0x6d, 0x53, 0xeb, 0xc0, 0xca, 0x8a, 0x55, 0xaa, 0xe2, 0x4b,
            0x5d, 0xf4, 0xa4, 0xd1, 0xbf, 0x28, 0xd3, 0x73, 0x37, 0xb9, 0x82, 0xfe, 0x8e, 0xbf,
            0x54, 0x84, 0x6e, 0xeb,
        ]);
        let content_id = ContentId::from_uuid(Uuid::now_v7());
        let source = 测试正文投影(room_id.clone(), content_id, digest);
        let pending = 测试已送达交接(&source, 测试_agent_身份().agent_instance_id());
        let handoff_id = pending.fields().id;
        let handoffs = Arc::new(固定交接运行时 { pending, body });
        let projections = Arc::new(固定正文投影(source));
        let handler = FoundationBridgeIpcRequestHandler::with_agent_runtime(
            Arc::new(固定状态),
            Arc::new(固定Agent运行时(
                BridgeAgentRuntimeSnapshot::new(
                    测试_agent_身份(),
                    "DEVICE-1",
                    room_id,
                    ["handoff.consume", "handoff.decline"],
                )
                .with_handoffs(handoffs),
            )),
            projections.clone(),
            空正文服务(projections),
        );

        let consumed = handler
            .dispatch(IpcMethod::ConsumeHandoff(IpcHandoffRequest {
                handoff_id: handoff_id.to_string(),
            }))
            .await
            .expect("交接可经本机运行时原子消费");
        assert!(matches!(
            consumed,
            IpcResponse::ConsumedHandoff { handoff }
                if handoff.handoff_id == handoff_id.to_string()
                    && handoff.body == "正文"
                    && handoff.source_event_id == "$message:matrix.test"
                    && handoff.source_actor.agent.display_name == "Codex Agent"
                    && handoff.purpose == "summarize"
                    && handoff.risk_flags == ["external_link"]
        ));

        let declined = handler
            .dispatch(IpcMethod::DeclineHandoff(IpcHandoffRequest {
                handoff_id: handoff_id.to_string(),
            }))
            .await
            .expect("拒绝命令交给独立运行时用例");
        assert!(matches!(
            declined,
            IpcResponse::DeclinedHandoff { handoff }
                if handoff.handoff_id == handoff_id.to_string()
                    && handoff.status == IpcHandoffStatus::Declined
        ));
    }

    fn 测试_agent_身份() -> BridgeAgentIdentity {
        BridgeAgentIdentity::new(
            AgentId::from_uuid(
                Uuid::parse_str("01945c1e-7b5a-7c7f-8a28-2de53f56a9a3").expect("Agent 标识有效"),
            ),
            "Codex Agent",
            "@_agent_01945c1e7b5a7c7f8a282de53f56a9a3:matrix.test",
            AgentInstanceId::from_uuid(
                Uuid::parse_str("01945c1e-7b5a-7c7f-8a28-2de53f56a9a4").expect("实例标识有效"),
            ),
        )
        .expect("Agent 身份有效")
    }

    fn 测试正文投影(
        room_id: MatrixRoomId,
        content_id: ContentId,
        digest: Sha256Digest,
    ) -> ProjectedMessagePreview {
        let media_type = ContentMediaType::new("text/plain").expect("媒体类型有效");
        ProjectedMessagePreview {
            event_id: MatrixEventId::new("$message:matrix.test").expect("事件标识有效"),
            transaction_id: None,
            room_id,
            message_id: MessageId::from_uuid(
                Uuid::parse_str("01945c1e-7b5a-7c7f-8a28-2de53f56a9a6").expect("消息标识有效"),
            ),
            created_at: UtcMillis::new(1_000).expect("消息时间有效"),
            origin_server_timestamp: Some(1_000),
            actor: ProjectedMessageActor::new(
                测试_agent_身份(),
                MessageProvenance::AutonomousAgent,
            ),
            preview: MessagePreview::new(
                MessageTitle::new("标题").expect("标题有效"),
                MessageSummary::new("摘要").expect("摘要有效"),
                media_type,
                None,
                MessageSensitivity::Normal,
                MessageRiskFlags::new([
                    MessageRiskFlag::new("external_link").expect("风险标签有效")
                ])
                .expect("风险标签集合有效"),
            ),
            content: MessageContentReference::new(content_id, digest, 6).expect("正文引用有效"),
            relation: None,
        }
    }

    fn 测试已送达交接(
        source: &ProjectedMessagePreview,
        target_instance_id: AgentInstanceId,
    ) -> ContextHandoff {
        let actor = source.actor.identity();
        let mut handoff = ContextHandoff::propose(ContextHandoffFields {
            id: HandoffId::from_uuid(Uuid::now_v7()),
            requester_agent_id: 测试_agent_身份().agent_id(),
            requester_instance_id: 测试_agent_身份().agent_instance_id(),
            source: HandoffSource::new(
                MatrixRoomReference::new(source.room_id.as_str()).expect("房间引用有效"),
                HandoffSourceEventId::new(source.event_id.as_str()).expect("来源事件有效"),
                source.message_id,
                HandoffSourceActor::new(
                    actor.agent_id(),
                    actor.agent_instance_id(),
                    source.actor.provenance(),
                ),
            ),
            target_agent_id: 测试_agent_身份().agent_id(),
            target_instance_id,
            content: HandoffContentReference::new(
                source.content.content_id(),
                source.content.digest(),
                ContentByteLength::new(source.content.size_bytes()).expect("正文长度有效"),
                source.preview.content_type().clone(),
            ),
            permissions: HandoffPermissions::new([HandoffPermission::ReadText])
                .expect("交接权限有效"),
            purpose: HandoffPurpose::Summarize,
            risk_flags: source.preview.risk_flags().clone(),
            proposed_at: UtcMillis::new(900).expect("提案时间有效"),
            expires_at: UtcMillis::new(2_000).expect("到期时间有效"),
        })
        .expect("交接提案有效");
        handoff
            .approve(
                PrincipalId::from_uuid(Uuid::now_v7()),
                UtcMillis::new(950).expect("批准时间有效"),
            )
            .expect("交接可批准");
        handoff
            .mark_delivered(UtcMillis::new(1_000).expect("送达时间有效"))
            .expect("交接可标记送达");
        handoff
    }

    fn 空正文服务(
        projections: Arc<dyn MessageTimelineQueryRepository>,
    ) -> Arc<OpenMessageContentService> {
        Arc::new(OpenMessageContentService::new(
            OpenMessageContentDependencies {
                projections,
                content: Arc::new(拒绝正文网关),
            },
        ))
    }

    #[tokio::test]
    async fn 完成挑战后只能调用已授权的闭合方法() {
        let installation_id = IpcInstallationId::new("install_1").expect("安装标识有效");
        let secret = IpcSharedSecret::new([7; 32]);
        let context = BridgeIpcContext {
            installation_id: installation_id.clone(),
            shared_secret: secret.clone(),
            server_instance_id: Uuid::from_u128(99),
            request_handler: Arc::new(FoundationBridgeIpcRequestHandler::new(Arc::new(固定状态))),
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

        assert_get_self_scope_denied(&mut client).await;
        drop(client);
        server_task
            .await
            .expect("服务端任务未崩溃")
            .expect("客户端正常断开");
    }

    async fn assert_get_self_scope_denied<S>(client: &mut S)
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let denied_id = Uuid::from_u128(2);
        IpcFrameCodec::write(
            client,
            &IpcFrame::Request {
                correlation_id: denied_id,
                method: IpcMethod::GetSelf,
            },
        )
        .await
        .expect("越权请求可发送");
        assert!(matches!(
            IpcFrameCodec::read(client).await.expect("越权响应可读取"),
            IpcFrame::Error {
                correlation_id: Some(id),
                category: IpcErrorCategory::Authorization,
                ..
            } if id == denied_id
        ));
    }

    #[tokio::test]
    async fn 错误挑战证明在任何请求分派前拒绝() {
        let installation_id = IpcInstallationId::new("install_2").expect("安装标识有效");
        let context = BridgeIpcContext {
            installation_id: installation_id.clone(),
            shared_secret: IpcSharedSecret::new([7; 32]),
            server_instance_id: Uuid::from_u128(99),
            request_handler: Arc::new(FoundationBridgeIpcRequestHandler::new(Arc::new(固定状态))),
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
            Arc::new(FoundationBridgeIpcRequestHandler::new(Arc::new(固定状态))),
        )
        .expect("首个私有端点可创建");

        let Err(second) = BridgeIpcServer::bind(
            &paths,
            installation_id,
            IpcSharedSecret::new([1; 32]),
            Arc::new(FoundationBridgeIpcRequestHandler::new(Arc::new(固定状态))),
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
