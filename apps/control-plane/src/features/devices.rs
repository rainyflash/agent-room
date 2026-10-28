use std::sync::Arc;

use agent_room_application::{
    agent_room_departures::AgentRoomDepartureUseCases,
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    devices::{
        AuthenticateDeviceRequest, AuthenticatedDevice, DeviceAuthorizationResult,
        DeviceAuthorizationUseCases, DeviceCredentials, DeviceMatrixCleanup, DeviceRequestProof,
        DeviceRequestProofPayload, RefreshDeviceSession, RegisterDevice, RevokedDevice,
        VerifiedDeviceAuthorization,
    },
    ports::{
        DeviceSignature, OidcDeviceAssertionVerifier, OidcFailure, OidcFailureKind,
        ProfileImportConsent, SecretFactory, SecretValue,
    },
};
use agent_room_domain::{
    devices::{Device, DevicePlatform, DevicePublicSigningKey},
    ids::{DeviceId, DeviceRefreshAttemptId},
    time::UtcMillis,
};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Extension, Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use axum_extra::extract::CookieJar;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

use crate::{
    agent_room_departures::depart_after_revocation,
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::{TrustedOrigins, authenticate_session, no_store, origin_matches},
        resource_ids::parse_uuid_v7,
    },
    request_regions,
};

const REGISTER_DEVICE_PATH: &str = "/auth/devices/register";
const REFRESH_DEVICE_PATH: &str = "/auth/devices/refresh";
/// 这台电脑自己请控制面把它从账户里移除（换账户，`specs/sign-out/design.md` 第 4 步）。
const RELEASE_CURRENT_DEVICE_PATH: &str = "/auth/devices/current/release";
const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";
const DEVICE_ID_HEADER: &str = "x-agent-room-device-id";
const PROOF_ISSUED_AT_HEADER: &str = "x-agent-room-proof-issued-at";
const PROOF_NONCE_HEADER: &str = "x-agent-room-proof-nonce";
const PROOF_SIGNATURE_HEADER: &str = "x-agent-room-proof-signature";
pub(super) const MAX_DEVICE_BODY_BYTES: usize = 16 * 1_024;
const MAX_ENCODED_PUBLIC_KEY_LENGTH: usize = 64;
const MAX_ENCODED_SIGNATURE_LENGTH: usize = 128;

#[derive(Clone)]
pub(crate) struct DeviceHttpState {
    devices: Arc<dyn DeviceAuthorizationUseCases>,
    /// 撤销电脑以后让它的 Agent 离开房间：撤销的那次请求里先做一遍。
    departures: Arc<dyn AgentRoomDepartureUseCases>,
    assertion_verifier: Arc<dyn OidcDeviceAssertionVerifier>,
    authentication: Arc<dyn AuthenticationUseCases>,
    secrets: Arc<dyn SecretFactory>,
    trusted_origins: TrustedOrigins,
}

pub(crate) struct DeviceHttpDependencies {
    pub(crate) devices: Arc<dyn DeviceAuthorizationUseCases>,
    pub(crate) departures: Arc<dyn AgentRoomDepartureUseCases>,
    pub(crate) assertion_verifier: Arc<dyn OidcDeviceAssertionVerifier>,
    pub(crate) authentication: Arc<dyn AuthenticationUseCases>,
    pub(crate) secrets: Arc<dyn SecretFactory>,
}

impl DeviceHttpState {
    pub(crate) fn new(
        dependencies: DeviceHttpDependencies,
        frontend_origin: &Url,
        desktop_origins: &crate::config::DesktopOrigins,
    ) -> Self {
        Self {
            devices: dependencies.devices,
            departures: dependencies.departures,
            assertion_verifier: dependencies.assertion_verifier,
            authentication: dependencies.authentication,
            secrets: dependencies.secrets,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origins),
        }
    }
}

pub(crate) fn router(state: DeviceHttpState) -> Router {
    Router::new()
        .route(REGISTER_DEVICE_PATH, post(register_device))
        .route(REFRESH_DEVICE_PATH, post(refresh_device_session))
        .route("/auth/devices", get(list_devices))
        .route("/auth/devices/{device_id}", delete(revoke_device))
        .route(RELEASE_CURRENT_DEVICE_PATH, post(release_current_device))
        .layer(DefaultBodyLimit::max(MAX_DEVICE_BODY_BYTES))
        .with_state(state)
}

