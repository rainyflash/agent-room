use std::sync::Arc;

use agent_room_bridge_core::{
    messages::{
        MessageContentBindRequest, MessageContentFailure, MessageContentFailureKind,
        MessageContentGateway, MessageContentRecord, MessageContentRedactRequest,
        MessageContentUploadRequest,
    },
    session::{
        AuthorizedControlPlaneRequest, BridgeSessionFailure, BridgeSessionFailureKind,
        ControlPlaneRequestAuthorizer,
    },
};
use agent_room_domain::{content::Sha256Digest, ids::ContentId};
use reqwest::{Client, StatusCode, header};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};
use url::Url;
use uuid::{Uuid, Version};

use super::{
    ControlPlaneHttpConfig, ControlPlaneHttpConfigurationError, IDEMPOTENCY_KEY_HEADER,
    configured_client, signed_request_headers,
};

const ACCESS_MODE: &str = "room_member";
const CONTENT_SHA256_HEADER: &str = "x-agent-room-content-sha256";
const CONTENT_BYTE_LENGTH_HEADER: &str = "x-agent-room-content-byte-length";
const MAX_RESPONSE_BYTES: usize = 32 * 1_024;

pub struct ReqwestControlPlaneMessageContentGateway {
    client: Client,
    base_url: Url,
    authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
}

impl ReqwestControlPlaneMessageContentGateway {
    /// 创建以 Bridge 设备会话签名的消息正文写入网关。
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

    async fn upload_internal(
        &self,
        request: &MessageContentUploadRequest,
    ) -> Result<MessageContentRecord, MessageContentFailure> {
        validate_upload_request(request)?;
        let content_id = self.begin_upload(request).await?;
        self.complete_upload(content_id, request).await
    }

    async fn begin_upload(
        &self,
        request: &MessageContentUploadRequest,
    ) -> Result<ContentId, MessageContentFailure> {
        let target = "/content/uploads";
        let digest = encode_hex(request.digest.as_bytes());
        let body = serde_json::to_string(&BeginUploadBody {
            matrix_room_id: request.room_id.as_str(),
            access_mode: ACCESS_MODE,
            sha256: &digest,
            byte_length: request.byte_length.value(),
            media_type: request.media_type.as_str(),
            encryption_mode: request.encryption_mode.as_str(),
            expires_at_unix_ms: request
                .expires_at
                .map(agent_room_domain::time::UtcMillis::value),
        })
        .map_err(|_| failure(MessageContentFailureKind::Internal))?;
        let authorized = self.authorize("POST", target, &body).await?;
        let response = Self::signed_request(
            self.client
                .post(self.url(target)?)
                .header(header::ACCEPT, "application/json")
                .header(header::CONTENT_TYPE, "application/json")
                .header(IDEMPOTENCY_KEY_HEADER, request.request_id.to_string()),
            &authorized,
            "POST",
            target,
        )?
        .body(body)
        .send()
        .await
        .map_err(|error| write_transport_failure(&error))?;
        let status = response.status();
        if !status.is_success() {
            return Err(write_status_failure(status));
        }
        let decoded = decode_json::<BeginUploadResponse>(response).await?;
        let expected_created = status == StatusCode::CREATED;
        if decoded.created != expected_created
            || decoded.matrix_room_id != request.room_id.as_str()
            || decoded.access_mode != ACCESS_MODE
            || !decoded.content.matches_declaration(request)
            || !decoded.content.is_uploadable()
        {
            return Err(failure(MessageContentFailureKind::Internal));
        }
        decoded.content.content_id()
    }

    async fn complete_upload(
        &self,
        content_id: ContentId,
        request: &MessageContentUploadRequest,
    ) -> Result<MessageContentRecord, MessageContentFailure> {
        let target = format!("/content/{content_id}/bytes");
        let digest = encode_hex(request.digest.as_bytes());
        let byte_length = request.byte_length.value().to_string();
        let proof_body = format!("sha256={digest}\nbyte-length={byte_length}");
        let authorized = self.authorize("PUT", &target, &proof_body).await?;
        let response = Self::signed_request(
            self.client
                .put(self.url(&target)?)
                .header(header::ACCEPT, "application/json")
                .header(header::CONTENT_TYPE, request.media_type.as_str())
                .header(header::CONTENT_LENGTH, &byte_length)
                .header(CONTENT_SHA256_HEADER, &digest)
                .header(CONTENT_BYTE_LENGTH_HEADER, &byte_length),
            &authorized,
            "PUT",
            &target,
        )?
        .body(request.body.to_vec())
        .send()
        .await
        .map_err(|error| write_transport_failure(&error))?;
        if !response.status().is_success() {
            return Err(write_status_failure(response.status()));
        }
        let decoded = decode_json::<CompleteUploadResponse>(response).await?;
        if !decoded.content.matches_declaration(request)
            || decoded.content.parsed_content_id()? != content_id
            || !decoded.content.is_active()
        {
            return Err(failure(MessageContentFailureKind::Internal));
        }
        Ok(MessageContentRecord {
            content_id,
            digest: request.digest,
            byte_length: request.byte_length,
            media_type: request.media_type.clone(),
        })
    }

