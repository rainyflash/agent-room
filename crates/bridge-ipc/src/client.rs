use std::collections::BTreeMap;

use agent_room_bridge_core::ipc::{
    IpcCallerKind, IpcHandshakeAgreement, IpcHandshakeFailure, IpcHandshakeFailureKind,
    IpcHandshakeOffer, IpcInstallationId, IpcProtocolVersion, IpcScope,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use tokio::io::{AsyncRead, AsyncWrite};
use uuid::Uuid;

use crate::{
    IpcCaller, IpcChallenge, IpcErrorCategory, IpcFrame, IpcFrameCodec, IpcMethod,
    IpcProtocolFailure, IpcResponse, IpcScopeName, IpcSharedSecret, IpcVersion,
    create_challenge_proof,
};

const CHALLENGE_BYTES: usize = 32;

#[derive(Clone)]
pub struct IpcClientCredentials {
    installation_id: IpcInstallationId,
    shared_secret: IpcSharedSecret,
}

impl IpcClientCredentials {
    pub const fn new(installation_id: IpcInstallationId, shared_secret: IpcSharedSecret) -> Self {
        Self {
            installation_id,
            shared_secret,
        }
    }

    pub const fn installation_id(&self) -> &IpcInstallationId {
        &self.installation_id
    }
}

#[derive(Debug)]
pub struct IpcClientSession<S> {
    stream: S,
    agreement: IpcHandshakeAgreement,
    server_instance_id: Uuid,
}

impl<S> IpcClientSession<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// 在已连接的本地流上完成版本、作用域和安装身份挑战。
    ///
    /// # Errors
    ///
    /// 线协议、版本、作用域、HMAC 证明或服务端拒绝失败时返回结构化错误。
    pub async fn authenticate(
        mut stream: S,
        credentials: &IpcClientCredentials,
        caller: IpcCallerKind,
        requested_scopes: impl IntoIterator<Item = IpcScope>,
    ) -> Result<Self, IpcClientFailure> {
        let offer = IpcHandshakeOffer::new(caller, [IpcProtocolVersion::V2_0], requested_scopes)
            .map_err(IpcClientFailure::handshake)?;
        send_client_hello(&mut stream, credentials.installation_id(), &offer).await?;
        let (challenge_id, challenge, agreement) =
            read_server_challenge(&mut stream, &offer).await?;
        send_client_proof(
            &mut stream,
            credentials,
            &offer,
            &agreement,
            challenge_id,
            challenge,
        )
        .await?;
        let server_instance_id = read_server_ready(&mut stream, &offer, &agreement).await?;
        Ok(Self {
            stream,
            agreement,
            server_instance_id,
        })
    }

    /// 在已鉴权会话中发送一个闭合工具请求。
    ///
    /// # Errors
    ///
    /// 本地参数、作用域、传输、响应关联标识或 Bridge 业务失败时返回结构化错误。
    pub async fn request(&mut self, method: IpcMethod) -> Result<IpcResponse, IpcClientFailure> {
        method.validate().map_err(|failure| {
            IpcClientFailure::local(
                failure.code(),
                IpcClientFailureKind::Validation,
                IpcErrorCategory::Validation,
                false,
            )
        })?;
        if !self
            .agreement
            .granted_scopes()
            .contains(&method.required_scope())
        {
            return Err(IpcClientFailure::local(
                "bridge.ipc.scope_denied",
                IpcClientFailureKind::Authorization,
                IpcErrorCategory::Authorization,
                false,
            ));
        }

        let correlation_id = Uuid::now_v7();
        IpcFrameCodec::write(
            &mut self.stream,
            &IpcFrame::Request {
                correlation_id,
                method,
            },
        )
        .await
        .map_err(IpcClientFailure::protocol)?;
        let frame = IpcFrameCodec::read(&mut self.stream)
            .await
            .map_err(IpcClientFailure::protocol)?;
        response_from_frame(frame, correlation_id)
    }

    pub const fn server_instance_id(&self) -> Uuid {
        self.server_instance_id
    }

    pub const fn agreement(&self) -> &IpcHandshakeAgreement {
        &self.agreement
    }
}

async fn send_client_hello<S>(
    stream: &mut S,
    installation_id: &IpcInstallationId,
    offer: &IpcHandshakeOffer,
) -> Result<(), IpcClientFailure>
where
    S: AsyncWrite + Unpin,
{
    let frame = IpcFrame::ClientHello {
        installation_id: installation_id.as_str().to_owned(),
        caller: wire_caller(offer.caller()),
        supported_versions: offer
            .supported_versions()
            .iter()
            .copied()
            .map(IpcVersion::from)
            .collect(),
        requested_scopes: offer
            .requested_scopes()
            .iter()
            .copied()
            .map(IpcScopeName::from)
            .collect(),
    };
    IpcFrameCodec::write(stream, &frame)
        .await
        .map_err(IpcClientFailure::protocol)
}