/// 设备码和电脑授权两条注册路的请求体一样。电脑授权那条路的账户已经有了，
/// 不再按登录资料改名字和语言，两个 `import_*` 收下不用。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RegisterDeviceBody {
    label: String,
    platform: String,
    public_signing_key: String,
    possession_signature: String,
    #[serde(default)]
    import_display_name: bool,
    #[serde(default)]
    import_locale: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DeviceCredentialsResponse {
    device_id: String,
    principal_id: String,
    matrix_user_id: String,
    display_name: String,
    avatar_content_id: Option<String>,
    locale: String,
    access_token: String,
    access_token_expires_at_unix_ms: i64,
    refresh_token: String,
    refresh_token_expires_at_unix_ms: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceListResponse {
    devices: Vec<DeviceSummaryResponse>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingDeviceRevocationResponse {
    local_revocation: &'static str,
    matrix_cleanup: &'static str,
    pending_agent_instance_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceSummaryResponse {
    device_id: String,
    label: String,
    platform: &'static str,
    trust_state: &'static str,
    matrix_device_id: Option<String>,
    last_seen_at_unix_ms: Option<i64>,
    revoked_at_unix_ms: Option<i64>,
    created_at_unix_ms: i64,
}

async fn register_device(
    State(state): State<DeviceHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    body: Result<Json<RegisterDeviceBody>, JsonRejection>,
) -> Response {
    let Ok(assertion) = bearer_secret(&headers) else {
        return no_store(invalid_bearer(correlation_id).into_response());
    };
    let Ok(Json(body)) = body else {
        return no_store(
            ApiError::invalid_request("device.invalid_registration_body", correlation_id)
                .into_response(),
        );
    };
    let request = match registration_request(&state, assertion, body, correlation_id).await {
        Ok(request) => request,
        Err(error) => return no_store(error.into_response()),
    };
    match state.devices.register_device(request).await {
        Ok(credentials) => {
            no_store(Json(DeviceCredentialsResponse::from(credentials)).into_response())
        }
        Err(failure) => no_store(ApiError::device(failure, correlation_id).into_response()),
    }
}

async fn registration_request(
    state: &DeviceHttpState,
    assertion: SecretValue,
    body: RegisterDeviceBody,
    correlation_id: CorrelationId,
) -> Result<RegisterDevice, ApiError> {
    let verified_assertion = state
        .assertion_verifier
        .verify_assertion(&assertion)
        .await
        .map_err(|failure| oidc_assertion_error(failure, correlation_id))?;
    let profile_import = ProfileImportConsent {
        display_name: body.import_display_name,
        locale: body.import_locale,
    };
    let fields = registration_fields(body)
        .map_err(|code| ApiError::invalid_request(code, correlation_id))?;
    let authorization = VerifiedDeviceAuthorization::new(
        verified_assertion,
        state.secrets.digest(assertion.expose()),
    );

    Ok(RegisterDevice {
        authorization,
        label: fields.label,
        platform: fields.platform,
        public_signing_key: fields.public_signing_key,
        possession_signature: fields.possession_signature,
        profile_import,
    })
}

pub(super) struct DeviceRegistrationFields {
    pub(super) label: String,
    pub(super) platform: DevicePlatform,
    pub(super) public_signing_key: DevicePublicSigningKey,
    pub(super) possession_signature: DeviceSignature,
}

/// 解出两条注册路共用的名字、平台、签名公钥和持有证明；不对时回错误码。
pub(super) fn registration_fields(
    body: RegisterDeviceBody,
) -> Result<DeviceRegistrationFields, &'static str> {
    const PUBLIC_KEY: &str = "device.invalid_public_key";
    const SIGNATURE: &str = "device.invalid_possession_signature";
    let public_signing_key = DevicePublicSigningKey::new(
        decode_bounded(&body.public_signing_key, MAX_ENCODED_PUBLIC_KEY_LENGTH)
            .map_err(|()| PUBLIC_KEY)?,
    )
    .map_err(|_| PUBLIC_KEY)?;
    let possession_signature = DeviceSignature::new(
        decode_bounded(&body.possession_signature, MAX_ENCODED_SIGNATURE_LENGTH)
            .map_err(|()| SIGNATURE)?,
    )
    .map_err(|_| SIGNATURE)?;
    let platform = DevicePlatform::parse_client(body.platform.as_str())
        .map_err(|_| "device.invalid_platform")?;
    Ok(DeviceRegistrationFields {
        label: body.label,
        platform,
        public_signing_key,
        possession_signature,
    })
}

async fn refresh_device_session(
    State(state): State<DeviceHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
) -> Response {
    let Ok(refresh_token) = bearer_secret(&headers) else {
        return no_store(invalid_bearer(correlation_id).into_response());
    };
    let Ok(proof) = request_proof(&state, &headers, REFRESH_DEVICE_PATH) else {
        return no_store(
            ApiError::invalid_request("device.invalid_proof_headers", correlation_id)
                .into_response(),
        );
    };
    let Ok(attempt_id) = refresh_attempt_id(&headers) else {
        return no_store(
            ApiError::invalid_request("device.invalid_idempotency_key", correlation_id)
                .into_response(),
        );
    };
    match state
        .devices
        .refresh_device_session(RefreshDeviceSession {
            refresh_token: &refresh_token,
            proof: &proof,
            attempt_id,
        })
        .await
    {
        Ok(credentials) => {
            no_store(Json(DeviceCredentialsResponse::from(credentials)).into_response())
        }
        Err(failure) => no_store(ApiError::device(failure, correlation_id).into_response()),
    }
}

async fn list_devices(
    State(state): State<DeviceHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    jar: CookieJar,
) -> Response {
    let principal = match authenticate_session(
        state.authentication.as_ref(),
        &jar,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
    {
        Ok(principal) => principal,
        Err(response) => return response,
    };
    match state.devices.list_devices(principal.principal_id).await {
        Ok(devices) => no_store(
            Json(DeviceListResponse {
                devices: devices.iter().map(DeviceSummaryResponse::from).collect(),
            })
            .into_response(),
        ),
        Err(failure) => no_store(ApiError::device(failure, correlation_id).into_response()),
    }
}

async fn revoke_device(
    State(state): State<DeviceHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(device_id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    if !origin_matches(&headers, &state.trusted_origins) {
        return no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "device.invalid_revoke_origin",
                ErrorCategory::Authorization,
                "设备撤销请求来源无效。",
                correlation_id,
            )
            .into_response(),
        );
    }
    let Ok(device_id) = Uuid::parse_str(&device_id) else {
        return no_store(
            ApiError::invalid_request("device.invalid_device_id", correlation_id).into_response(),
        );
    };
    let principal = match authenticate_session(
        state.authentication.as_ref(),
        &jar,
        AuthenticationRequirement::RecentAuthentication,
        correlation_id,
    )
    .await
    {
        Ok(principal) => principal,
        Err(response) => return response,
    };
    let device_id = DeviceId::from_uuid(device_id);
    let revoked = state
        .devices
        .revoke_device(principal.principal_id, device_id)
        .await;
    if revoked.is_ok() {
        depart_after_revocation(state.departures.as_ref(), device_id).await;
    }
    revocation_response(revoked, correlation_id)
}

/// 拿着这台电脑凭据签名的就是这台电脑本身，所以不要最近登录过：换账户时这台电脑先把自己移出原来的
/// 账户，再交给新账户。撤销和“撤销电脑”是同一套，它的 Agent 连接一起撤掉。
async fn release_current_device(
    State(state): State<DeviceHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Ok(body) = std::str::from_utf8(&body) else {
        return no_store(
            ApiError::invalid_request("device.invalid_release_body", correlation_id)
                .into_response(),
        );
    };
    let device = match authenticate_signed_device_request(
        state.devices.as_ref(),
        state.secrets.as_ref(),
        &headers,
        "POST",
        RELEASE_CURRENT_DEVICE_PATH,
        body,
        correlation_id,
    )
    .await
    {
        Ok(device) => device,
        Err(response) => return response,
    };
    let revoked = state
        .devices
        .revoke_device(device.account.principal.id(), device.device_id)
        .await;
    if revoked.is_ok() {
        depart_after_revocation(state.departures.as_ref(), device.device_id).await;
    }
    revocation_response(revoked, correlation_id)
}

fn revocation_response(
    revoked: DeviceAuthorizationResult<RevokedDevice>,
    correlation_id: CorrelationId,
) -> Response {
    match revoked {
        Ok(revoked) => match revoked.matrix_cleanup {
            DeviceMatrixCleanup::Complete => no_store(StatusCode::NO_CONTENT.into_response()),
            DeviceMatrixCleanup::Pending {
                agent_instance_count,
            } => no_store(
                (
                    StatusCode::ACCEPTED,
                    Json(PendingDeviceRevocationResponse {
                        local_revocation: "complete",
                        matrix_cleanup: "pending",
                        pending_agent_instance_count: agent_instance_count,
                    }),
                )
                    .into_response(),
            ),
        },
        Err(failure) => no_store(ApiError::device(failure, correlation_id).into_response()),
    }
}

fn request_proof(
    state: &DeviceHttpState,
    headers: &HeaderMap,
    request_target: &str,
) -> Result<DeviceRequestProof, ()> {
    request_proof_for(state.secrets.as_ref(), headers, "POST", request_target, "")
}

/// 新版 Bridge 用幂等键标识一次刷新尝试；旧版不带，仍按不可重试的轮换处理。
fn refresh_attempt_id(headers: &HeaderMap) -> Result<Option<DeviceRefreshAttemptId>, ()> {
    let Some(value) = headers.get(IDEMPOTENCY_KEY_HEADER) else {
        return Ok(None);
    };
    let value = value.to_str().map_err(|_| ())?;
    parse_uuid_v7(value).map(|id| Some(DeviceRefreshAttemptId::from_uuid(id)))
}

pub(crate) async fn authenticate_signed_device_request(
    devices: &dyn DeviceAuthorizationUseCases,
    secrets: &dyn SecretFactory,
    headers: &HeaderMap,
    method: &str,
    request_target: &str,
    body: &str,
    correlation_id: CorrelationId,
) -> Result<AuthenticatedDevice, Response> {
    let access_token = bearer_secret(headers)
        .map_err(|()| no_store(invalid_bearer(correlation_id).into_response()))?;
    let proof =
        request_proof_for(secrets, headers, method, request_target, body).map_err(|()| {
            no_store(
                ApiError::invalid_request("device.invalid_proof_headers", correlation_id)
                    .into_response(),
            )
        })?;
    let device = devices
        .authenticate_device(AuthenticateDeviceRequest {
            access_token: &access_token,
            proof: &proof,
        })
        .await
        .map_err(|failure| no_store(ApiError::device(failure, correlation_id).into_response()))?;
    request_regions::note_local_agent(device.account.principal.id());
    Ok(device)
}

fn request_proof_for(
    secrets: &dyn SecretFactory,
    headers: &HeaderMap,
    method: &str,
    request_target: &str,
    body: &str,
) -> Result<DeviceRequestProof, ()> {
    let device_id = required_header(headers, DEVICE_ID_HEADER)
        .and_then(|value| Uuid::parse_str(value).map_err(|_| ()))
        .map(DeviceId::from_uuid)?;
    let issued_at = required_header(headers, PROOF_ISSUED_AT_HEADER)
        .and_then(|value| value.parse::<i64>().map_err(|_| ()))
        .and_then(|value| UtcMillis::new(value).map_err(|_| ()))?;
    let nonce = required_header(headers, PROOF_NONCE_HEADER)
        .and_then(|value| SecretValue::new(value).map_err(|_| ()))?;
    let signature = required_header(headers, PROOF_SIGNATURE_HEADER)
        .and_then(|value| decode_bounded(value, MAX_ENCODED_SIGNATURE_LENGTH))
        .and_then(|value| DeviceSignature::new(value).map_err(|_| ()))?;
    let payload = DeviceRequestProofPayload::new(
        device_id,
        issued_at,
        nonce,
        method.to_owned(),
        request_target.to_owned(),
        secrets.digest(body),
    )
    .map_err(|_| ())?;
    Ok(DeviceRequestProof::new(payload, signature))
}

fn required_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, ()> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .ok_or(())
}

pub(crate) fn bearer_secret(headers: &HeaderMap) -> Result<SecretValue, ()> {
    let value = required_header(headers, header::AUTHORIZATION.as_str())?;
    let (scheme, credential) = value.split_once(' ').ok_or(())?;
    if !scheme.eq_ignore_ascii_case("Bearer")
        || credential.is_empty()
        || credential.chars().any(char::is_whitespace)
    {
        return Err(());
    }
    SecretValue::new(credential).map_err(|_| ())
}

fn decode_bounded(value: &str, maximum_encoded_length: usize) -> Result<Vec<u8>, ()> {
    if value.is_empty() || value.len() > maximum_encoded_length {
        return Err(());
    }
    URL_SAFE_NO_PAD.decode(value).map_err(|_| ())
}

pub(super) fn invalid_bearer(correlation_id: CorrelationId) -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        "device.invalid_bearer",
        ErrorCategory::Authentication,
        "设备认证凭据缺失或无效。",
        correlation_id,
    )
}

