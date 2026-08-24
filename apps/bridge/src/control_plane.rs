use std::time::Duration;

use agent_room_application::{
    devices::{AuthenticatedDevice, DeviceCredentials},
    ports::{PortFuture, PrincipalAccount, SecretValue},
};
use agent_room_bridge_core::ports::{
    ControlPlaneDeviceFailure, ControlPlaneDeviceFailureKind, ControlPlaneDeviceGateway,
    ControlPlaneDeviceResult, RefreshBridgeDevice, RegisterBridgeDevice,
};
use agent_room_domain::{
    identity::Principal,
    ids::{ContentId, DeviceId, PrincipalId},
    time::UtcMillis,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{Client, StatusCode, redirect::Policy};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

const REGISTER_DEVICE_PATH: &str = "auth/devices/register";
const REFRESH_DEVICE_PATH: &str = "auth/devices/refresh";
const DEVICE_ID_HEADER: &str = "x-agent-room-device-id";
const PROOF_ISSUED_AT_HEADER: &str = "x-agent-room-proof-issued-at";
const PROOF_NONCE_HEADER: &str = "x-agent-room-proof-nonce";
const PROOF_SIGNATURE_HEADER: &str = "x-agent-room-proof-signature";
const MAX_RESPONSE_BYTES: usize = 64 * 1_024;

pub(crate) struct ControlPlaneHttpConfig {
    pub(crate) base_url: String,
    pub(crate) request_timeout: Duration,
}

pub(crate) struct ReqwestControlPlaneDeviceGateway {
    client: Client,
    register_url: Url,
    refresh_url: Url,
}

impl ReqwestControlPlaneDeviceGateway {
    /// 创建不跟随重定向、限制超时且只向固定控制面发送凭据的客户端。
    ///
    /// # Errors
    ///
    /// URL、明文传输边界或 HTTP 客户端配置无效时返回稳定配置错误。
    pub(crate) fn new(
        config: &ControlPlaneHttpConfig,
    ) -> Result<Self, ControlPlaneHttpConfigurationError> {
        if config.request_timeout.is_zero() {
            return Err(ControlPlaneHttpConfigurationError::InvalidTimeout);
        }
        let base_url = Url::parse(&config.base_url)
            .map_err(|_| ControlPlaneHttpConfigurationError::InvalidBaseUrl)?;
        validate_base_url(&base_url)?;
        let register_url = base_url
            .join(REGISTER_DEVICE_PATH)
            .map_err(|_| ControlPlaneHttpConfigurationError::InvalidBaseUrl)?;
        let refresh_url = base_url
            .join(REFRESH_DEVICE_PATH)
            .map_err(|_| ControlPlaneHttpConfigurationError::InvalidBaseUrl)?;
        let client = Client::builder()
            .timeout(config.request_timeout)
            .connect_timeout(config.request_timeout)
            .redirect(Policy::none())
            .build()
            .map_err(|_| ControlPlaneHttpConfigurationError::HttpClient)?;
        Ok(Self {
            client,
            register_url,
            refresh_url,
        })
    }

    async fn register_internal(
        &self,
        request: RegisterBridgeDevice,
    ) -> ControlPlaneDeviceResult<DeviceCredentials> {
        let body = RegisterDeviceBody {
            label: request.label,
            platform: request.platform.as_str(),
            public_signing_key: URL_SAFE_NO_PAD.encode(request.public_signing_key.as_bytes()),
            possession_signature: URL_SAFE_NO_PAD.encode(request.possession_signature.as_bytes()),
            import_display_name: request.import_display_name,
            import_locale: request.import_locale,
        };
        let response = self
            .client
            .post(self.register_url.clone())
            .bearer_auth(request.oidc_assertion.expose())
            .json(&body)
            .send()
            .await
            .map_err(|error| transport_failure(&error))?;
        decode_credentials_response(response).await
    }

    async fn refresh_internal(
        &self,
        request: RefreshBridgeDevice,
    ) -> ControlPlaneDeviceResult<DeviceCredentials> {
        if !request.proof.nonce().expose().is_ascii() {
            return Err(failure(ControlPlaneDeviceFailureKind::InvalidRequest));
        }
        let response = self
            .client
            .post(self.refresh_url.clone())
            .bearer_auth(request.refresh_token.expose())
            .header(DEVICE_ID_HEADER, request.proof.device_id().to_string())
            .header(
                PROOF_ISSUED_AT_HEADER,
                request.proof.issued_at().value().to_string(),
            )
            .header(PROOF_NONCE_HEADER, request.proof.nonce().expose())
            .header(
                PROOF_SIGNATURE_HEADER,
                URL_SAFE_NO_PAD.encode(request.proof.signature().as_bytes()),
            )
            .body(Vec::new())
            .send()
            .await
            .map_err(|error| transport_failure(&error))?;
        decode_credentials_response(response).await
    }
}

impl ControlPlaneDeviceGateway for ReqwestControlPlaneDeviceGateway {
    fn register(
        &self,
        request: RegisterBridgeDevice,
    ) -> PortFuture<'_, ControlPlaneDeviceResult<DeviceCredentials>> {
        Box::pin(self.register_internal(request))
    }

    fn refresh(
        &self,
        request: RefreshBridgeDevice,
    ) -> PortFuture<'_, ControlPlaneDeviceResult<DeviceCredentials>> {
        Box::pin(self.refresh_internal(request))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RegisterDeviceBody {
    label: String,
    platform: &'static str,
    public_signing_key: String,
    possession_signature: String,
    import_display_name: bool,
    import_locale: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

async fn decode_credentials_response(
    response: reqwest::Response,
) -> ControlPlaneDeviceResult<DeviceCredentials> {
    let status = response.status();
    if !status.is_success() {
        return Err(status_failure(status));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(failure(ControlPlaneDeviceFailureKind::UnknownCommit));
    }
    let body = read_limited_body(response).await?;
    let response = serde_json::from_slice::<DeviceCredentialsResponse>(&body)
        .map_err(|_| failure(ControlPlaneDeviceFailureKind::UnknownCommit))?;
    response
        .try_into()
        .map_err(|()| failure(ControlPlaneDeviceFailureKind::UnknownCommit))
}

async fn read_limited_body(mut response: reqwest::Response) -> ControlPlaneDeviceResult<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| failure(ControlPlaneDeviceFailureKind::UnknownCommit))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(failure(ControlPlaneDeviceFailureKind::UnknownCommit));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

impl TryFrom<DeviceCredentialsResponse> for DeviceCredentials {
    type Error = ();

    fn try_from(value: DeviceCredentialsResponse) -> Result<Self, Self::Error> {
        validate_account_text(&value.matrix_user_id, 512)?;
        if !value.matrix_user_id.starts_with('@') || !value.matrix_user_id.contains(':') {
            return Err(());
        }
        validate_account_text(&value.display_name, 128)?;
        validate_locale(&value.locale)?;
        let device_id = parse_id(&value.device_id).map(DeviceId::from_uuid)?;
        let principal_id = parse_id(&value.principal_id).map(PrincipalId::from_uuid)?;
        let avatar_content_id = value
            .avatar_content_id
            .map(|content_id| parse_id(&content_id).map(ContentId::from_uuid))
            .transpose()?;
        let access_token = SecretValue::new(value.access_token).map_err(|_| ())?;
        let refresh_token = SecretValue::new(value.refresh_token).map_err(|_| ())?;
        let access_token_expires_at =
            UtcMillis::new(value.access_token_expires_at_unix_ms).map_err(|_| ())?;
        let refresh_token_expires_at =
            UtcMillis::new(value.refresh_token_expires_at_unix_ms).map_err(|_| ())?;
        if access_token_expires_at >= refresh_token_expires_at {
            return Err(());
        }

        Ok(Self {
            device: AuthenticatedDevice {
                account: PrincipalAccount {
                    principal: Principal::new(principal_id),
                    matrix_user_id: value.matrix_user_id,
                    display_name: value.display_name,
                    avatar_content_id,
                    locale: value.locale,
                },
                device_id,
                access_token_expires_at,
            },
            access_token,
            refresh_token,
            refresh_token_expires_at,
        })
    }
}

fn validate_base_url(url: &Url) -> Result<(), ControlPlaneHttpConfigurationError> {
    let is_loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || url.scheme() == "http" && !is_loopback
    {
        return Err(ControlPlaneHttpConfigurationError::InvalidBaseUrl);
    }
    Ok(())
}

fn parse_id(value: &str) -> Result<Uuid, ()> {
    Uuid::parse_str(value).map_err(|_| ())
}

fn validate_account_text(value: &str, maximum_length: usize) -> Result<(), ()> {
    if value.is_empty() || value.len() > maximum_length || value.chars().any(char::is_control) {
        return Err(());
    }
    Ok(())
}

fn validate_locale(value: &str) -> Result<(), ()> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(());
    }
    Ok(())
}