async fn read_server_challenge<S>(
    stream: &mut S,
    offer: &IpcHandshakeOffer,
) -> Result<(Uuid, IpcChallenge, IpcHandshakeAgreement), IpcClientFailure>
where
    S: AsyncRead + Unpin,
{
    match IpcFrameCodec::read(stream)
        .await
        .map_err(IpcClientFailure::protocol)?
    {
        IpcFrame::ServerChallenge {
            challenge_id,
            challenge,
            selected_version,
            granted_scopes,
        } => {
            let challenge = decode_challenge(&challenge)?;
            let agreement = agreement_from_wire(offer, selected_version, granted_scopes)?;
            Ok((challenge_id, challenge, agreement))
        }
        IpcFrame::Error {
            correlation_id: None,
            code,
            category,
            retryable,
            details,
        } => Err(IpcClientFailure::remote(code, category, retryable, details)),
        _ => Err(IpcClientFailure::invalid_response()),
    }
}

async fn send_client_proof<S>(
    stream: &mut S,
    credentials: &IpcClientCredentials,
    offer: &IpcHandshakeOffer,
    agreement: &IpcHandshakeAgreement,
    challenge_id: Uuid,
    challenge: IpcChallenge,
) -> Result<(), IpcClientFailure>
where
    S: AsyncWrite + Unpin,
{
    let proof = create_challenge_proof(
        &credentials.shared_secret,
        challenge_id,
        challenge,
        credentials.installation_id(),
        offer,
        agreement,
    )
    .map_err(|_| IpcClientFailure::authentication())?;
    IpcFrameCodec::write(
        stream,
        &IpcFrame::ClientProof {
            challenge_id,
            proof: URL_SAFE_NO_PAD.encode(proof.as_bytes()),
        },
    )
    .await
    .map_err(IpcClientFailure::protocol)
}

async fn read_server_ready<S>(
    stream: &mut S,
    offer: &IpcHandshakeOffer,
    challenge_agreement: &IpcHandshakeAgreement,
) -> Result<Uuid, IpcClientFailure>
where
    S: AsyncRead + Unpin,
{
    match IpcFrameCodec::read(stream)
        .await
        .map_err(IpcClientFailure::protocol)?
    {
        IpcFrame::ServerReady {
            server_instance_id,
            selected_version,
            granted_scopes,
        } => {
            let ready_agreement = agreement_from_wire(offer, selected_version, granted_scopes)?;
            if ready_agreement != *challenge_agreement {
                return Err(IpcClientFailure::invalid_response());
            }
            Ok(server_instance_id)
        }
        IpcFrame::Error {
            correlation_id: None,
            code,
            category,
            retryable,
            details,
        } => Err(IpcClientFailure::remote(code, category, retryable, details)),
        _ => Err(IpcClientFailure::invalid_response()),
    }
}

fn response_from_frame(
    frame: IpcFrame,
    expected_correlation_id: Uuid,
) -> Result<IpcResponse, IpcClientFailure> {
    match frame {
        IpcFrame::Response {
            correlation_id,
            result,
        } if correlation_id == expected_correlation_id => Ok(result),
        IpcFrame::Error {
            correlation_id: Some(correlation_id),
            code,
            category,
            retryable,
            details,
        } if correlation_id == expected_correlation_id => {
            Err(IpcClientFailure::remote(code, category, retryable, details))
        }
        _ => Err(IpcClientFailure::invalid_response()),
    }
}

fn agreement_from_wire(
    offer: &IpcHandshakeOffer,
    selected_version: IpcVersion,
    granted_scopes: Vec<IpcScopeName>,
) -> Result<IpcHandshakeAgreement, IpcClientFailure> {
    let version = IpcProtocolVersion::new(selected_version.major, selected_version.minor)
        .map_err(IpcClientFailure::handshake)?;
    IpcHandshakeAgreement::from_server_selection(
        offer,
        version,
        granted_scopes.into_iter().map(IpcScope::from),
    )
    .map_err(IpcClientFailure::handshake)
}

fn decode_challenge(encoded: &str) -> Result<IpcChallenge, IpcClientFailure> {
    let bytes: [u8; CHALLENGE_BYTES] = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| IpcClientFailure::authentication())?
        .try_into()
        .map_err(|_| IpcClientFailure::authentication())?;
    Ok(IpcChallenge::new(bytes))
}

