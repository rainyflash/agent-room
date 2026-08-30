use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    content::{
        BeginContentUploadOutcome, BeginContentUploadRequest, BindContentEventOutcome,
        BindContentEventRequest, CompleteContentUploadOutcome, CompleteContentUploadRequest,
        ContentUseCases, IssueContentReadTicketRequest, OpenContentRequest, RedactContentOutcome,
        RedactContentRequest,
    },
    devices::DeviceAuthorizationUseCases,
    ports::{
        ContentAccessMode, ContentByteStream, ContentReadTicket, ContentStreamFailure,
        ContentStreamFailureKind, MatrixEventId, MatrixRoomId, SecretFactory,
    },
};
use agent_room_domain::{
    content::{
        ContentByteLength, ContentEncryptionMode, ContentMediaType, ContentObject, Sha256Digest,
    },
    ids::{AgentId, ContentId, ContentUploadRequestId, PrincipalId},
    time::UtcMillis,
};
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Extension, Path, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, post, put},
};
use axum_extra::extract::CookieJar;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
        devices::authenticate_signed_device_request,
        resource_ids::parse_uuid_v7,
    },
};

const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";
const CONTENT_SHA256_HEADER: &str = "x-agent-room-content-sha256";
const CONTENT_BYTE_LENGTH_HEADER: &str = "x-agent-room-content-byte-length";
const CONTENT_DIGEST_HEADER: HeaderName = HeaderName::from_static("content-digest");
const MAX_CONTENT_JSON_BYTES: usize = 32 * 1_024;

#[derive(Clone)]
pub(crate) struct ContentHttpState {
    content: Arc<dyn ContentUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    devices: Arc<dyn DeviceAuthorizationUseCases>,
    secrets: Arc<dyn SecretFactory>,
    trusted_origins: TrustedOrigins,
}

pub(crate) struct ContentHttpDependencies {
    pub(crate) content: Arc<dyn ContentUseCases>,
    pub(crate) authentication: Arc<dyn AuthenticationUseCases>,
    pub(crate) devices: Arc<dyn DeviceAuthorizationUseCases>,
    pub(crate) secrets: Arc<dyn SecretFactory>,
}

impl ContentHttpState {
    pub(crate) fn new(
        dependencies: ContentHttpDependencies,
        frontend_origin: &url::Url,
        desktop_origin: &url::Url,
    ) -> Self {
        Self {
            content: dependencies.content,
            authentication: dependencies.authentication,
            devices: dependencies.devices,
            secrets: dependencies.secrets,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origin),
        }
    }
}

