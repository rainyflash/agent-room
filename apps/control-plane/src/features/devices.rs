use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    devices::{
        AuthenticateDeviceRequest, AuthenticatedDevice, DeviceAuthorizationUseCases,
        DeviceCredentials, DeviceRequestProof, DeviceRequestProofPayload, RefreshDeviceSession,
        RegisterDevice, VerifiedDeviceAuthorization,
    },
    ports::{
        DeviceSignature, OidcDeviceAssertionVerifier, OidcFailure, OidcFailureKind,
        ProfileImportConsent, SecretFactory, SecretValue,
    },
};
use agent_room_domain::{
    devices::{Device, DevicePlatform, DevicePublicSigningKey},
    ids::DeviceId,
    time::UtcMillis,
};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
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
    correlation::CorrelationId,
    error::ApiError,
    features::authentication::{authenticate_session, no_store, origin_matches},
};

const REGISTER_DEVICE_PATH: &str = "/auth/devices/register";
const REFRESH_DEVICE_PATH: &str = "/auth/devices/refresh";
const DEVICE_ID_HEADER: &str = "x-agent-room-device-id";
const PROOF_ISSUED_AT_HEADER: &str = "x-agent-room-proof-issued-at";
const PROOF_NONCE_HEADER: &str = "x-agent-room-proof-nonce";
const PROOF_SIGNATURE_HEADER: &str = "x-agent-room-proof-signature";
const MAX_DEVICE_BODY_BYTES: usize = 16 * 1_024;
const MAX_ENCODED_PUBLIC_KEY_LENGTH: usize = 64;
const MAX_ENCODED_SIGNATURE_LENGTH: usize = 128;

#[derive(Clone)]
pub(crate) struct DeviceHttpState {
    devices: Arc<dyn DeviceAuthorizationUseCases>,
    assertion_verifier: Arc<dyn OidcDeviceAssertionVerifier>,
    authentication: Arc<dyn AuthenticationUseCases>,
    secrets: Arc<dyn SecretFactory>,
    frontend_origin: String,
}

pub(crate) struct DeviceHttpDependencies {
    pub(crate) devices: Arc<dyn DeviceAuthorizationUseCases>,
    pub(crate) assertion_verifier: Arc<dyn OidcDeviceAssertionVerifier>,
    pub(crate) authentication: Arc<dyn AuthenticationUseCases>,
    pub(crate) secrets: Arc<dyn SecretFactory>,
}

impl DeviceHttpState {
    pub(crate) fn new(dependencies: DeviceHttpDependencies, frontend_origin: &Url) -> Self {
        Self {
            devices: dependencies.devices,
            assertion_verifier: dependencies.assertion_verifier,
            authentication: dependencies.authentication,
            secrets: dependencies.secrets,
            frontend_origin: frontend_origin.origin().ascii_serialization(),
        }
    }
}