    async fn bind_internal(
        &self,
        request: &MessageContentBindRequest,
    ) -> Result<(), MessageContentFailure> {
        let target = format!("/content/{}/event-binding", request.content_id);
        let body = serde_json::to_string(&BindEventBody {
            matrix_room_id: request.room_id.as_str(),
            matrix_event_id: request.event_id.as_str(),
        })
        .map_err(|_| failure(MessageContentFailureKind::Internal))?;
        let authorized = self.authorize("PUT", &target, &body).await?;
        let response = Self::signed_request(
            self.client
                .put(self.url(&target)?)
                .header(header::ACCEPT, "application/json")
                .header(header::CONTENT_TYPE, "application/json"),
            &authorized,
            "PUT",
            &target,
        )?
        .body(body)
        .send()
        .await
        .map_err(|error| write_transport_failure(&error))?;
        if !response.status().is_success() {
            return Err(write_status_failure(response.status()));
        }
        let decoded = decode_json::<BindEventResponse>(response).await?;
        let parsed_id = parse_content_id(&decoded.content_id)?;
        if parsed_id != request.content_id
            || decoded.matrix_room_id != request.room_id.as_str()
            || decoded.matrix_event_id != request.event_id.as_str()
            || decoded.access_mode != ACCESS_MODE
        {
            return Err(failure(MessageContentFailureKind::Internal));
        }
        Ok(())
    }

    async fn redact_internal(
        &self,
        request: &MessageContentRedactRequest,
    ) -> Result<(), MessageContentFailure> {
        let target = format!("/content/{}", request.content_id);
        let authorized = self.authorize("DELETE", &target, "").await?;
        let response = Self::signed_request(
            self.client.delete(self.url(&target)?),
            &authorized,
            "DELETE",
            &target,
        )?
        .body(Vec::new())
        .send()
        .await
        .map_err(|error| write_transport_failure(&error))?;
        if !response.status().is_success() {
            return Err(write_status_failure(response.status()));
        }
        let decoded = decode_json::<RedactContentResponse>(response).await?;
        if parse_content_id(&decoded.content_id)? != request.content_id
            || decoded.lifecycle_state != "redacted"
        {
            return Err(failure(MessageContentFailureKind::Internal));
        }
        Ok(())
    }

    async fn authorize(
        &self,
        method: &str,
        target: &str,
        body: &str,
    ) -> Result<AuthorizedControlPlaneRequest, MessageContentFailure> {
        self.authorizer
            .authorize(method, target, body)
            .await
            .map_err(map_session_failure)
    }

    fn url(&self, target: &str) -> Result<Url, MessageContentFailure> {
        self.base_url
            .join(target.trim_start_matches('/'))
            .map_err(|_| failure(MessageContentFailureKind::Internal))
    }

    fn signed_request(
        request: reqwest::RequestBuilder,
        authorized: &AuthorizedControlPlaneRequest,
        method: &str,
        target: &str,
    ) -> Result<reqwest::RequestBuilder, MessageContentFailure> {
        signed_request_headers(request, authorized, method, target)
            .map_err(|()| failure(MessageContentFailureKind::Internal))
    }
}

impl MessageContentGateway for ReqwestControlPlaneMessageContentGateway {
    fn upload<'a>(
        &'a self,
        request: &'a MessageContentUploadRequest,
    ) -> agent_room_application::ports::PortFuture<
        'a,
        Result<MessageContentRecord, MessageContentFailure>,
    > {
        Box::pin(self.upload_internal(request))
    }

    fn bind<'a>(
        &'a self,
        request: &'a MessageContentBindRequest,
    ) -> agent_room_application::ports::PortFuture<'a, Result<(), MessageContentFailure>> {
        Box::pin(self.bind_internal(request))
    }

    fn redact<'a>(
        &'a self,
        request: &'a MessageContentRedactRequest,
    ) -> agent_room_application::ports::PortFuture<'a, Result<(), MessageContentFailure>> {
        Box::pin(self.redact_internal(request))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BeginUploadBody<'a> {
    matrix_room_id: &'a str,
    access_mode: &'static str,
    sha256: &'a str,
    byte_length: u64,
    media_type: &'a str,
    encryption_mode: &'static str,
    expires_at_unix_ms: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BindEventBody<'a> {
    matrix_room_id: &'a str,
    matrix_event_id: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ContentObjectResponse {
    content_id: String,
    sha256: String,
    byte_length: u64,
    media_type: String,
    encryption_mode: String,
    scan_state: String,
    lifecycle_state: String,
    expires_at_unix_ms: Option<i64>,
    created_at_unix_ms: i64,
}