fn oidc_assertion_error(failure: OidcFailure, correlation_id: CorrelationId) -> ApiError {
    let (status, code, category, message) = match failure.kind() {
        OidcFailureKind::DependencyUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "device.identity_provider_unavailable",
            ErrorCategory::DependencyUnavailable,
            "身份提供方暂时不可用。",
        ),
        OidcFailureKind::ProviderRejected
        | OidcFailureKind::AuthorizationExpired
        | OidcFailureKind::InvalidIdentityToken => (
            StatusCode::UNAUTHORIZED,
            "device.invalid_oidc_assertion",
            ErrorCategory::Authentication,
            "设备授权断言无效或已过期。",
        ),
        OidcFailureKind::PromptUnavailable | OidcFailureKind::InvalidConfiguration => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "device.identity_configuration_invalid",
            ErrorCategory::Transient,
            "设备身份验证配置无效。",
        ),
    };
    tracing::warn!(
        correlation.id = %correlation_id.as_uuid(),
        failure = ?failure.kind(),
        "OIDC 设备授权断言验证失败"
    );
    ApiError::new(status, code, category, message, correlation_id)
}

impl From<DeviceCredentials> for DeviceCredentialsResponse {
    fn from(value: DeviceCredentials) -> Self {
        Self {
            device_id: value.device.device_id.to_string(),
            principal_id: value.device.account.principal.id().to_string(),
            matrix_user_id: value.device.account.matrix_user_id,
            display_name: value.device.account.display_name,
            avatar_content_id: value
                .device
                .account
                .avatar_content_id
                .map(|content_id| content_id.to_string()),
            locale: value.device.account.locale,
            access_token: value.access_token.expose().to_owned(),
            access_token_expires_at_unix_ms: value.device.access_token_expires_at.value(),
            refresh_token: value.refresh_token.expose().to_owned(),
            refresh_token_expires_at_unix_ms: value.refresh_token_expires_at.value(),
        }
    }
}