pub(crate) fn router(state: ContentHttpState) -> Router {
    let json_routes = Router::new()
        .route("/content/uploads", post(begin_upload))
        .route(
            "/content/{content_id}/event-binding",
            put(bind_content_event),
        )
        .route(
            "/content/{content_id}/read-tickets",
            post(issue_read_ticket),
        )
        .route("/content/{content_id}", delete(redact_content))
        .route("/content/{content_id}/open", post(open_content))
        .layer(DefaultBodyLimit::max(MAX_CONTENT_JSON_BYTES));
    let streaming_routes = Router::new().route(
        "/content/{content_id}/bytes",
        put(complete_upload).layer(DefaultBodyLimit::disable()),
    );
    Router::new()
        .merge(json_routes)
        .merge(streaming_routes)
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BeginUploadBody {
    #[serde(default)]
    actor_agent_id: Option<String>,
    matrix_room_id: String,
    access_mode: String,
    sha256: String,
    byte_length: u64,
    media_type: String,
    encryption_mode: String,
    #[serde(default)]
    expires_at_unix_ms: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadTicketBody {
    #[serde(default)]
    actor_agent_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BindEventBody {
    matrix_room_id: String,
    matrix_event_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenContentBody {
    ticket: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ContentObjectResponse {
    content_id: String,
    sha256: String,
    byte_length: u64,
    media_type: String,
    encryption_mode: &'static str,
    scan_state: &'static str,
    lifecycle_state: &'static str,
    expires_at_unix_ms: Option<i64>,
    created_at_unix_ms: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BeginUploadResponse {
    #[serde(flatten)]
    content: ContentObjectResponse,
    matrix_room_id: String,
    access_mode: &'static str,
    created: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompleteUploadResponse {
    #[serde(flatten)]
    content: ContentObjectResponse,
    already_active: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BindEventResponse {
    content_id: String,
    matrix_room_id: String,
    matrix_event_id: String,
    access_mode: &'static str,
    already_bound: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadTicketResponse {
    ticket: String,
    expires_at_unix_ms: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RedactContentResponse {
    content_id: String,
    lifecycle_state: &'static str,
    already_redacted: bool,
}

async fn begin_upload(
    State(state): State<ContentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let Ok(request_id) = idempotency_request_id(&headers) else {
        return invalid_request("content.upload.invalid_idempotency_key", correlation_id);
    };
    let Ok(body_text) = std::str::from_utf8(&body) else {
        return invalid_request("content.upload.invalid_body", correlation_id);
    };
    let principal_id = match authenticate_content_request(
        &state,
        &headers,
        &jar,
        "POST",
        "/content/uploads",
        body_text,
        correlation_id,
    )
    .await
    {
        Ok(principal_id) => principal_id,
        Err(response) => return response,
    };
    let Ok(body) = serde_json::from_slice::<BeginUploadBody>(&body) else {
        return invalid_request("content.upload.invalid_body", correlation_id);
    };
    let Ok(request) = begin_upload_request(request_id, principal_id, body) else {
        return invalid_request("content.upload.invalid_body", correlation_id);
    };

    match state.content.begin_upload(request).await {
        Ok(outcome) => {
            let (status, response) = begin_upload_response(outcome);
            no_store((status, axum::Json(response)).into_response())
        }
        Err(failure) => {
            no_store(ApiError::begin_content_upload(&failure, correlation_id).into_response())
        }
    }
}

async fn complete_upload(
    State(state): State<ContentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(content_id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Body,
) -> Response {
    let Ok(content_id) = parse_content_id(&content_id) else {
        return invalid_request("content.invalid_content_id", correlation_id);
    };
    let request_target = format!("/content/{content_id}/bytes");
    let proof_body = if is_device_request(&headers) {
        match streaming_proof_body(&headers) {
            Ok(value) => value,
            Err(()) => {
                return invalid_request("content.upload.invalid_integrity_headers", correlation_id);
            }
        }
    } else {
        String::new()
    };
    let principal_id = match authenticate_content_request(
        &state,
        &headers,
        &jar,
        "PUT",
        &request_target,
        &proof_body,
        correlation_id,
    )
    .await
    {
        Ok(principal_id) => principal_id,
        Err(response) => return response,
    };
    let body: ContentByteStream = Box::pin(body.into_data_stream().map(|chunk| {
        chunk.map(|bytes| bytes.to_vec()).map_err(|_| {
            ContentStreamFailure::new(
                "content.http.request_body",
                ContentStreamFailureKind::Source,
            )
        })
    }));

    match state
        .content
        .complete_upload(CompleteContentUploadRequest {
            principal_id,
            content_id,
            body,
        })
        .await
    {
        Ok(outcome) => no_store(axum::Json(complete_upload_response(outcome)).into_response()),
        Err(failure) => {
            no_store(ApiError::complete_content_upload(&failure, correlation_id).into_response())
        }
    }
}

async fn bind_content_event(
    State(state): State<ContentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(content_id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let Ok(content_id) = parse_content_id(&content_id) else {
        return invalid_request("content.invalid_content_id", correlation_id);
    };
    let Ok(body_text) = std::str::from_utf8(&body) else {
        return invalid_request("content.binding.invalid_body", correlation_id);
    };
    let request_target = format!("/content/{content_id}/event-binding");
    let principal_id = match authenticate_content_request(
        &state,
        &headers,
        &jar,
        "PUT",
        &request_target,
        body_text,
        correlation_id,
    )
    .await
    {
        Ok(principal_id) => principal_id,
        Err(response) => return response,
    };
    let Ok(body) = serde_json::from_slice::<BindEventBody>(&body) else {
        return invalid_request("content.binding.invalid_body", correlation_id);
    };
    let Ok(matrix_room_id) = MatrixRoomId::new(body.matrix_room_id) else {
        return invalid_request("content.binding.invalid_body", correlation_id);
    };
    let Ok(matrix_event_id) = MatrixEventId::new(body.matrix_event_id) else {
        return invalid_request("content.binding.invalid_body", correlation_id);
    };

    match state
        .content
        .bind_event(BindContentEventRequest {
            principal_id,
            content_id,
            matrix_room_id,
            matrix_event_id,
        })
        .await
    {
        Ok(outcome) => no_store(axum::Json(bind_event_response(outcome)).into_response()),
        Err(failure) => {
            no_store(ApiError::bind_content_event(&failure, correlation_id).into_response())
        }
    }
}

async fn issue_read_ticket(
    State(state): State<ContentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(content_id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let Ok(content_id) = parse_content_id(&content_id) else {
        return invalid_request("content.invalid_content_id", correlation_id);
    };
    let Ok(body_text) = std::str::from_utf8(&body) else {
        return invalid_request("content.ticket.invalid_body", correlation_id);
    };
    let request_target = format!("/content/{content_id}/read-tickets");
    let principal_id = match authenticate_content_request(
        &state,
        &headers,
        &jar,
        "POST",
        &request_target,
        body_text,
        correlation_id,
    )
    .await
    {
        Ok(principal_id) => principal_id,
        Err(response) => return response,
    };
    let body = if body.is_empty() {
        ReadTicketBody::default()
    } else {
        match serde_json::from_slice::<ReadTicketBody>(&body) {
            Ok(body) => body,
            Err(_) => return invalid_request("content.ticket.invalid_body", correlation_id),
        }
    };
    let Ok(actor_agent_id) = parse_optional_agent_id(body.actor_agent_id.as_deref()) else {
        return invalid_request("content.ticket.invalid_body", correlation_id);
    };

    match state
        .content
        .issue_read_ticket(IssueContentReadTicketRequest {
            principal_id,
            actor_agent_id,
            content_id,
        })
        .await
    {
        Ok(issued) => no_store(
            axum::Json(ReadTicketResponse {
                ticket: issued.ticket.expose().to_owned(),
                expires_at_unix_ms: issued.expires_at.value(),
            })
            .into_response(),
        ),
        Err(failure) => {
            no_store(ApiError::issue_content_ticket(&failure, correlation_id).into_response())
        }
    }
}

async fn redact_content(
    State(state): State<ContentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(content_id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let Ok(content_id) = parse_content_id(&content_id) else {
        return invalid_request("content.invalid_content_id", correlation_id);
    };
    let request_target = format!("/content/{content_id}");
    let principal_id = match authenticate_content_request(
        &state,
        &headers,
        &jar,
        "DELETE",
        &request_target,
        "",
        correlation_id,
    )
    .await
    {
        Ok(principal_id) => principal_id,
        Err(response) => return response,
    };

    match state
        .content
        .redact(RedactContentRequest {
            principal_id,
            content_id,
        })
        .await
    {
        Ok(outcome) => no_store(axum::Json(redact_content_response(outcome)).into_response()),
        Err(failure) => {
            no_store(ApiError::redact_content(&failure, correlation_id).into_response())
        }
    }
}

async fn open_content(
    State(state): State<ContentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(content_id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let Ok(content_id) = parse_content_id(&content_id) else {
        return invalid_request("content.invalid_content_id", correlation_id);
    };
    let Ok(body_text) = std::str::from_utf8(&body) else {
        return invalid_request("content.open.invalid_body", correlation_id);
    };
    let request_target = format!("/content/{content_id}/open");
    let principal_id = match authenticate_content_request(
        &state,
        &headers,
        &jar,
        "POST",
        &request_target,
        body_text,
        correlation_id,
    )
    .await
    {
        Ok(principal_id) => principal_id,
        Err(response) => return response,
    };
    let Ok(body) = serde_json::from_slice::<OpenContentBody>(&body) else {
        return invalid_request("content.open.invalid_body", correlation_id);
    };
    let Ok(ticket) = ContentReadTicket::new(body.ticket) else {
        return invalid_request("content.open.invalid_ticket", correlation_id);
    };

    match state
        .content
        .open(OpenContentRequest {
            principal_id,
            content_id,
            ticket,
        })
        .await
    {
        Ok(opened) => verified_content_response(opened),
        Err(failure) => no_store(ApiError::open_content(&failure, correlation_id).into_response()),
    }
}

async fn authenticate_content_request(
    state: &ContentHttpState,
    headers: &HeaderMap,
    jar: &CookieJar,
    method: &str,
    request_target: &str,
    signed_body: &str,
    correlation_id: CorrelationId,
) -> Result<PrincipalId, Response> {
    if is_device_request(headers) {
        return authenticate_signed_device_request(
            state.devices.as_ref(),
            state.secrets.as_ref(),
            headers,
            method,
            request_target,
            signed_body,
            correlation_id,
        )
        .await
        .map(|device| device.account.principal.id());
    }
    if !origin_matches(headers, &state.trusted_origins) {
        return Err(no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "content.invalid_origin",
                agent_room_protocol_conformance::generated::ErrorCategory::Authorization,
                "内容请求来源无效。",
                correlation_id,
            )
            .into_response(),
        ));
    }
    authenticate_session(
        state.authentication.as_ref(),
        jar,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
    .map(|principal| principal.principal_id)
}

fn is_device_request(headers: &HeaderMap) -> bool {
    headers.contains_key(header::AUTHORIZATION)
}

fn begin_upload_request(
    request_id: ContentUploadRequestId,
    owner_principal_id: PrincipalId,
    body: BeginUploadBody,
) -> Result<BeginContentUploadRequest, ()> {
    Ok(BeginContentUploadRequest {
        request_id,
        owner_principal_id,
        actor_agent_id: parse_optional_agent_id(body.actor_agent_id.as_deref())?,
        matrix_room_id: MatrixRoomId::new(body.matrix_room_id).map_err(|_| ())?,
        access_mode: ContentAccessMode::try_from(body.access_mode.as_str()).map_err(|_| ())?,
        digest: parse_sha256(&body.sha256)?,
        byte_length: ContentByteLength::new(body.byte_length).map_err(|_| ())?,
        media_type: ContentMediaType::new(body.media_type).map_err(|_| ())?,
        encryption_mode: ContentEncryptionMode::try_from(body.encryption_mode.as_str())
            .map_err(|_| ())?,
        expires_at: body
            .expires_at_unix_ms
            .map(UtcMillis::new)
            .transpose()
            .map_err(|_| ())?,
    })
}

fn parse_optional_agent_id(value: Option<&str>) -> Result<Option<AgentId>, ()> {
    value
        .map(parse_uuid_v7)
        .transpose()
        .map(|value| value.map(AgentId::from_uuid))
}

fn idempotency_request_id(headers: &HeaderMap) -> Result<ContentUploadRequestId, ()> {
    let value = headers
        .get(IDEMPOTENCY_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or(())?;
    parse_uuid_v7(value).map(ContentUploadRequestId::from_uuid)
}

fn parse_content_id(value: &str) -> Result<ContentId, ()> {
    parse_uuid_v7(value).map(ContentId::from_uuid)
}

fn parse_sha256(value: &str) -> Result<Sha256Digest, ()> {
    if value.len() != 64 {
        return Err(());
    }
    let mut digest = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        digest[index] = decode_hex_nibble(pair[0])?
            .checked_mul(16)
            .and_then(|high| high.checked_add(decode_hex_nibble(pair[1]).ok()?))
            .ok_or(())?;
    }
    Ok(Sha256Digest::from_bytes(digest))
}

fn decode_hex_nibble(value: u8) -> Result<u8, ()> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(()),
    }
}

fn sha256_hex(digest: Sha256Digest) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for byte in digest.as_bytes() {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn streaming_proof_body(headers: &HeaderMap) -> Result<String, ()> {
    let digest = required_header(headers, CONTENT_SHA256_HEADER)?;
    parse_sha256(digest)?;
    let byte_length = required_header(headers, CONTENT_BYTE_LENGTH_HEADER)?
        .parse::<u64>()
        .map_err(|_| ())?;
    ContentByteLength::new(byte_length).map_err(|_| ())?;
    if let Some(content_length) = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
    {
        let content_length = content_length.parse::<u64>().map_err(|_| ())?;
        if content_length != byte_length {
            return Err(());
        }
    }
    Ok(format!("sha256={digest}\nbyte-length={byte_length}"))
}

fn required_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, ()> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .ok_or(())
}

fn begin_upload_response(outcome: BeginContentUploadOutcome) -> (StatusCode, BeginUploadResponse) {
    let (content, access_policy, created, status) = match outcome {
        BeginContentUploadOutcome::Created {
            content,
            access_policy,
        } => (content, access_policy, true, StatusCode::CREATED),
        BeginContentUploadOutcome::Existing {
            content,
            access_policy,
        } => (content, access_policy, false, StatusCode::OK),
    };
    (
        status,
        BeginUploadResponse {
            content: ContentObjectResponse::from(&content),
            matrix_room_id: access_policy.matrix_room_id().as_str().to_owned(),
            access_mode: access_policy.access_mode().as_str(),
            created,
        },
    )
}

fn complete_upload_response(outcome: CompleteContentUploadOutcome) -> CompleteUploadResponse {
    let (content, already_active) = match outcome {
        CompleteContentUploadOutcome::Activated(content) => (content, false),
        CompleteContentUploadOutcome::AlreadyActive(content) => (content, true),
    };
    CompleteUploadResponse {
        content: ContentObjectResponse::from(&content),
        already_active,
    }
}

fn bind_event_response(outcome: BindContentEventOutcome) -> BindEventResponse {
    let (policy, already_bound) = match outcome {
        BindContentEventOutcome::Bound(policy) => (policy, false),
        BindContentEventOutcome::AlreadyBound(policy) => (policy, true),
    };
    BindEventResponse {
        content_id: policy.content_id().to_string(),
        matrix_room_id: policy.matrix_room_id().as_str().to_owned(),
        matrix_event_id: policy
            .matrix_event_id()
            .expect("绑定用例成功结果必须包含事件标识")
            .as_str()
            .to_owned(),
        access_mode: policy.access_mode().as_str(),
        already_bound,
    }
}

fn redact_content_response(outcome: RedactContentOutcome) -> RedactContentResponse {
    let (content, already_redacted) = match outcome {
        RedactContentOutcome::Redacted(content) => (content, false),
        RedactContentOutcome::AlreadyRedacted(content) => (content, true),
    };
    RedactContentResponse {
        content_id: content.id().to_string(),
        lifecycle_state: content.lifecycle_state().as_str(),
        already_redacted,
    }
}

fn verified_content_response(
    opened: agent_room_application::content::OpenedVerifiedContent,
) -> Response {
    let digest = STANDARD.encode(opened.digest.as_bytes());
    let media_type = HeaderValue::from_str(opened.media_type.as_str())
        .expect("领域白名单媒体类型必须是合法响应头");
    let byte_length = HeaderValue::from_str(&opened.byte_length.value().to_string())
        .expect("有效内容长度必须是合法响应头");
    let content_digest =
        HeaderValue::from_str(&format!("sha-256=:{digest}:")).expect("Base64 摘要必须是合法响应头");
    let mut response = Response::new(Body::from_stream(opened.body));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, media_type);
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, byte_length);
    response
        .headers_mut()
        .insert(CONTENT_DIGEST_HEADER, content_digest);
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    no_store(response)
}

fn invalid_request(code: &'static str, correlation_id: CorrelationId) -> Response {
    no_store(ApiError::invalid_request(code, correlation_id).into_response())
}

impl From<&ContentObject> for ContentObjectResponse {
    fn from(content: &ContentObject) -> Self {
        Self {
            content_id: content.id().to_string(),
            sha256: sha256_hex(content.digest()),
            byte_length: content.byte_length().value(),
            media_type: content.media_type().as_str().to_owned(),
            encryption_mode: content.encryption_mode().as_str(),
            scan_state: content.scan_state().as_str(),
            lifecycle_state: content.lifecycle_state().as_str(),
            expires_at_unix_ms: content.expires_at().map(UtcMillis::value),
            created_at_unix_ms: content.created_at().value(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use agent_room_application::{
        authentication::{
            AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
            AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
        },
        content::{
            BeginContentUploadOutcome, BeginContentUploadRequest, BeginContentUploadResult,
            BindContentEventOutcome, BindContentEventRequest, BindContentEventResult,
            CompleteContentUploadOutcome, CompleteContentUploadRequest,
            CompleteContentUploadResult, ContentUseCases, IssueContentReadTicketRequest,
            IssueContentReadTicketResult, IssuedContentReadTicket, OpenContentRequest,
            OpenContentResult, OpenedVerifiedContent, RedactContentOutcome, RedactContentRequest,
            RedactContentResult,
        },
        devices::{
            AuthenticateDeviceRequest, AuthenticatedDevice, DeviceAuthorizationResult,
            DeviceAuthorizationUseCases, DeviceCredentials, RefreshDeviceSession, RegisterDevice,
            RevokedDevice,
        },
        ports::{
            ContentAccessMode, ContentAccessPolicy, ContentReadTicket, MatrixRoomId, PortFuture,
            PrincipalAccount, SecretFactory, SecretValue,
        },
    };
    use agent_room_domain::{
        content::{
            ContentByteLength, ContentEncryptionMode, ContentLifecycleState, ContentMediaType,
            ContentObject, ContentObjectFields, ContentScanState, ContentStorageKey, Sha256Digest,
        },
        devices::Device,
        identity::Principal,
        ids::{AgentId, ContentId, DeviceId, PrincipalId},
        time::UtcMillis,
    };
    use agent_room_identity_adapter::SecureSecretFactory;
    use axum::{
        body::{Body, to_bytes},
        http::{HeaderValue, Request, StatusCode, header},
        middleware,
        response::Response,
    };
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use futures_util::{StreamExt, stream};
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use url::Url;
    use uuid::Uuid;

    use super::{ContentHttpDependencies, ContentHttpState, router};

    const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
    const DEVICE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e43";
    const CONTENT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
    const REQUEST_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e45";
    const AGENT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
    const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";
    const SESSION_COOKIE: &str = "__Host-agent-room-session=session-secret";
    const ROOM_ID: &str = "!room:matrix.agent-room.test";
    const DIGEST_HEX: &str = "0707070707070707070707070707070707070707070707070707070707070707";

    struct FakeContent {
        content: ContentObject,
        policy: ContentAccessPolicy,
        begin_request: Mutex<Option<BeginContentUploadRequest>>,
        ticket_request: Mutex<Option<IssueContentReadTicketRequest>>,
        uploaded_body: Mutex<Vec<u8>>,
        open_request: Mutex<Option<(PrincipalId, ContentId, String)>>,
        opened: Mutex<Option<OpenedVerifiedContent>>,
    }

    impl FakeContent {
        fn new() -> Self {
            let content = active_content();
            Self {
                policy: ContentAccessPolicy::new(
                    content.id(),
                    MatrixRoomId::new(ROOM_ID).expect("测试房间标识有效"),
                    ContentAccessMode::RoomMember,
                    time(1_700_000_000_000),
                ),
                content,
                begin_request: Mutex::new(None),
                ticket_request: Mutex::new(None),
                uploaded_body: Mutex::new(Vec::new()),
                open_request: Mutex::new(None),
                opened: Mutex::new(Some(OpenedVerifiedContent {
                    content_id: content_id(),
                    digest: Sha256Digest::from_bytes([7; 32]),
                    byte_length: ContentByteLength::new(7).expect("测试长度有效"),
                    media_type: ContentMediaType::new("text/plain").expect("测试媒体类型有效"),
                    body: Box::pin(stream::iter(vec![Ok(b"payload".to_vec())])),
                })),
            }
        }
    }

    impl ContentUseCases for FakeContent {
        fn begin_upload(
            &self,
            request: BeginContentUploadRequest,
        ) -> PortFuture<'_, BeginContentUploadResult<BeginContentUploadOutcome>> {
            *self.begin_request.lock().expect("上传请求记录锁可用") = Some(request);
            let content = uploading_content();
            let policy = self.policy.clone();
            Box::pin(async move {
                Ok(BeginContentUploadOutcome::Created {
                    content,
                    access_policy: policy,
                })
            })
        }

        fn complete_upload(
            &self,
            mut request: CompleteContentUploadRequest,
        ) -> PortFuture<'_, CompleteContentUploadResult<CompleteContentUploadOutcome>> {
            Box::pin(async move {
                let mut bytes = Vec::new();
                while let Some(chunk) = request.body.next().await {
                    bytes.extend(chunk.expect("测试上传流必须可读"));
                }
                *self.uploaded_body.lock().expect("上传正文记录锁可用") = bytes;
                Ok(CompleteContentUploadOutcome::Activated(
                    self.content.clone(),
                ))
            })
        }

        fn bind_event(
            &self,
            _request: BindContentEventRequest,
        ) -> PortFuture<'_, BindContentEventResult<BindContentEventOutcome>> {
            Box::pin(async { unreachable!("当前测试不会绑定事件") })
        }

        fn redact(
            &self,
            _request: RedactContentRequest,
        ) -> PortFuture<'_, RedactContentResult<RedactContentOutcome>> {
            let mut content = self.content.clone();
            content.redact().expect("测试内容可撤回");
            Box::pin(async move { Ok(RedactContentOutcome::Redacted(content)) })
        }

        fn issue_read_ticket(
            &self,
            request: IssueContentReadTicketRequest,
        ) -> PortFuture<'_, IssueContentReadTicketResult<IssuedContentReadTicket>> {
            *self.ticket_request.lock().expect("读取票据请求记录锁可用") = Some(request);
            Box::pin(async {
                Ok(IssuedContentReadTicket {
                    ticket: ContentReadTicket::new("test-read-ticket").expect("测试票据有效"),
                    expires_at: time(1_700_000_060_000),
                })
            })
        }

        fn open(
            &self,
            request: OpenContentRequest,
        ) -> PortFuture<'_, OpenContentResult<OpenedVerifiedContent>> {
            *self.open_request.lock().expect("读取请求记录锁可用") = Some((
                request.principal_id,
                request.content_id,
                request.ticket.expose().to_owned(),
            ));
            let opened = self
                .opened
                .lock()
                .expect("读取响应记录锁可用")
                .take()
                .expect("测试读取响应只使用一次");
            Box::pin(async move { Ok(opened) })
        }
    }

    #[derive(Default)]
    struct FakeAuthentication {
        calls: AtomicUsize,
    }

    impl AuthenticationUseCases for FakeAuthentication {
        fn begin_login(
            &self,
            _request: BeginLogin,
        ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>> {
            Box::pin(async { unreachable!("内容路由不会开始登录") })
        }

        fn complete_login<'a>(
            &'a self,
            _request: CompleteLogin<'a>,
        ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
            Box::pin(async { unreachable!("内容路由不会完成登录") })
        }

        fn authenticate<'a>(
            &'a self,
            session_secret: &'a SecretValue,
            requirement: AuthenticationRequirement,
        ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>> {
            assert_eq!(session_secret.expose(), "session-secret");
            assert_eq!(requirement, AuthenticationRequirement::ActiveSession);
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(authenticated_principal()) })
        }

        fn logout<'a>(
            &'a self,
            _session_secret: &'a SecretValue,
        ) -> PortFuture<'a, AuthenticationResult<()>> {
            Box::pin(async { unreachable!("内容路由不会退出登录") })
        }

        fn suspend_principal(
            &self,
            _principal_id: PrincipalId,
        ) -> PortFuture<'_, AuthenticationResult<()>> {
            Box::pin(async { unreachable!("内容路由不会暂停主体") })
        }
    }

    #[derive(Default)]
    struct FakeDevices {
        calls: AtomicUsize,
        expected: Mutex<Option<ExpectedProof>>,
    }

    struct ExpectedProof {
        method: &'static str,
        target: String,
        body: String,
    }

    impl FakeDevices {
        fn expect(&self, method: &'static str, target: String, body: String) {
            *self.expected.lock().expect("设备证明预期锁可用") = Some(ExpectedProof {
                method,
                target,
                body,
            });
        }
    }

    impl DeviceAuthorizationUseCases for FakeDevices {
        fn register_device(
            &self,
            _request: RegisterDevice,
        ) -> PortFuture<'_, DeviceAuthorizationResult<DeviceCredentials>> {
            Box::pin(async { unreachable!("内容路由不会注册设备") })
        }

        fn authenticate_device<'a>(
            &'a self,
            request: AuthenticateDeviceRequest<'a>,
        ) -> PortFuture<'a, DeviceAuthorizationResult<AuthenticatedDevice>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(request.access_token.expose(), "device-access-token");
            let expected = self
                .expected
                .lock()
                .expect("设备证明预期锁可用")
                .take()
                .expect("测试必须登记设备证明预期");
            assert_eq!(request.proof.method(), expected.method);
            assert_eq!(request.proof.request_target(), expected.target);
            assert_eq!(
                request.proof.body_digest(),
                &SecureSecretFactory.digest(&expected.body)
            );
            Box::pin(async { Ok(authenticated_device()) })
        }

        fn refresh_device_session<'a>(
            &'a self,
            _request: RefreshDeviceSession<'a>,
        ) -> PortFuture<'a, DeviceAuthorizationResult<DeviceCredentials>> {
            Box::pin(async { unreachable!("内容路由不会刷新设备会话") })
        }

        fn list_devices(
            &self,
            _principal_id: PrincipalId,
        ) -> PortFuture<'_, DeviceAuthorizationResult<Vec<Device>>> {
            Box::pin(async { unreachable!("内容路由不会列出设备") })
        }

        fn revoke_device(
            &self,
            _principal_id: PrincipalId,
            _device_id: DeviceId,
        ) -> PortFuture<'_, DeviceAuthorizationResult<RevokedDevice>> {
            Box::pin(async { unreachable!("内容路由不会撤销设备") })
        }
    }

    #[tokio::test]
    async fn 浏览器上传声明要求同源会话且响应不泄漏对象键() {
        let content = Arc::new(FakeContent::new());
        let authentication = Arc::new(FakeAuthentication::default());
        let body = json!({
            "actorAgentId": AGENT_UUID,
            "matrixRoomId": ROOM_ID,
            "accessMode": "room_member",
            "sha256": DIGEST_HEX,
            "byteLength": 7,
            "mediaType": "text/plain",
            "encryptionMode": "server_side"
        })
        .to_string();

        let response = test_router(
            content.clone(),
            authentication.clone(),
            Arc::new(FakeDevices::default()),
        )
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/content/uploads")
                .header(header::ORIGIN, FRONTEND_ORIGIN)
                .header(header::COOKIE, SESSION_COOKIE)
                .header("idempotency-key", REQUEST_UUID)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .expect("上传声明请求有效"),
        )
        .await
        .expect("上传声明路由可调用");

        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(authentication.calls.load(Ordering::SeqCst), 1);
        let payload = response_json(response).await;
        assert_eq!(payload["contentId"], CONTENT_UUID);
        assert_eq!(payload["created"], true);
        let serialized = payload.to_string();
        assert!(!serialized.contains("storage"));
        assert!(!serialized.contains("downloadUrl"));
        assert!(!serialized.contains("objects/test-private-key"));
        let recorded = content
            .begin_request
            .lock()
            .expect("上传请求记录锁可用")
            .clone()
            .expect("上传用例已调用");
        assert_eq!(recorded.owner_principal_id, principal_id());
        assert_eq!(recorded.actor_agent_id, Some(agent_id()));
        assert_eq!(recorded.request_id.to_string(), REQUEST_UUID);
    }

    #[tokio::test]
    async fn 设备读取票据绑定显式_agent_且签名完整请求体() {
        let content = Arc::new(FakeContent::new());
        let devices = Arc::new(FakeDevices::default());
        let body = json!({ "actorAgentId": AGENT_UUID }).to_string();
        devices.expect(
            "POST",
            format!("/content/{CONTENT_UUID}/read-tickets"),
            body.clone(),
        );

        let response = test_router(
            content.clone(),
            Arc::new(FakeAuthentication::default()),
            devices,
        )
        .oneshot(device_request(
            "POST",
            format!("/content/{CONTENT_UUID}/read-tickets"),
            Body::from(body),
            None,
        ))
        .await
        .expect("读取票据路由可调用");

        assert_eq!(response.status(), StatusCode::OK);
        let payload = response_json(response).await;
        assert_eq!(payload["ticket"], "test-read-ticket");
        assert_eq!(payload["expiresAtUnixMs"], 1_700_000_060_000_i64);
        assert_eq!(
            *content
                .ticket_request
                .lock()
                .expect("读取票据请求记录锁可用"),
            Some(IssueContentReadTicketRequest {
                principal_id: principal_id(),
                actor_agent_id: Some(agent_id()),
                content_id: content_id(),
            })
        );
    }

    #[tokio::test]
    async fn 设备上传正文只签名完整性声明并保持流式传输() {
        let content = Arc::new(FakeContent::new());
        let devices = Arc::new(FakeDevices::default());
        devices.expect(
            "PUT",
            format!("/content/{CONTENT_UUID}/bytes"),
            format!("sha256={DIGEST_HEX}\nbyte-length=7"),
        );

        let response = test_router(
            content.clone(),
            Arc::new(FakeAuthentication::default()),
            devices.clone(),
        )
        .oneshot(device_request(
            "PUT",
            format!("/content/{CONTENT_UUID}/bytes"),
            Body::from("payload"),
            Some((DIGEST_HEX, 7)),
        ))
        .await
        .expect("上传正文路由可调用");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(devices.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            *content.uploaded_body.lock().expect("上传正文记录锁可用"),
            b"payload"
        );
    }

    #[tokio::test]
    async fn 读取响应流携带标准摘要且票据绑定路径内容() {
        let content = Arc::new(FakeContent::new());
        let devices = Arc::new(FakeDevices::default());
        let body = json!({ "ticket": "short-lived-ticket" }).to_string();
        devices.expect(
            "POST",
            format!("/content/{CONTENT_UUID}/open"),
            body.clone(),
        );

        let response = test_router(
            content.clone(),
            Arc::new(FakeAuthentication::default()),
            devices,
        )
        .oneshot(device_request(
            "POST",
            format!("/content/{CONTENT_UUID}/open"),
            Body::from(body),
            None,
        ))
        .await
        .expect("内容读取路由可调用");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("content-digest"),
            Some(&HeaderValue::from_static(
                "sha-256=:BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=:"
            ))
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-store"))
        );
        let bytes = to_bytes(response.into_body(), 64)
            .await
            .expect("正文可读取");
        assert_eq!(bytes, b"payload"[..]);
        assert_eq!(
            *content.open_request.lock().expect("读取请求记录锁可用"),
            Some((
                principal_id(),
                content_id(),
                "short-lived-ticket".to_owned()
            ))
        );
    }

    fn test_router(
        content: Arc<FakeContent>,
        authentication: Arc<FakeAuthentication>,
        devices: Arc<FakeDevices>,
    ) -> axum::Router {
        router(ContentHttpState::new(
            ContentHttpDependencies {
                content,
                authentication,
                devices,
                secrets: Arc::new(SecureSecretFactory),
            },
            &Url::parse(FRONTEND_ORIGIN).expect("前端 Origin 有效"),
            &Url::parse("http://tauri.localhost").expect("桌面 Origin 有效"),
        ))
        .layer(middleware::from_fn(crate::correlation::attach))
    }

    fn device_request(
        method: &str,
        uri: String,
        body: Body,
        integrity: Option<(&str, u64)>,
    ) -> Request<Body> {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, "Bearer device-access-token")
            .header("x-agent-room-device-id", DEVICE_UUID)
            .header("x-agent-room-proof-issued-at", "1700000000000")
            .header("x-agent-room-proof-nonce", "nonce-0123456789abcdef")
            .header(
                "x-agent-room-proof-signature",
                URL_SAFE_NO_PAD.encode([9_u8; 64]),
            );
        if let Some((digest, byte_length)) = integrity {
            request = request
                .header("x-agent-room-content-sha256", digest)
                .header("x-agent-room-content-byte-length", byte_length)
                .header(header::CONTENT_LENGTH, byte_length);
        } else {
            request = request.header(header::CONTENT_TYPE, "application/json");
        }
        request.body(body).expect("设备请求有效")
    }

    async fn response_json(response: Response) -> Value {
        let body = to_bytes(response.into_body(), 64 * 1_024)
            .await
            .expect("响应正文可读取");
        serde_json::from_slice(&body).expect("响应正文是 JSON")
    }

    fn uploading_content() -> ContentObject {
        content(ContentLifecycleState::Uploading, ContentScanState::Pending)
    }

    fn active_content() -> ContentObject {
        content(ContentLifecycleState::Active, ContentScanState::Clean)
    }

    fn content(
        lifecycle_state: ContentLifecycleState,
        scan_state: ContentScanState,
    ) -> ContentObject {
        ContentObject::restore(ContentObjectFields {
            id: content_id(),
            owner_principal_id: principal_id(),
            storage_key: ContentStorageKey::new("objects/test-private-key")
                .expect("测试对象键有效"),
            digest: Sha256Digest::from_bytes([7; 32]),
            byte_length: ContentByteLength::new(7).expect("测试长度有效"),
            media_type: ContentMediaType::new("text/plain").expect("测试媒体类型有效"),
            encryption_mode: ContentEncryptionMode::ServerSide,
            scan_state,
            lifecycle_state,
            expires_at: None,
            created_at: time(1_700_000_000_000),
            deleted_at: None,
        })
        .expect("测试内容聚合有效")
    }

    fn authenticated_principal() -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            principal_id: principal_id(),
            matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
            display_name: "Agent Room User".to_owned(),
            locale: "zh-CN".to_owned(),
            authenticated_at: time(1_700_000_000_000),
            expires_at: time(1_700_028_800_000),
            recently_authenticated: true,
        }
    }

    fn authenticated_device() -> AuthenticatedDevice {
        AuthenticatedDevice {
            account: PrincipalAccount {
                principal: Principal::new(principal_id()),
                matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
                display_name: "Agent Room User".to_owned(),
                avatar_content_id: None,
                locale: "zh-CN".to_owned(),
            },
            device_id: DeviceId::from_uuid(uuid(DEVICE_UUID)),
            access_token_expires_at: time(1_700_000_900_000),
        }
    }

    fn principal_id() -> PrincipalId {
        PrincipalId::from_uuid(uuid(PRINCIPAL_UUID))
    }

    fn content_id() -> ContentId {
        ContentId::from_uuid(uuid(CONTENT_UUID))
    }

    fn agent_id() -> AgentId {
        AgentId::from_uuid(uuid(AGENT_UUID))
    }

    fn uuid(value: &str) -> Uuid {
        Uuid::parse_str(value).expect("测试 UUID 有效")
    }

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("测试时间有效")
    }
}