impl ContentObjectResponse {
    fn matches_declaration(&self, request: &MessageContentUploadRequest) -> bool {
        self.sha256 == encode_hex(request.digest.as_bytes())
            && self.byte_length == request.byte_length.value()
            && self.media_type == request.media_type.as_str()
            && self.encryption_mode == request.encryption_mode.as_str()
            && self.expires_at_unix_ms
                == request
                    .expires_at
                    .map(agent_room_domain::time::UtcMillis::value)
            && self.created_at_unix_ms >= 0
    }

    fn is_uploadable(&self) -> bool {
        matches!(
            (self.lifecycle_state.as_str(), self.scan_state.as_str()),
            ("uploading", "pending") | ("active", "clean")
        )
    }

    fn is_active(&self) -> bool {
        self.lifecycle_state == "active" && self.scan_state == "clean"
    }

    fn content_id(&self) -> Result<ContentId, MessageContentFailure> {
        self.parsed_content_id()
    }

    fn parsed_content_id(&self) -> Result<ContentId, MessageContentFailure> {
        parse_content_id(&self.content_id)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BeginUploadResponse {
    #[serde(flatten)]
    content: ContentObjectResponse,
    matrix_room_id: String,
    access_mode: String,
    created: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompleteUploadResponse {
    #[serde(flatten)]
    content: ContentObjectResponse,
    #[serde(rename = "alreadyActive")]
    _already_active: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BindEventResponse {
    content_id: String,
    matrix_room_id: String,
    matrix_event_id: String,
    access_mode: String,
    #[serde(rename = "alreadyBound")]
    _already_bound: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RedactContentResponse {
    content_id: String,
    lifecycle_state: String,
    #[serde(rename = "alreadyRedacted")]
    _already_redacted: bool,
}

async fn decode_json<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, MessageContentFailure> {
    let body = read_limited_body(response).await?;
    serde_json::from_slice(&body).map_err(|_| failure(MessageContentFailureKind::Internal))
}

async fn read_limited_body(
    mut response: reqwest::Response,
) -> Result<Vec<u8>, MessageContentFailure> {
    if response.content_length().is_some_and(|length| {
        usize::try_from(length).map_or(true, |value| value > MAX_RESPONSE_BYTES)
    }) {
        return Err(failure(MessageContentFailureKind::Internal));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| failure(MessageContentFailureKind::UnknownCommit))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(failure(MessageContentFailureKind::Internal));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn validate_upload_request(
    request: &MessageContentUploadRequest,
) -> Result<(), MessageContentFailure> {
    let byte_length = u64::try_from(request.body.len())
        .map_err(|_| failure(MessageContentFailureKind::InvalidRequest))?;
    let actual_digest = Sha256Digest::from_bytes(Sha256::digest(&request.body).into());
    if byte_length != request.byte_length.value() || actual_digest != request.digest {
        return Err(failure(MessageContentFailureKind::InvalidRequest));
    }
    Ok(())
}

fn parse_content_id(value: &str) -> Result<ContentId, MessageContentFailure> {
    let id = Uuid::parse_str(value).map_err(|_| failure(MessageContentFailureKind::Internal))?;
    if id.get_version() != Some(Version::SortRand) || id.to_string() != value {
        return Err(failure(MessageContentFailureKind::Internal));
    }
    Ok(ContentId::from_uuid(id))
}

fn map_session_failure(failure_value: BridgeSessionFailure) -> MessageContentFailure {
    let kind = match failure_value.kind() {
        BridgeSessionFailureKind::NotAuthorized
        | BridgeSessionFailureKind::RefreshOutcomeUnknown => MessageContentFailureKind::Denied,
        BridgeSessionFailureKind::SecureStorageUnavailable
        | BridgeSessionFailureKind::ControlPlaneUnavailable => {
            MessageContentFailureKind::Unavailable
        }
        BridgeSessionFailureKind::CorruptSecureStorage
        | BridgeSessionFailureKind::InvalidControlPlaneResponse
        | BridgeSessionFailureKind::Internal => MessageContentFailureKind::Internal,
    };
    failure(kind)
}

fn write_transport_failure(error: &reqwest::Error) -> MessageContentFailure {
    if error.is_connect() {
        failure(MessageContentFailureKind::Unavailable)
    } else {
        failure(MessageContentFailureKind::UnknownCommit)
    }
}

fn write_status_failure(status: StatusCode) -> MessageContentFailure {
    let kind = match status {
        StatusCode::BAD_REQUEST
        | StatusCode::NOT_FOUND
        | StatusCode::METHOD_NOT_ALLOWED
        | StatusCode::UNPROCESSABLE_ENTITY => MessageContentFailureKind::InvalidRequest,
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => MessageContentFailureKind::Denied,
        StatusCode::CONFLICT => MessageContentFailureKind::Conflict,
        StatusCode::REQUEST_TIMEOUT
        | StatusCode::TOO_MANY_REQUESTS
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => MessageContentFailureKind::Unavailable,
        _ if status.is_server_error() => MessageContentFailureKind::Unavailable,
        _ => MessageContentFailureKind::Internal,
    };
    failure(kind)
}

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

const fn failure(kind: MessageContentFailureKind) -> MessageContentFailure {
    MessageContentFailure::new(kind)
}