fn transport_failure(error: &reqwest::Error) -> ControlPlaneDeviceFailure {
    if error.is_connect() {
        failure(ControlPlaneDeviceFailureKind::DependencyUnavailable)
    } else {
        failure(ControlPlaneDeviceFailureKind::UnknownCommit)
    }
}

fn status_failure(status: StatusCode) -> ControlPlaneDeviceFailure {
    let kind = match status {
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
            ControlPlaneDeviceFailureKind::InvalidRequest
        }
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            ControlPlaneDeviceFailureKind::AuthenticationRejected
        }
        StatusCode::CONFLICT => ControlPlaneDeviceFailureKind::Conflict,
        StatusCode::REQUEST_TIMEOUT
        | StatusCode::TOO_MANY_REQUESTS
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => ControlPlaneDeviceFailureKind::DependencyUnavailable,
        _ if status.is_server_error() => ControlPlaneDeviceFailureKind::DependencyUnavailable,
        _ => ControlPlaneDeviceFailureKind::Internal,
    };
    failure(kind)
}

const fn failure(kind: ControlPlaneDeviceFailureKind) -> ControlPlaneDeviceFailure {
    ControlPlaneDeviceFailure::new(kind)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlPlaneHttpConfigurationError {
    InvalidBaseUrl,
    InvalidTimeout,
    HttpClient,
}

impl std::fmt::Display for ControlPlaneHttpConfigurationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::InvalidBaseUrl => "控制面地址无效或使用了不安全的明文传输",
            Self::InvalidTimeout => "控制面请求超时配置无效",
            Self::HttpClient => "控制面 HTTP 客户端初始化失败",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ControlPlaneHttpConfigurationError {}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use agent_room_application::{
        devices::{DeviceRequestProof, DeviceRequestProofPayload},
        ports::{DeviceSignature, SecretFactory, SecretValue},
    };
    use agent_room_bridge_core::ports::{
        ControlPlaneDeviceFailureKind, ControlPlaneDeviceGateway, RefreshBridgeDevice,
        RegisterBridgeDevice,
    };
    use agent_room_domain::{
        devices::{DevicePlatform, DevicePublicSigningKey},
        ids::DeviceId,
        time::UtcMillis,
    };
    use agent_room_identity_adapter::SecureSecretFactory;
    use axum::{
        Json, Router,
        body::HttpBody,
        extract::Request,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::post,
    };
    use serde_json::{Value, json};
    use tokio::net::TcpListener;
    use uuid::Uuid;

    use super::{
        ControlPlaneHttpConfig, ControlPlaneHttpConfigurationError,
        ReqwestControlPlaneDeviceGateway,
    };

    #[tokio::test]
    async fn 注册请求只向固定地址发送规范载荷() {
        let app = Router::new().route(
            "/auth/devices/register",
            post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                let valid = headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    == Some("Bearer oidc-assertion")
                    && body
                        == json!({
                            "label": "Windows 设备",
                            "platform": "windows",
                            "publicSigningKey": "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc",
                            "possessionSignature": "CQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQ",
                            "importDisplayName": true,
                            "importLocale": true
                        });
                if valid {
                    (StatusCode::OK, Json(valid_credentials_response())).into_response()
                } else {
                    StatusCode::BAD_REQUEST.into_response()
                }
            }),
        );
        let gateway = gateway(spawn_server(app).await);

        let credentials = gateway
            .register(RegisterBridgeDevice {
                oidc_assertion: secret("oidc-assertion"),
                label: "Windows 设备".to_owned(),
                platform: DevicePlatform::Windows,
                public_signing_key: DevicePublicSigningKey::new(vec![7; 32]).expect("测试公钥有效"),
                possession_signature: DeviceSignature::new(vec![9; 64]).expect("测试签名有效"),
                import_display_name: true,
                import_locale: true,
            })
            .await
            .expect("规范注册请求应成功");

        assert_eq!(credentials.device.device_id, device_id());
        assert_eq!(credentials.device.account.display_name, "测试用户");
        assert_eq!(credentials.access_token.expose(), "access-token");
    }

    #[tokio::test]
    async fn 刷新请求携带发送方约束证明且不在正文复制令牌() {
        let app = Router::new().route(
            "/auth/devices/refresh",
            post(|headers: HeaderMap, request: Request| async move {
                let valid = headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    == Some("Bearer refresh-token")
                    && header(&headers, "x-agent-room-device-id")
                        == Some("00000000-0000-0000-0000-000000000001")
                    && header(&headers, "x-agent-room-proof-issued-at") == Some("1000")
                    && header(&headers, "x-agent-room-proof-nonce")
                        == Some("0123456789abcdef")
                    && header(&headers, "x-agent-room-proof-signature")
                        == Some("BQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQUFBQ")
                    && request.body().size_hint().exact() == Some(0);
                if valid {
                    (StatusCode::OK, Json(valid_credentials_response())).into_response()
                } else {
                    StatusCode::BAD_REQUEST.into_response()
                }
            }),
        );
        let gateway = gateway(spawn_server(app).await);
        let secrets = SecureSecretFactory;
        let payload = DeviceRequestProofPayload::new(
            device_id(),
            UtcMillis::new(1_000).expect("测试时间有效"),
            secret("0123456789abcdef"),
            "POST".to_owned(),
            "/auth/devices/refresh".to_owned(),
            secrets.digest(""),
        )
        .expect("测试证明有效");

        let credentials = gateway
            .refresh(RefreshBridgeDevice {
                refresh_token: secret("refresh-token"),
                proof: DeviceRequestProof::new(
                    payload,
                    DeviceSignature::new(vec![5; 64]).expect("测试签名有效"),
                ),
            })
            .await
            .expect("规范刷新请求应成功");

        assert_eq!(credentials.refresh_token.expose(), "next-refresh-token");
    }

    #[tokio::test]
    async fn 已提交语义不明的响应绝不被当作可安全重试() {
        for app in [
            Router::new().route(
                "/auth/devices/register",
                post(|| async { (StatusCode::OK, "not-json") }),
            ),
            Router::new().route(
                "/auth/devices/register",
                post(|| async { "x".repeat(65 * 1_024) }),
            ),
        ] {
            let failure = gateway(spawn_server(app).await)
                .register(register_request())
                .await
                .expect_err("畸形成功响应必须进入未知提交状态");

            assert_eq!(failure.kind(), ControlPlaneDeviceFailureKind::UnknownCommit);
        }
    }

    #[tokio::test]
    async fn 服务端冲突映射为稳定业务错误() {
        let app = Router::new().route(
            "/auth/devices/register",
            post(|| async { StatusCode::CONFLICT }),
        );

        let failure = gateway(spawn_server(app).await)
            .register(register_request())
            .await
            .expect_err("冲突响应必须失败");

        assert_eq!(failure.kind(), ControlPlaneDeviceFailureKind::Conflict);
    }

    #[test]
    fn 非回环地址禁止使用明文_http() {
        let result = ReqwestControlPlaneDeviceGateway::new(&ControlPlaneHttpConfig {
            base_url: "http://example.com/".to_owned(),
            request_timeout: Duration::from_secs(1),
        });

        assert!(matches!(
            result,
            Err(ControlPlaneHttpConfigurationError::InvalidBaseUrl)
        ));
    }

    fn gateway(base_url: String) -> ReqwestControlPlaneDeviceGateway {
        ReqwestControlPlaneDeviceGateway::new(&ControlPlaneHttpConfig {
            base_url,
            request_timeout: Duration::from_secs(2),
        })
        .expect("本地测试地址有效")
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

    fn register_request() -> RegisterBridgeDevice {
        RegisterBridgeDevice {
            oidc_assertion: secret("oidc-assertion"),
            label: "Windows 设备".to_owned(),
            platform: DevicePlatform::Windows,
            public_signing_key: DevicePublicSigningKey::new(vec![7; 32]).expect("测试公钥有效"),
            possession_signature: DeviceSignature::new(vec![9; 64]).expect("测试签名有效"),
            import_display_name: true,
            import_locale: true,
        }
    }

    fn valid_credentials_response() -> Value {
        json!({
            "deviceId": "00000000-0000-0000-0000-000000000001",
            "principalId": "00000000-0000-0000-0000-000000000002",
            "matrixUserId": "@test:matrix.agent-room.localhost",
            "displayName": "测试用户",
            "avatarContentId": null,
            "locale": "zh-CN",
            "accessToken": "access-token",
            "accessTokenExpiresAtUnixMs": 2_000,
            "refreshToken": "next-refresh-token",
            "refreshTokenExpiresAtUnixMs": 3_000
        })
    }

    fn secret(value: &str) -> SecretValue {
        SecretValue::new(value).expect("测试密钥有效")
    }

    fn device_id() -> DeviceId {
        DeviceId::from_uuid(Uuid::from_u128(1))
    }

    fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
        headers.get(name).and_then(|value| value.to_str().ok())
    }
}
