use std::sync::Arc;

use agent_room_bridge_core::{
    messages::{
        DownloadedMessageContent, MessageContentReadFailure, MessageContentReadFailureKind,
        MessageContentReadGateway, MessageContentReadRequest,
    },
    session::{
        AuthorizedControlPlaneRequest, BridgeSessionFailure, BridgeSessionFailureKind,
        ControlPlaneRequestAuthorizer,
    },
};
use agent_room_domain::content::{ContentByteLength, ContentMediaType, Sha256Digest};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{Client, StatusCode, header};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{
    ControlPlaneHttpConfig, ControlPlaneHttpConfigurationError, configured_client,
    signed_request_headers,
};

const MAX_TICKET_BYTES: usize = 4_096;
const CONTENT_DIGEST_HEADER: &str = "content-digest";

pub struct ReqwestControlPlaneContentGateway {
    client: Client,
    base_url: Url,
    authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
}

impl ReqwestControlPlaneContentGateway {
    /// 创建以 Bridge 设备会话签名的按需正文客户端。
    ///
    /// # Errors
    ///
    /// 控制面地址、明文传输边界、超时或 HTTP 客户端配置无效时返回错误。
    pub fn new(
        config: &ControlPlaneHttpConfig,
        authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
    ) -> Result<Self, ControlPlaneHttpConfigurationError> {
        let (client, base_url) = configured_client(config)?;
        Ok(Self {
            client,
            base_url,
            authorizer,
        })
    }

    async fn open_internal(
        &self,
        request: &MessageContentReadRequest,
    ) -> Result<DownloadedMessageContent, MessageContentReadFailure> {
        let ticket = self.issue_read_ticket(request).await?;
        self.download(request, &ticket).await
    }

    async fn issue_read_ticket(
        &self,
        request: &MessageContentReadRequest,
    ) -> Result<String, MessageContentReadFailure> {
        let request_target = format!("/content/{}/read-tickets", request.content_id());
        let authorized = self.authorize("POST", &request_target, "").await?;
        let response = Self::signed_request(
            self.client.post(self.url(&request_target)?),
            &authorized,
            "POST",
            &request_target,
        )?
        .body(Vec::new())
        .send()
        .await
        .map_err(|_| read_failure(MessageContentReadFailureKind::Unavailable))?;
        if !response.status().is_success() {
            return Err(read_status_failure(response.status()));
        }
        let bytes = read_response_body(response, MAX_TICKET_BYTES).await?;
        let decoded = serde_json::from_slice::<ReadTicketResponse>(&bytes)
            .map_err(|_| read_failure(MessageContentReadFailureKind::InvalidResponse))?;
        if !(16..=MAX_TICKET_BYTES).contains(&decoded.ticket.len())
            || decoded.ticket.chars().any(char::is_control)
            || decoded.expires_at_unix_ms <= 0
        {
            return Err(read_failure(MessageContentReadFailureKind::InvalidResponse));
        }
        Ok(decoded.ticket)
    }

    async fn download(
        &self,
        request: &MessageContentReadRequest,
        ticket: &str,
    ) -> Result<DownloadedMessageContent, MessageContentReadFailure> {
        let request_target = format!("/content/{}/open", request.content_id());
        let body = serde_json::to_string(&OpenContentBody { ticket })
            .map_err(|_| read_failure(MessageContentReadFailureKind::Internal))?;
        let authorized = self.authorize("POST", &request_target, &body).await?;
        let response = Self::signed_request(
            self.client
                .post(self.url(&request_target)?)
                .header(header::ACCEPT, "application/octet-stream")
                .header(header::CONTENT_TYPE, "application/json"),
            &authorized,
            "POST",
            &request_target,
        )?
        .body(body)
        .send()
        .await
        .map_err(|_| read_failure(MessageContentReadFailureKind::Unavailable))?;
        if !response.status().is_success() {
            return Err(read_status_failure(response.status()));
        }
        decode_download(response, request.maximum_bytes()).await
    }

    async fn authorize(
        &self,
        method: &str,
        request_target: &str,
        body: &str,
    ) -> Result<AuthorizedControlPlaneRequest, MessageContentReadFailure> {
        self.authorizer
            .authorize(method, request_target, body)
            .await
            .map_err(map_session_failure)
    }