impl From<&Device> for DeviceSummaryResponse {
    fn from(value: &Device) -> Self {
        Self {
            device_id: value.id().to_string(),
            label: value.label().to_owned(),
            platform: value.platform().as_str(),
            trust_state: value.trust_state().as_str(),
            matrix_device_id: value.matrix_device_id().map(str::to_owned),
            last_seen_at_unix_ms: value.last_seen_at().map(UtcMillis::value),
            revoked_at_unix_ms: value.revoked_at().map(UtcMillis::value),
            created_at_unix_ms: value.created_at().value(),
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
        agent_room_departures::{AgentRoomDepartureUseCases, DepartureRound},
        authentication::{
            AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
            AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
        },
        devices::{
            AuthenticateDeviceRequest, AuthenticatedDevice, DeviceAuthorizationResult,
            DeviceAuthorizationUseCases, DeviceCredentials, DeviceMatrixCleanup,
            RefreshDeviceSession, RegisterDevice, RevokedDevice,
        },
        persistence::RepositoryError,
        ports::{
            OidcDeviceAssertionVerifier, OidcResult, PortFuture, PrincipalAccount, SecretDigest,
            SecretFactory, SecretValue, VerifiedOidcDeviceAssertion, VerifiedOidcIdentity,
        },
    };
    use agent_room_domain::{
        devices::{Device, DevicePlatform, DevicePublicSigningKey},
        identity::Principal,
        ids::{DeviceId, DeviceRefreshAttemptId, PrincipalId},
        time::UtcMillis,
    };
    use agent_room_identity_adapter::SecureSecretFactory;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
        middleware,
    };
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use url::Url;
    use uuid::Uuid;