const fn wire_caller(caller: IpcCallerKind) -> IpcCaller {
    match caller {
        IpcCallerKind::McpServer => IpcCaller::McpServer,
        IpcCallerKind::DesktopShell => IpcCaller::DesktopShell,
        IpcCallerKind::DiagnosticCli => IpcCaller::DiagnosticCli,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcClientFailureKind {
    Validation,
    Protocol,
    InvalidHandshake,
    Authentication,
    Authorization,
    IncompatibleVersion,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcClientFailure {
    kind: IpcClientFailureKind,
    code: String,
    category: IpcErrorCategory,
    retryable: bool,
    details: BTreeMap<String, String>,
}

impl IpcClientFailure {
    fn local(
        code: impl Into<String>,
        kind: IpcClientFailureKind,
        category: IpcErrorCategory,
        retryable: bool,
    ) -> Self {
        Self {
            kind,
            code: code.into(),
            category,
            retryable,
            details: BTreeMap::new(),
        }
    }

    fn protocol(_failure: IpcProtocolFailure) -> Self {
        Self::local(
            "bridge.ipc.protocol_failed",
            IpcClientFailureKind::Protocol,
            IpcErrorCategory::DependencyUnavailable,
            true,
        )
    }

    fn invalid_response() -> Self {
        Self::local(
            "bridge.ipc.response_invalid",
            IpcClientFailureKind::InvalidHandshake,
            IpcErrorCategory::Internal,
            false,
        )
    }

    fn authentication() -> Self {
        Self::local(
            "bridge.ipc.authentication_rejected",
            IpcClientFailureKind::Authentication,
            IpcErrorCategory::Authentication,
            false,
        )
    }

    fn handshake(failure: IpcHandshakeFailure) -> Self {
        match failure.kind() {
            IpcHandshakeFailureKind::IncompatibleVersion => Self::local(
                "bridge.ipc.version_incompatible",
                IpcClientFailureKind::IncompatibleVersion,
                IpcErrorCategory::IncompatibleVersion,
                false,
            ),
            IpcHandshakeFailureKind::ScopeDenied => Self::local(
                "bridge.ipc.scope_denied",
                IpcClientFailureKind::Authorization,
                IpcErrorCategory::Authorization,
                false,
            ),
            IpcHandshakeFailureKind::AuthenticationRejected => Self::authentication(),
            IpcHandshakeFailureKind::InvalidConfiguration
            | IpcHandshakeFailureKind::InvalidOffer => Self::local(
                "bridge.ipc.handshake_invalid",
                IpcClientFailureKind::InvalidHandshake,
                IpcErrorCategory::Validation,
                false,
            ),
        }
    }

    fn remote(
        code: String,
        category: IpcErrorCategory,
        retryable: bool,
        details: BTreeMap<String, String>,
    ) -> Self {
        Self {
            kind: IpcClientFailureKind::Remote,
            code,
            category,
            retryable,
            details,
        }
    }

    pub const fn kind(&self) -> IpcClientFailureKind {
        self.kind
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub const fn category(&self) -> IpcErrorCategory {
        self.category
    }

    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    pub const fn details(&self) -> &BTreeMap<String, String> {
        &self.details
    }
}

#[cfg(test)]
mod tests {
    use agent_room_bridge_core::ipc::{
        FoundationIpcScopePolicy, IpcCallerKind, IpcHandshakeNegotiator, IpcInstallationId,
        IpcProtocolVersion, IpcScope,
    };
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use tokio::io::{DuplexStream, duplex};
    use uuid::Uuid;

    use crate::{
        IpcBridgeState, IpcChallenge, IpcChallengeProof, IpcErrorCategory, IpcFrame, IpcFrameCodec,
        IpcMethod, IpcResponse, IpcScopeName, IpcSharedSecret, IpcVersion, client_offer_from_frame,
        server_agreement_frame, verify_challenge_proof,
    };

    use super::{IpcClientCredentials, IpcClientFailureKind, IpcClientSession};

    #[tokio::test]
    async fn 客户端完成挑战并关联单个响应() {
        let installation_id = IpcInstallationId::new("install_1").expect("安装标识有效");
        let secret = IpcSharedSecret::new([7; 32]);
        let credentials = IpcClientCredentials::new(installation_id.clone(), secret.clone());
        let (client_stream, server_stream) = duplex(16 * 1_024);
        let server = tokio::spawn(script_successful_server(
            server_stream,
            installation_id,
            secret,
        ));

        let mut client = IpcClientSession::authenticate(
            client_stream,
            &credentials,
            IpcCallerKind::McpServer,
            [IpcScope::BridgeStatusRead],
        )
        .await
        .expect("挑战应成功");
        let response = client
            .request(IpcMethod::BridgeStatus)
            .await
            .expect("状态请求应成功");

        assert_eq!(
            response,
            IpcResponse::BridgeStatus {
                state: IpcBridgeState::Ready,
                started_at_unix_ms: 1_000,
            }
        );
        server.await.expect("脚本服务端未崩溃");
    }

    #[tokio::test]
    async fn 客户端把未提议版本报告为可修复不兼容() {
        let installation_id = IpcInstallationId::new("install_2").expect("安装标识有效");
        let credentials = IpcClientCredentials::new(installation_id, IpcSharedSecret::new([7; 32]));
        let (client_stream, mut server_stream) = duplex(4 * 1_024);
        let server = tokio::spawn(async move {
            let _hello = IpcFrameCodec::read(&mut server_stream)
                .await
                .expect("问候可读取");
            IpcFrameCodec::write(
                &mut server_stream,
                &IpcFrame::ServerChallenge {
                    challenge_id: Uuid::from_u128(1),
                    challenge: URL_SAFE_NO_PAD.encode([9_u8; 32]),
                    selected_version: IpcVersion { major: 1, minor: 0 },
                    granted_scopes: vec![IpcScopeName::BridgeStatusRead],
                },
            )
            .await
            .expect("挑战可发送");
        });

        let failure = IpcClientSession::authenticate(
            client_stream,
            &credentials,
            IpcCallerKind::McpServer,
            [IpcScope::BridgeStatusRead],
        )
        .await
        .expect_err("未提议版本必须失败");

        assert_eq!(failure.kind(), IpcClientFailureKind::IncompatibleVersion);
        assert_eq!(failure.code(), "bridge.ipc.version_incompatible");
        assert_eq!(failure.category(), IpcErrorCategory::IncompatibleVersion);
        server.await.expect("脚本服务端未崩溃");
    }

    async fn script_successful_server(
        mut stream: DuplexStream,
        installation_id: IpcInstallationId,
        secret: IpcSharedSecret,
    ) {
        let hello = IpcFrameCodec::read(&mut stream).await.expect("问候可读取");
        let (_, offer) = client_offer_from_frame(&hello).expect("问候有效");
        let agreement =
            IpcHandshakeNegotiator::new([IpcProtocolVersion::V2_0], FoundationIpcScopePolicy)
                .expect("协商器有效")
                .negotiate(&offer)
                .expect("提议可协商");
        let challenge_id = Uuid::from_u128(1);
        let challenge = IpcChallenge::new([9; 32]);
        IpcFrameCodec::write(
            &mut stream,
            &IpcFrame::ServerChallenge {
                challenge_id,
                challenge: URL_SAFE_NO_PAD.encode(challenge.as_bytes()),
                selected_version: agreement.selected_version().into(),
                granted_scopes: agreement
                    .granted_scopes()
                    .iter()
                    .copied()
                    .map(IpcScopeName::from)
                    .collect(),
            },
        )
        .await
        .expect("挑战可发送");
        assert_valid_proof(
            &mut stream,
            &secret,
            challenge_id,
            challenge,
            &installation_id,
            &offer,
            &agreement,
        )
        .await;
        IpcFrameCodec::write(
            &mut stream,
            &server_agreement_frame(Uuid::from_u128(2), &agreement),
        )
        .await
        .expect("就绪帧可发送");
        respond_to_status(&mut stream).await;
    }

    async fn assert_valid_proof(
        stream: &mut DuplexStream,
        secret: &IpcSharedSecret,
        challenge_id: Uuid,
        challenge: IpcChallenge,
        installation_id: &IpcInstallationId,
        offer: &agent_room_bridge_core::ipc::IpcHandshakeOffer,
        agreement: &agent_room_bridge_core::ipc::IpcHandshakeAgreement,
    ) {
        let IpcFrame::ClientProof {
            challenge_id: received_id,
            proof,
        } = IpcFrameCodec::read(stream).await.expect("证明可读取")
        else {
            panic!("必须返回客户端证明");
        };
        let bytes: [u8; 32] = URL_SAFE_NO_PAD
            .decode(proof)
            .expect("证明为 base64url")
            .try_into()
            .expect("证明长度固定");
        assert_eq!(received_id, challenge_id);
        assert!(verify_challenge_proof(
            secret,
            challenge_id,
            challenge,
            installation_id,
            offer,
            agreement,
            IpcChallengeProof::new(bytes),
        ));
    }

    async fn respond_to_status(stream: &mut DuplexStream) {
        let IpcFrame::Request {
            correlation_id,
            method: IpcMethod::BridgeStatus,
        } = IpcFrameCodec::read(stream).await.expect("请求可读取")
        else {
            panic!("必须请求 Bridge 状态");
        };
        IpcFrameCodec::write(
            stream,
            &IpcFrame::Response {
                correlation_id,
                result: IpcResponse::BridgeStatus {
                    state: IpcBridgeState::Ready,
                    started_at_unix_ms: 1_000,
                },
            },
        )
        .await
        .expect("响应可发送");
    }
}
