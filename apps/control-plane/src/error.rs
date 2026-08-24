use std::collections::BTreeMap;

use agent_room_application::{
    agents::{AgentManagementFailure, AgentManagementFailureKind},
    authentication::{AuthenticationFailure, AuthenticationFailureKind},
    devices::{DeviceAuthorizationFailure, DeviceAuthorizationFailureKind},
};
use agent_room_protocol_conformance::generated::{ErrorCategory, ErrorEnvelope};
use axum::{
    Json,
    extract::Extension,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::correlation::CorrelationId;

pub(crate) struct ApiError {
    status: StatusCode,
    envelope: ErrorEnvelope,
}

impl ApiError {
    pub(crate) fn new(
        status: StatusCode,
        code: &str,
        category: ErrorCategory,
        message: &str,
        correlation_id: CorrelationId,
    ) -> Self {
        Self {
            status,
            envelope: ErrorEnvelope {
                category,
                code: code.to_owned(),
                correlation_id: correlation_id.as_uuid().to_string(),
                details: BTreeMap::new(),
                message: message.to_owned(),
                retryable: false,
                retry_after_seconds: None,
                extensions: BTreeMap::new(),
            },
        }
    }

    pub(crate) fn invalid_request(code: &str, correlation_id: CorrelationId) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            code,
            ErrorCategory::Validation,
            "请求无法通过安全校验。",
            correlation_id,
        )
    }

    pub(crate) fn authentication(
        failure: AuthenticationFailure,
        correlation_id: CorrelationId,
    ) -> Self {
        let (status, code, category, message) = match failure.kind() {
            AuthenticationFailureKind::InvalidRequest => (
                StatusCode::BAD_REQUEST,
                "authentication.invalid_request",
                ErrorCategory::Validation,
                "认证请求无效。",
            ),
            AuthenticationFailureKind::InvalidLoginState => (
                StatusCode::BAD_REQUEST,
                "authentication.invalid_login_state",
                ErrorCategory::Validation,
                "登录状态已失效，请重新开始登录。",
            ),
            AuthenticationFailureKind::ProviderRejected => (
                StatusCode::UNAUTHORIZED,
                "authentication.provider_rejected",
                ErrorCategory::Authentication,
                "身份提供方拒绝了登录。",
            ),
            AuthenticationFailureKind::InvalidIdentityToken => (
                StatusCode::UNAUTHORIZED,
                "authentication.invalid_identity_token",
                ErrorCategory::Authentication,
                "身份声明无法验证。",
            ),
            AuthenticationFailureKind::InvalidSession => (
                StatusCode::UNAUTHORIZED,
                "authentication.invalid_session",
                ErrorCategory::Authentication,
                "会话无效或已过期。",
            ),
            AuthenticationFailureKind::PrincipalSuspended => (
                StatusCode::FORBIDDEN,
                "authentication.principal_suspended",
                ErrorCategory::Authorization,
                "该主体已暂停。",
            ),
            AuthenticationFailureKind::ReauthenticationRequired => (
                StatusCode::UNAUTHORIZED,
                "authentication.reauthentication_required",
                ErrorCategory::Authentication,
                "该操作需要近期重新认证。",
            ),
            AuthenticationFailureKind::DependencyUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "authentication.dependency_unavailable",
                ErrorCategory::DependencyUnavailable,
                "认证依赖暂时不可用。",
            ),
            AuthenticationFailureKind::Conflict => (
                StatusCode::CONFLICT,
                "authentication.conflict",
                ErrorCategory::Conflict,
                "认证状态发生冲突，请重试。",
            ),
            AuthenticationFailureKind::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "authentication.internal",
                ErrorCategory::Transient,
                "认证服务发生内部错误。",
            ),
        };
        tracing::warn!(
            correlation.id = %correlation_id.as_uuid(),
            operation = failure.operation(),
            failure = ?failure.kind(),
            "认证请求失败"
        );
        Self::new(status, code, category, message, correlation_id)
    }

    pub(crate) fn device(
        failure: DeviceAuthorizationFailure,
        correlation_id: CorrelationId,
    ) -> Self {
        let (status, code, category, message) = match failure.kind() {
            DeviceAuthorizationFailureKind::InvalidRequest => (
                StatusCode::BAD_REQUEST,
                "device.invalid_request",
                ErrorCategory::Validation,
                "设备请求无效。",
            ),
            DeviceAuthorizationFailureKind::InvalidAuthorization => (
                StatusCode::UNAUTHORIZED,
                "device.invalid_authorization",
                ErrorCategory::Authentication,
                "设备授权无效或已过期。",
            ),
            DeviceAuthorizationFailureKind::InvalidToken => (
                StatusCode::UNAUTHORIZED,
                "device.invalid_token",
                ErrorCategory::Authentication,
                "设备凭据无效或已过期。",
            ),
            DeviceAuthorizationFailureKind::InvalidProof => (
                StatusCode::UNAUTHORIZED,
                "device.invalid_proof",
                ErrorCategory::Authentication,
                "设备持有证明无效。",
            ),
            DeviceAuthorizationFailureKind::ProofReplay => (
                StatusCode::UNAUTHORIZED,
                "device.proof_replay",
                ErrorCategory::Authentication,
                "设备请求证明已被使用。",
            ),
            DeviceAuthorizationFailureKind::RefreshTokenReuse => (
                StatusCode::UNAUTHORIZED,
                "device.refresh_token_reuse",
                ErrorCategory::Authentication,
                "检测到刷新凭据重用，设备会话已撤销。",
            ),
            DeviceAuthorizationFailureKind::PrincipalSuspended => (
                StatusCode::FORBIDDEN,
                "device.principal_suspended",
                ErrorCategory::Authorization,
                "该主体已暂停。",
            ),
            DeviceAuthorizationFailureKind::DeviceRevoked => (
                StatusCode::FORBIDDEN,
                "device.revoked",
                ErrorCategory::Authorization,
                "该设备已撤销。",
            ),
            DeviceAuthorizationFailureKind::NotFound => (
                StatusCode::NOT_FOUND,
                "device.not_found",
                ErrorCategory::Validation,
                "设备不存在。",
            ),
            DeviceAuthorizationFailureKind::Conflict => (
                StatusCode::CONFLICT,
                "device.conflict",
                ErrorCategory::Conflict,
                "设备状态发生冲突，请重新授权。",
            ),
            DeviceAuthorizationFailureKind::DependencyUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "device.dependency_unavailable",
                ErrorCategory::DependencyUnavailable,
                "设备认证依赖暂时不可用。",
            ),
            DeviceAuthorizationFailureKind::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "device.internal",
                ErrorCategory::Transient,
                "设备认证服务发生内部错误。",
            ),
        };
        tracing::warn!(
            correlation.id = %correlation_id.as_uuid(),
            operation = failure.operation(),
            failure = ?failure.kind(),
            "设备认证请求失败"
        );
        Self::new(status, code, category, message, correlation_id)
    }

    pub(crate) fn agent(failure: AgentManagementFailure, correlation_id: CorrelationId) -> Self {
        let (status, code, category, message) = match failure.kind() {
            AgentManagementFailureKind::InvalidRequest => (
                StatusCode::BAD_REQUEST,
                "agent.invalid_request",
                ErrorCategory::Validation,
                "Agent 请求无效。",
            ),
            AgentManagementFailureKind::Forbidden => (
                StatusCode::FORBIDDEN,
                "agent.forbidden",
                ErrorCategory::Authorization,
                "无权操作该 Agent。",
            ),
            AgentManagementFailureKind::NotFound => (
                StatusCode::NOT_FOUND,
                "agent.not_found",
                ErrorCategory::Validation,
                "Agent 不存在。",
            ),
            AgentManagementFailureKind::Conflict => (
                StatusCode::CONFLICT,
                "agent.conflict",
                ErrorCategory::Conflict,
                "Agent 状态或幂等请求发生冲突。",
            ),
            AgentManagementFailureKind::DependencyUnavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "agent.dependency_unavailable",
                ErrorCategory::DependencyUnavailable,
                "Agent 身份依赖暂时不可用。",
            ),
            AgentManagementFailureKind::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "agent.internal",
                ErrorCategory::Transient,
                "Agent 服务发生内部错误。",
            ),
        };
        tracing::warn!(
            correlation.id = %correlation_id.as_uuid(),
            operation = failure.operation(),
            failure = ?failure.kind(),
            "Agent 请求失败"
        );
        Self::new(status, code, category, message, correlation_id)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.envelope)).into_response()
    }
}

pub(crate) async fn not_found(Extension(correlation_id): Extension<CorrelationId>) -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "http.route_not_found",
        ErrorCategory::Validation,
        "请求的资源不存在。",
        correlation_id,
    )
}

pub(crate) async fn method_not_allowed(
    Extension(correlation_id): Extension<CorrelationId>,
) -> ApiError {
    ApiError::new(
        StatusCode::METHOD_NOT_ALLOWED,
        "http.method_not_allowed",
        ErrorCategory::Validation,
        "该资源不支持当前请求方法。",
        correlation_id,
    )
}