    use super::{
        DEVICE_ID_HEADER, DeviceHttpDependencies, DeviceHttpState, PROOF_ISSUED_AT_HEADER,
        PROOF_NONCE_HEADER, PROOF_SIGNATURE_HEADER, router,
    };

    const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
    const DEVICE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e43";

    #[derive(Default)]
    struct FakeDevices {
        registration: Mutex<Option<RegisterDevice>>,
        refreshes: AtomicUsize,
        refresh_attempts: Mutex<Vec<Option<DeviceRefreshAttemptId>>>,
        revocation: Mutex<Option<(PrincipalId, DeviceId)>>,
        pending_matrix_cleanup: AtomicUsize,
        /// 设备签名的请求：访问令牌、方法、目标和正文摘要。
        signed_requests: Mutex<Vec<(String, String, String, SecretDigest)>>,
        /// 撤销以后接着让哪台电脑的 Agent 离开房间。
        departed: Mutex<Vec<DeviceId>>,
    }

    impl AgentRoomDepartureUseCases for FakeDevices {
        fn depart_for_device(
            &self,
            device_id: DeviceId,
        ) -> PortFuture<'_, Result<DepartureRound, RepositoryError>> {
            self.departed
                .lock()
                .expect("离开记录锁可用")
                .push(device_id);
            Box::pin(async { Ok(DepartureRound::default()) })
        }