    fn url(&self, request_target: &str) -> Result<Url, MessageContentReadFailure> {
        self.base_url
            .join(request_target.trim_start_matches('/'))
            .map_err(|_| read_failure(MessageContentReadFailureKind::Internal))
    }

    fn signed_request(
        request: reqwest::RequestBuilder,
        authorized: &AuthorizedControlPlaneRequest,
        method: &str,
        request_target: &str,
    ) -> Result<reqwest::RequestBuilder, MessageContentReadFailure> {
        signed_request_headers(request, authorized, method, request_target)
            .map_err(|()| read_failure(MessageContentReadFailureKind::Internal))
    }
}

impl MessageContentReadGateway for ReqwestControlPlaneContentGateway {
    fn open<'a>(
        &'a self,
        request: &'a MessageContentReadRequest,
    ) -> agent_room_application::ports::PortFuture<
        'a,
        Result<DownloadedMessageContent, MessageContentReadFailure>,
    > {
        Box::pin(self.open_internal(request))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadTicketResponse {
    ticket: String,
    expires_at_unix_ms: i64,
}

#[derive(Serialize)]
struct OpenContentBody<'a> {
    ticket: &'a str,
}

async fn decode_download(
    response: reqwest::Response,
    maximum_bytes: u64,
) -> Result<DownloadedMessageContent, MessageContentReadFailure> {
    let declared_length = response
        .content_length()
        .ok_or_else(|| read_failure(MessageContentReadFailureKind::InvalidResponse))?;
    if declared_length == 0 || declared_length > maximum_bytes {
        return Err(read_failure(MessageContentReadFailureKind::InvalidResponse));
    }
    let media_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .and_then(|value| ContentMediaType::new(value).ok())
        .ok_or_else(|| read_failure(MessageContentReadFailureKind::InvalidResponse))?;
    let digest = response
        .headers()
        .get(CONTENT_DIGEST_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_content_digest)
        .ok_or_else(|| read_failure(MessageContentReadFailureKind::InvalidResponse))?;
    let bytes = read_response_body(
        response,
        usize::try_from(maximum_bytes)
            .map_err(|_| read_failure(MessageContentReadFailureKind::InvalidRequest))?,
    )
    .await?;
    if u64::try_from(bytes.len()).ok() != Some(declared_length) {
        return Err(read_failure(MessageContentReadFailureKind::InvalidResponse));
    }
    Ok(DownloadedMessageContent {
        bytes: Arc::from(bytes),
        digest,
        byte_length: ContentByteLength::new(declared_length)
            .map_err(|_| read_failure(MessageContentReadFailureKind::InvalidResponse))?,
        media_type,
    })
}

