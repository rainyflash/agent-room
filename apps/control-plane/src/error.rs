use std::collections::BTreeMap;

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
    fn new(
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