pub(crate) fn router(state: DeviceHttpState) -> Router {
    Router::new()
        .route(REGISTER_DEVICE_PATH, post(register_device))
        .route(REFRESH_DEVICE_PATH, post(refresh_device_session))
        .route("/auth/devices", get(list_devices))
        .route("/auth/devices/{device_id}", delete(revoke_device))
        .layer(DefaultBodyLimit::max(MAX_DEVICE_BODY_BYTES))
        .with_state(state)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegisterDeviceBody {
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
struct DeviceCredentialsResponse {
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
    let identity = state
        .assertion_verifier
        .verify_assertion(&assertion)
        .await
        .map_err(|failure| oidc_assertion_error(failure, correlation_id))?;
    let public_signing_key = DevicePublicSigningKey::new(
        decode_bounded(&body.public_signing_key, MAX_ENCODED_PUBLIC_KEY_LENGTH)
            .map_err(|()| ApiError::invalid_request("device.invalid_public_key", correlation_id))?,
    )
    .map_err(|_| ApiError::invalid_request("device.invalid_public_key", correlation_id))?;
    let possession_signature = DeviceSignature::new(
        decode_bounded(&body.possession_signature, MAX_ENCODED_SIGNATURE_LENGTH).map_err(|()| {
            ApiError::invalid_request("device.invalid_possession_signature", correlation_id)
        })?,
    )
    .map_err(|_| {
        ApiError::invalid_request("device.invalid_possession_signature", correlation_id)
    })?;
    let platform = DevicePlatform::try_from(body.platform.as_str())
        .map_err(|_| ApiError::invalid_request("device.invalid_platform", correlation_id))?;
    let authorization =
        VerifiedDeviceAuthorization::new(identity, state.secrets.digest(assertion.expose()));

    Ok(RegisterDevice {
        authorization,
        label: body.label,
        platform,
        public_signing_key,
        possession_signature,
        profile_import: ProfileImportConsent {
            display_name: body.import_display_name,
            locale: body.import_locale,
        },
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
    match state
        .devices
        .refresh_device_session(RefreshDeviceSession {
            refresh_token: &refresh_token,
            proof: &proof,
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
    if !origin_matches(&headers, &state.frontend_origin) {
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
    match state
        .devices
        .revoke_device(principal.principal_id, DeviceId::from_uuid(device_id))
        .await
    {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
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
    devices
        .authenticate_device(AuthenticateDeviceRequest {
            access_token: &access_token,
            proof: &proof,
        })
        .await
        .map_err(|failure| no_store(ApiError::device(failure, correlation_id).into_response()))
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

fn invalid_bearer(correlation_id: CorrelationId) -> ApiError {
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
        OidcFailureKind::ProviderRejected | OidcFailureKind::InvalidIdentityToken => (
            StatusCode::UNAUTHORIZED,
            "device.invalid_oidc_assertion",
            ErrorCategory::Authentication,
            "设备授权断言无效或已过期。",
        ),
        OidcFailureKind::InvalidConfiguration => (
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
        authentication::{
            AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
            AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
        },
        devices::{
            AuthenticateDeviceRequest, AuthenticatedDevice, DeviceAuthorizationResult,
            DeviceAuthorizationUseCases, DeviceCredentials, RefreshDeviceSession, RegisterDevice,
        },
        ports::{
            OidcDeviceAssertionVerifier, OidcResult, PortFuture, PrincipalAccount, SecretFactory,
            SecretValue, VerifiedOidcIdentity,
        },
    };
    use agent_room_domain::{
        devices::{Device, DevicePlatform, DevicePublicSigningKey},
        identity::Principal,
        ids::{DeviceId, PrincipalId},
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
        revocation: Mutex<Option<(PrincipalId, DeviceId)>>,
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
            _request: AuthenticateDeviceRequest<'a>,
        ) -> PortFuture<'a, DeviceAuthorizationResult<AuthenticatedDevice>> {
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
        ) -> PortFuture<'_, DeviceAuthorizationResult<()>> {
            *self.revocation.lock().expect("撤销记录锁可用") = Some((principal_id, device_id));
            Box::pin(async { Ok(()) })
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
        ) -> PortFuture<'a, OidcResult<VerifiedOidcIdentity>> {
            assert_eq!(assertion.expose(), "oidc-assertion");
            Box::pin(async {
                Ok(VerifiedOidcIdentity::new(
                    "https://identity.example",
                    "stable-subject",
                    Some("Agent Room User".to_owned()),
                    Some("zh-CN".to_owned()),
                    Some(time(1_700_000_000_000)),
                )
                .expect("测试 OIDC 身份有效"))
            })
        }
    }

    fn test_router(
        devices: Arc<FakeDevices>,
        authentication: Arc<FakeAuthentication>,
    ) -> axum::Router {
        let state = DeviceHttpState::new(
            DeviceHttpDependencies {
                devices,
                assertion_verifier: Arc::new(FakeAssertionVerifier),
                authentication,
                secrets: Arc::new(SecureSecretFactory),
            },
            &Url::parse("https://app.agent-room.test").expect("前端 Origin 有效"),
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
    }
}