async fn read_response_body(
    mut response: reqwest::Response,
    maximum_bytes: usize,
) -> Result<Vec<u8>, MessageContentReadFailure> {
    if response
        .content_length()
        .is_some_and(|length| usize::try_from(length).map_or(true, |value| value > maximum_bytes))
    {
        return Err(read_failure(MessageContentReadFailureKind::InvalidResponse));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| read_failure(MessageContentReadFailureKind::Unavailable))?
    {
        if body.len().saturating_add(chunk.len()) > maximum_bytes {
            return Err(read_failure(MessageContentReadFailureKind::InvalidResponse));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn parse_content_digest(value: &str) -> Option<Sha256Digest> {
    let encoded = value.strip_prefix("sha-256=:")?.strip_suffix(':')?;
    let bytes: [u8; 32] = STANDARD.decode(encoded).ok()?.try_into().ok()?;
    Some(Sha256Digest::from_bytes(bytes))
}

fn map_session_failure(failure: BridgeSessionFailure) -> MessageContentReadFailure {
    let kind = match failure.kind() {
        BridgeSessionFailureKind::NotAuthorized
        | BridgeSessionFailureKind::RefreshOutcomeUnknown => MessageContentReadFailureKind::Denied,
        BridgeSessionFailureKind::SecureStorageUnavailable
        | BridgeSessionFailureKind::ControlPlaneUnavailable => {
            MessageContentReadFailureKind::Unavailable
        }
        BridgeSessionFailureKind::CorruptSecureStorage
        | BridgeSessionFailureKind::InvalidControlPlaneResponse
        | BridgeSessionFailureKind::Internal => MessageContentReadFailureKind::Internal,
    };
    read_failure(kind)
}

fn read_status_failure(status: StatusCode) -> MessageContentReadFailure {
    let kind = match status {
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
            MessageContentReadFailureKind::InvalidRequest
        }
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => MessageContentReadFailureKind::Denied,
        StatusCode::NOT_FOUND | StatusCode::GONE => MessageContentReadFailureKind::NotFound,
        StatusCode::TOO_MANY_REQUESTS => MessageContentReadFailureKind::RateLimited,
        StatusCode::REQUEST_TIMEOUT
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => MessageContentReadFailureKind::Unavailable,
        _ if status.is_server_error() => MessageContentReadFailureKind::Unavailable,
        _ => MessageContentReadFailureKind::InvalidResponse,
    };
    read_failure(kind)
}

const fn read_failure(kind: MessageContentReadFailureKind) -> MessageContentReadFailure {
    MessageContentReadFailure::new(kind)
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use agent_room_application::{
        devices::{DeviceRequestProof, DeviceRequestProofPayload},
        ports::{DeviceSignature, PortFuture, SecretFactory, SecretValue},
    };
    use agent_room_bridge_core::{
        messages::{
            MessageContentReadFailureKind, MessageContentReadGateway, MessageContentReadRequest,
        },
        session::{
            AuthorizedControlPlaneRequest, BridgeSessionResult, ControlPlaneRequestAuthorizer,
        },
    };
    use agent_room_domain::{ids::ContentId, time::UtcMillis};
    use agent_room_identity_adapter::SecureSecretFactory;
    use axum::{
        Json, Router,
        body::Body,
        extract::Path,
        http::{HeaderMap, Response, StatusCode, header},
        response::IntoResponse,
        routing::post,
    };
    use serde_json::json;
    use tokio::net::TcpListener;
    use uuid::Uuid;

    use super::{
        CONTENT_DIGEST_HEADER, ControlPlaneHttpConfig, ReqwestControlPlaneContentGateway, STANDARD,
    };
    use base64::Engine as _;

    const CONTENT_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e48";
    const TICKET: &str = "content-read-ticket-0123456789";

    #[derive(Default)]
    struct 测试请求授权器 {
        requests: Mutex<Vec<(String, String, String)>>,
    }

    impl ControlPlaneRequestAuthorizer for 测试请求授权器 {
        fn authorize<'a>(
            &'a self,
            method: &'a str,
            request_target: &'a str,
            body: &'a str,
        ) -> PortFuture<'a, BridgeSessionResult<AuthorizedControlPlaneRequest>> {
            self.requests.lock().expect("授权记录锁可用").push((
                method.to_owned(),
                request_target.to_owned(),
                body.to_owned(),
            ));
            let payload = DeviceRequestProofPayload::new(
                agent_room_domain::ids::DeviceId::from_uuid(Uuid::from_u128(1)),
                UtcMillis::new(1_000).expect("测试时间有效"),
                secret("0123456789abcdef"),
                method.to_owned(),
                request_target.to_owned(),
                SecureSecretFactory.digest(body),
            )
            .expect("测试设备证明有效");
            Box::pin(async move {
                Ok(AuthorizedControlPlaneRequest {
                    access_token: secret("access-token"),
                    proof: DeviceRequestProof::new(
                        payload,
                        DeviceSignature::new(vec![5; 64]).expect("测试签名有效"),
                    ),
                })
            })
        }
    }

    #[tokio::test]
    async fn 正文读取先取一次性票据再对精确开封正文签名() {
        let digest = STANDARD.encode([7_u8; 32]);
        let app = Router::new()
            .route(
                "/content/{content_id}/read-tickets",
                post(
                    |Path(content_id): Path<String>, headers: HeaderMap, body: String| async move {
                        let valid = content_id == CONTENT_ID
                            && signed_headers_are_valid(&headers)
                            && body.is_empty();
                        if valid {
                            (
                                StatusCode::OK,
                                Json(json!({
                                    "ticket": TICKET,
                                    "expiresAtUnixMs": 2_000
                                })),
                            )
                                .into_response()
                        } else {
                            StatusCode::BAD_REQUEST.into_response()
                        }
                    },
                ),
            )
            .route(
                "/content/{content_id}/open",
                post(
                    move |Path(content_id): Path<String>, headers: HeaderMap, body: String| {
                        let digest = digest.clone();
                        async move {
                            let valid = content_id == CONTENT_ID
                                && signed_headers_are_valid(&headers)
                                && headers
                                    .get(header::CONTENT_TYPE)
                                    .and_then(|value| value.to_str().ok())
                                    == Some("application/json")
                                && body == format!(r#"{{"ticket":"{TICKET}"}}"#);
                            if !valid {
                                return StatusCode::BAD_REQUEST.into_response();
                            }
                            Response::builder()
                                .status(StatusCode::OK)
                                .header(header::CONTENT_TYPE, "text/plain")
                                .header(CONTENT_DIGEST_HEADER, format!("sha-256=:{digest}:"))
                                .body(Body::from("正文".as_bytes().to_vec()))
                                .expect("测试响应可构造")
                                .into_response()
                        }
                    },
                ),
            );
        let authorizer = Arc::new(测试请求授权器::default());
        let gateway = gateway(spawn_server(app).await, authorizer.clone());

        let opened = gateway
            .open(&MessageContentReadRequest::new(content_id(), 16))
            .await
            .expect("规范正文响应应成功");

        assert_eq!(&*opened.bytes, "正文".as_bytes());
        assert_eq!(opened.digest.as_bytes(), &[7_u8; 32]);
        assert_eq!(opened.media_type.as_str(), "text/plain");
        assert_eq!(opened.byte_length.value(), 6);
        assert_eq!(
            authorizer
                .requests
                .lock()
                .expect("授权记录锁可用")
                .as_slice(),
            [
                (
                    "POST".to_owned(),
                    format!("/content/{CONTENT_ID}/read-tickets"),
                    String::new(),
                ),
                (
                    "POST".to_owned(),
                    format!("/content/{CONTENT_ID}/open"),
                    format!(r#"{{"ticket":"{TICKET}"}}"#),
                ),
            ]
        );
    }

    #[tokio::test]
    async fn 正文读取拒绝畸形内容摘要头() {
        let app = Router::new()
            .route(
                "/content/{content_id}/read-tickets",
                post(|| async {
                    Json(json!({
                        "ticket": TICKET,
                        "expiresAtUnixMs": 2_000
                    }))
                }),
            )
            .route(
                "/content/{content_id}/open",
                post(|| async {
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "text/plain")
                        .header(CONTENT_DIGEST_HEADER, "sha-256=:invalid:")
                        .body(Body::from("正文".as_bytes().to_vec()))
                        .expect("测试响应可构造")
                }),
            );
        let gateway = gateway(spawn_server(app).await, Arc::new(测试请求授权器::default()));

        let failure = gateway
            .open(&MessageContentReadRequest::new(content_id(), 16))
            .await
            .expect_err("畸形摘要头必须失败");

        assert_eq!(
            failure.kind(),
            MessageContentReadFailureKind::InvalidResponse
        );
    }

    fn gateway(
        base_url: String,
        authorizer: Arc<测试请求授权器>,
    ) -> ReqwestControlPlaneContentGateway {
        ReqwestControlPlaneContentGateway::new(
            &ControlPlaneHttpConfig {
                base_url,
                request_timeout: Duration::from_secs(2),
            },
            authorizer,
        )
        .expect("本地正文网关地址有效")
    }

    async fn spawn_server(app: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("测试监听器可绑定");
        let address = listener.local_addr().expect("测试地址存在");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("测试服务器应正常运行");
        });
        format!("http://{address}/")
    }

    fn signed_headers_are_valid(headers: &HeaderMap) -> bool {
        header_value(headers, "authorization") == Some("Bearer access-token")
            && header_value(headers, "x-agent-room-device-id")
                == Some("00000000-0000-0000-0000-000000000001")
            && header_value(headers, "x-agent-room-proof-issued-at") == Some("1000")
            && header_value(headers, "x-agent-room-proof-nonce") == Some("0123456789abcdef")
            && header_value(headers, "x-agent-room-proof-signature")
                == Some(
                    "BQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQ",
                )
    }

    fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
        headers.get(name).and_then(|value| value.to_str().ok())
    }

    fn content_id() -> ContentId {
        ContentId::from_uuid(Uuid::parse_str(CONTENT_ID).expect("测试正文 ID 有效"))
    }

    fn secret(value: &str) -> SecretValue {
        SecretValue::new(value.to_owned()).expect("测试密文有效")
    }
}