        fn depart_due(&self) -> PortFuture<'_, Result<DepartureRound, RepositoryError>> {
            Box::pin(async { unreachable!("撤销的请求里只做这台电脑的") })
        }
    }

    impl DeviceAuthorizationUseCases for FakeDevices {
        fn register_device(
            &self,
            request: RegisterDevice,
        ) -> PortFuture<'_, DeviceAuthorizationResult<DeviceCredentials>> {
            *self.registration.lock().expect("注册记录锁可用") = Some(request);
            Box::pin(async { Ok(credentials()) })
        }

        fn authenticate_device<'a>(
            &'a self,
            request: AuthenticateDeviceRequest<'a>,
        ) -> PortFuture<'a, DeviceAuthorizationResult<AuthenticatedDevice>> {
            self.signed_requests
                .lock()
                .expect("签名请求记录锁可用")
                .push((
                    request.access_token.expose().to_owned(),
                    request.proof.method().to_owned(),
                    request.proof.request_target().to_owned(),
                    *request.proof.body_digest(),
                ));
            Box::pin(async { Ok(credentials().device) })
        }

        fn refresh_device_session<'a>(
            &'a self,
            request: RefreshDeviceSession<'a>,
        ) -> PortFuture<'a, DeviceAuthorizationResult<DeviceCredentials>> {
            assert_eq!(request.refresh_token.expose(), "refresh-token");
            assert_eq!(request.proof.device_id(), device_id());
            assert_eq!(request.proof.method(), "POST");
            assert_eq!(request.proof.request_target(), "/auth/devices/refresh");
            assert_eq!(request.proof.body_digest(), &SecureSecretFactory.digest(""));
            self.refreshes.fetch_add(1, Ordering::SeqCst);
            self.refresh_attempts
                .lock()
                .expect("刷新尝试记录锁可用")
                .push(request.attempt_id);
            Box::pin(async { Ok(credentials()) })
        }

        fn list_devices(
            &self,
            principal_id: PrincipalId,
        ) -> PortFuture<'_, DeviceAuthorizationResult<Vec<Device>>> {
            assert_eq!(principal_id, principal_id_value());
            Box::pin(async { Ok(vec![device()]) })
        }

        fn revoke_device(
            &self,
            principal_id: PrincipalId,
            device_id: DeviceId,
        ) -> PortFuture<'_, DeviceAuthorizationResult<RevokedDevice>> {
            *self.revocation.lock().expect("撤销记录锁可用") = Some((principal_id, device_id));
            let pending = self.pending_matrix_cleanup.load(Ordering::SeqCst);
            Box::pin(async move {
                Ok(RevokedDevice {
                    matrix_cleanup: if pending == 0 {
                        DeviceMatrixCleanup::Complete
                    } else {
                        DeviceMatrixCleanup::Pending {
                            agent_instance_count: pending,
                        }
                    },
                })
            })
        }
    }

    #[derive(Default)]
    struct FakeAuthentication {
        requirements: Mutex<Vec<AuthenticationRequirement>>,
    }

    impl AuthenticationUseCases for FakeAuthentication {
        fn begin_login(
            &self,
            _request: BeginLogin,
        ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>> {
            Box::pin(async { unreachable!("设备路由不会开始浏览器登录") })
        }

        fn complete_login<'a>(
            &'a self,
            _request: CompleteLogin<'a>,
        ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
            Box::pin(async { unreachable!("设备路由不会完成浏览器登录") })
        }

        fn authenticate<'a>(
            &'a self,
            session_secret: &'a SecretValue,
            requirement: AuthenticationRequirement,
        ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>> {
            assert_eq!(session_secret.expose(), "session-secret");
            self.requirements
                .lock()
                .expect("认证记录锁可用")
                .push(requirement);
            Box::pin(async { Ok(authenticated_principal()) })
        }

        fn logout<'a>(
            &'a self,
            _session_secret: &'a SecretValue,
        ) -> PortFuture<'a, AuthenticationResult<()>> {
            Box::pin(async { Ok(()) })
        }

        fn suspend_principal(
            &self,
            _principal_id: PrincipalId,
        ) -> PortFuture<'_, AuthenticationResult<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    struct FakeAssertionVerifier;

    impl OidcDeviceAssertionVerifier for FakeAssertionVerifier {
        fn verify_assertion<'a>(
            &'a self,
            assertion: &'a SecretValue,
        ) -> PortFuture<'a, OidcResult<VerifiedOidcDeviceAssertion>> {
            assert_eq!(assertion.expose(), "oidc-assertion");
            Box::pin(async {
                let identity = VerifiedOidcIdentity::new(
                    "https://identity.example",
                    "stable-subject",
                    Some("Agent Room User".to_owned()),
                    Some("zh-CN".to_owned()),
                    Some(time(1_699_913_600_000)),
                )
                .expect("测试 OIDC 身份有效");
                Ok(VerifiedOidcDeviceAssertion::new(
                    identity,
                    time(1_700_000_000_000),
                ))
            })
        }
    }

    fn test_router(
        devices: Arc<FakeDevices>,
        authentication: Arc<FakeAuthentication>,
    ) -> axum::Router {
        let state = DeviceHttpState::new(
            DeviceHttpDependencies {
                devices: devices.clone(),
                departures: devices,
                assertion_verifier: Arc::new(FakeAssertionVerifier),
                authentication,
                secrets: Arc::new(SecureSecretFactory),
            },
            &Url::parse("https://app.agent-room.test").expect("前端 Origin 有效"),
            &crate::config::DesktopOrigins::for_tests(),
        );
        router(state).layer(middleware::from_fn(crate::correlation::attach))
    }

    fn credentials() -> DeviceCredentials {
        DeviceCredentials {
            device: AuthenticatedDevice {
                account: account(),
                device_id: device_id(),
                access_token_expires_at: time(1_700_000_900_000),
            },
            access_token: SecretValue::new("access-token").expect("访问令牌有效"),
            refresh_token: SecretValue::new("refresh-token-next").expect("刷新令牌有效"),
            refresh_token_expires_at: time(1_702_592_000_000),
        }
    }

    fn account() -> PrincipalAccount {
        PrincipalAccount {
            principal: Principal::new(principal_id_value()),
            matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
            display_name: "Agent Room User".to_owned(),
            avatar_content_id: None,
            locale: "zh-CN".to_owned(),
        }
    }

    fn authenticated_principal() -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            principal_id: principal_id_value(),
            matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
            display_name: "Agent Room User".to_owned(),
            locale: "zh-CN".to_owned(),
            authenticated_at: time(1_700_000_000_000),
            expires_at: time(1_700_028_800_000),
            recently_authenticated: true,
        }
    }

    fn device() -> Device {
        let mut device = Device::register(
            device_id(),
            principal_id_value(),
            "Windows 工作站".to_owned(),
            DevicePlatform::Windows,
            DevicePublicSigningKey::new(vec![7; 32]).expect("测试公钥有效"),
            time(1_700_000_000_000),
        )
        .expect("测试设备有效");
        device.verify().expect("测试设备可验证");
        device
    }

    fn principal_id_value() -> PrincipalId {
        PrincipalId::from_uuid(Uuid::parse_str(PRINCIPAL_UUID).expect("主体 UUID 有效"))
    }

    fn device_id() -> DeviceId {
        DeviceId::from_uuid(Uuid::parse_str(DEVICE_UUID).expect("设备 UUID 有效"))
    }

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("测试时间有效")
    }

    async fn body_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), 64 * 1_024)
            .await
            .expect("响应正文可读取");
        serde_json::from_slice(&body).expect("响应正文是 JSON")
    }

    #[tokio::test]
    async fn 设备注册从_oidc_bearer_建立签名绑定并只返回一次凭据() {
        let devices = Arc::new(FakeDevices::default());
        let response = test_router(devices.clone(), Arc::new(FakeAuthentication::default()))
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/devices/register")
                    .header(header::AUTHORIZATION, "Bearer oidc-assertion")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "label": "Windows 工作站",
                            "platform": "windows",
                            "publicSigningKey": URL_SAFE_NO_PAD.encode([7_u8; 32]),
                            "possessionSignature": URL_SAFE_NO_PAD.encode([9_u8; 64]),
                            "importDisplayName": true,
                            "importLocale": true
                        })
                        .to_string(),
                    ))
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&header::HeaderValue::from_static("no-store"))
        );
        let body = body_json(response).await;
        assert_eq!(body["deviceId"], DEVICE_UUID);
        assert_eq!(body["accessToken"], "access-token");
        assert_eq!(body["refreshToken"], "refresh-token-next");
        let registration = devices
            .registration
            .lock()
            .expect("注册记录锁可用")
            .clone()
            .expect("注册用例已调用");
        assert_eq!(registration.label, "Windows 工作站");
        assert_eq!(registration.platform, DevicePlatform::Windows);
        assert!(registration.profile_import.display_name);
        assert!(registration.profile_import.locale);
        assert_eq!(
            registration.authorization.identity().subject(),
            "stable-subject"
        );
        assert_eq!(
            registration.authorization.issued_at(),
            time(1_700_000_000_000)
        );
        assert_eq!(
            registration.authorization.identity().authenticated_at(),
            Some(time(1_699_913_600_000)),
        );
    }

    #[tokio::test]
    async fn 刷新令牌只接受服务端规范化的设备证明头() {
        let devices = Arc::new(FakeDevices::default());
        let response = test_router(devices.clone(), Arc::new(FakeAuthentication::default()))
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/devices/refresh")
                    .header(header::AUTHORIZATION, "Bearer refresh-token")
                    .header(DEVICE_ID_HEADER, DEVICE_UUID)
                    .header(PROOF_ISSUED_AT_HEADER, "1700000000000")
                    .header(PROOF_NONCE_HEADER, "0123456789abcdef")
                    .header(PROOF_SIGNATURE_HEADER, URL_SAFE_NO_PAD.encode([5_u8; 64]))
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(devices.refreshes.load(Ordering::SeqCst), 1);

        let invalid = test_router(devices.clone(), Arc::new(FakeAuthentication::default()))
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/devices/refresh")
                    .header(header::AUTHORIZATION, "Bearer refresh-token")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
        assert_eq!(devices.refreshes.load(Ordering::SeqCst), 1);
        assert_eq!(
            devices
                .refresh_attempts
                .lock()
                .expect("刷新尝试记录锁可用")
                .as_slice(),
            [None],
            "旧版 Bridge 不带幂等键时仍走原有轮换"
        );
    }

    #[tokio::test]
    async fn 刷新幂等键作为尝试号传给用例且拒绝非_v7_值() {
        const ATTEMPT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
        let devices = Arc::new(FakeDevices::default());
        let refresh = |idempotency_key: &'static str| {
            Request::builder()
                .method("POST")
                .uri("/auth/devices/refresh")
                .header(header::AUTHORIZATION, "Bearer refresh-token")
                .header(DEVICE_ID_HEADER, DEVICE_UUID)
                .header(PROOF_ISSUED_AT_HEADER, "1700000000000")
                .header(PROOF_NONCE_HEADER, "0123456789abcdef")
                .header(PROOF_SIGNATURE_HEADER, URL_SAFE_NO_PAD.encode([5_u8; 64]))
                .header("idempotency-key", idempotency_key)
                .body(Body::empty())
                .expect("请求有效")
        };

        let accepted = test_router(devices.clone(), Arc::new(FakeAuthentication::default()))
            .oneshot(refresh(ATTEMPT_UUID))
            .await
            .expect("路由执行成功");
        assert_eq!(accepted.status(), StatusCode::OK);

        for invalid_key in ["not-a-uuid", "550e8400-e29b-41d4-a716-446655440000"] {
            let rejected = test_router(devices.clone(), Arc::new(FakeAuthentication::default()))
                .oneshot(refresh(invalid_key))
                .await
                .expect("路由执行成功");
            assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
            assert_eq!(
                body_json(rejected).await["code"],
                "device.invalid_idempotency_key"
            );
        }
        assert_eq!(
            devices
                .refresh_attempts
                .lock()
                .expect("刷新尝试记录锁可用")
                .as_slice(),
            [Some(DeviceRefreshAttemptId::from_uuid(
                Uuid::parse_str(ATTEMPT_UUID).expect("尝试号有效")
            ))],
            "非法幂等键不能触发轮换"
        );
    }

    #[tokio::test]
    async fn 设备列表使用活跃会话而撤销额外要求同源和近期认证() {
        let devices = Arc::new(FakeDevices::default());
        let authentication = Arc::new(FakeAuthentication::default());
        let app = test_router(devices.clone(), authentication.clone());
        let list = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/auth/devices")
                    .header(header::COOKIE, "__Host-agent-room-session=session-secret")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");
        assert_eq!(list.status(), StatusCode::OK);
        assert_eq!(body_json(list).await["devices"][0]["deviceId"], DEVICE_UUID);

        let wrong_origin = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/auth/devices/{DEVICE_UUID}"))
                    .header(header::ORIGIN, "https://evil.example")
                    .header(header::COOKIE, "__Host-agent-room-session=session-secret")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");
        assert_eq!(wrong_origin.status(), StatusCode::FORBIDDEN);
        assert!(devices.revocation.lock().expect("撤销记录锁可用").is_none());
        assert!(devices.departed.lock().expect("离开记录锁可用").is_empty());

        let revoked = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/auth/devices/{DEVICE_UUID}"))
                    .header(header::ORIGIN, "https://app.agent-room.test")
                    .header(header::COOKIE, "__Host-agent-room-session=session-secret")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");
        assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            authentication
                .requirements
                .lock()
                .expect("认证记录锁可用")
                .as_slice(),
            [
                AuthenticationRequirement::ActiveSession,
                AuthenticationRequirement::RecentAuthentication
            ]
        );
        assert_eq!(
            *devices.revocation.lock().expect("撤销记录锁可用"),
            Some((principal_id_value(), device_id()))
        );
        assert_eq!(
            devices.departed.lock().expect("离开记录锁可用").as_slice(),
            [device_id()],
            "撤销以后接着让这台电脑的 Agent 离开房间"
        );
    }

    #[tokio::test]
    async fn 设备本地撤销完成但_matrix_清理待重试时返回_202() {
        let devices = Arc::new(FakeDevices::default());
        devices.pending_matrix_cleanup.store(2, Ordering::SeqCst);
        let response = test_router(devices, Arc::new(FakeAuthentication::default()))
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/auth/devices/{DEVICE_UUID}"))
                    .header(header::ORIGIN, "https://app.agent-room.test")
                    .header(header::COOKIE, "__Host-agent-room-session=session-secret")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");

        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let body = body_json(response).await;
        assert_eq!(body["localRevocation"], "complete");
        assert_eq!(body["matrixCleanup"], "pending");
        assert_eq!(body["pendingAgentInstanceCount"], 2);
    }

    fn release_request(body: &'static str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/auth/devices/current/release")
            .header(header::AUTHORIZATION, "Bearer access-token")
            .header(DEVICE_ID_HEADER, DEVICE_UUID)
            .header(PROOF_ISSUED_AT_HEADER, "1700000000000")
            .header(PROOF_NONCE_HEADER, "0123456789abcdef")
            .header(PROOF_SIGNATURE_HEADER, URL_SAFE_NO_PAD.encode([5_u8; 64]))
            .body(Body::from(body))
            .expect("请求有效")
    }

    #[tokio::test]
    async fn 这台电脑凭自己的签名把自己移出账户_不要网页登录() {
        let devices = Arc::new(FakeDevices::default());
        let authentication = Arc::new(FakeAuthentication::default());
        let response = test_router(devices.clone(), authentication.clone())
            .oneshot(release_request(""))
            .await
            .expect("路由执行成功");

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&header::HeaderValue::from_static("no-store"))
        );
        assert_eq!(
            *devices.revocation.lock().expect("撤销记录锁可用"),
            Some((principal_id_value(), device_id())),
            "撤的是签名的这台电脑，主体取自它的凭据"
        );
        assert_eq!(
            devices.departed.lock().expect("离开记录锁可用").as_slice(),
            [device_id()],
            "移出以后接着让这台电脑的 Agent 离开房间"
        );
        assert_eq!(
            devices
                .signed_requests
                .lock()
                .expect("签名请求记录锁可用")
                .as_slice(),
            [(
                "access-token".to_owned(),
                "POST".to_owned(),
                "/auth/devices/current/release".to_owned(),
                SecureSecretFactory.digest(""),
            )]
        );
        assert!(
            authentication
                .requirements
                .lock()
                .expect("认证记录锁可用")
                .is_empty(),
            "不看网页登录，也不要最近登录过"
        );
    }

    #[tokio::test]
    async fn 移出这台电脑时_matrix_清理待重试也返回_202() {
        let devices = Arc::new(FakeDevices::default());
        devices.pending_matrix_cleanup.store(3, Ordering::SeqCst);
        let response = test_router(devices, Arc::new(FakeAuthentication::default()))
            .oneshot(release_request(""))
            .await
            .expect("路由执行成功");

        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let body = body_json(response).await;
        assert_eq!(body["localRevocation"], "complete");
        assert_eq!(body["matrixCleanup"], "pending");
        assert_eq!(body["pendingAgentInstanceCount"], 3);
    }

    #[tokio::test]
    async fn 移出这台电脑没有设备签名就什么都不动() {
        let devices = Arc::new(FakeDevices::default());
        let no_proof = test_router(devices.clone(), Arc::new(FakeAuthentication::default()))
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/devices/current/release")
                    .header(header::AUTHORIZATION, "Bearer access-token")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");
        assert_eq!(no_proof.status(), StatusCode::BAD_REQUEST);

        let cookie_only = test_router(devices.clone(), Arc::new(FakeAuthentication::default()))
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/devices/current/release")
                    .header(header::ORIGIN, "https://app.agent-room.test")
                    .header(header::COOKIE, "__Host-agent-room-session=session-secret")
                    .body(Body::empty())
                    .expect("请求有效"),
            )
            .await
            .expect("路由执行成功");
        assert_eq!(cookie_only.status(), StatusCode::UNAUTHORIZED);

        assert!(devices.revocation.lock().expect("撤销记录锁可用").is_none());
        assert!(devices.departed.lock().expect("离开记录锁可用").is_empty());
        assert!(
            devices
                .signed_requests
                .lock()
                .expect("签名请求记录锁可用")
                .is_empty()
        );
    }
}
