use std::{error::Error as StdError, str::FromStr, time::Duration};

use agent_room_application::ports::{
    MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult,
};
use agent_room_domain::time::DurationMillis;
use matrix_sdk::ruma::api::error::{ErrorKind, RetryAfter};
use matrix_sdk::{ClientBuildError, Error, HttpError};
use matrix_sdk::{
    encryption::DuplicateOneTimeKeyErrorMessage,
    ruma::api::error::{ErrorBody, StandardErrorBody},
};

pub(crate) fn map_sdk_error(operation: MatrixOperation, error: &Error) -> MatrixFailure {
    if let Some(api_error) = error.as_client_api_error() {
        return map_api_error(operation, api_error);
    }
    match error {
        Error::AuthenticationRequired => {
            MatrixFailure::new(operation, MatrixFailureKind::Unauthenticated)
        }
        Error::Timeout => transport_failure(operation, true),
        Error::Http(http_error) => map_http_error(operation, http_error),
        Error::InsufficientData => MatrixFailure::new(operation, MatrixFailureKind::NotFound),
        Error::Identifier(_) | Error::Url(_) => {
            MatrixFailure::new(operation, MatrixFailureKind::InvalidResponse)
        }
        Error::SerdeJson(_) => MatrixFailure::new(operation, MatrixFailureKind::InvalidResponse),
        _ => dependency_unavailable(operation, error),
    }
}

/// 归成“依赖不可用”时把根上的原因记下来：`MatrixFailure` 只带种类，上层的日志看不出是
/// 本机 `SQLite` 打不开、文件句柄用完了，还是连不上服务器（agent-room-ops#83）。
pub(crate) fn dependency_unavailable(
    operation: MatrixOperation,
    error: &(dyn StdError + 'static),
) -> MatrixFailure {
    log_dependency_cause(operation, error);
    MatrixFailure::new(operation, MatrixFailureKind::DependencyUnavailable)
}

fn log_dependency_cause(operation: MatrixOperation, error: &(dyn StdError + 'static)) {
    tracing::warn!(
        operation = ?operation,
        cause = %root_cause(error),
        "Matrix 客户端的依赖不可用"
    );
}

/// 错误链最底下那一层。只记这一层：`reqwest` 那一层的说明带着完整的请求地址，里面有账户和房间。
fn root_cause<'a>(error: &'a (dyn StdError + 'static)) -> &'a (dyn StdError + 'static) {
    let mut current = error;
    while let Some(next) = current.source() {
        current = next;
    }
    current
}

pub(crate) fn map_build_error(
    operation: MatrixOperation,
    error: &ClientBuildError,
) -> MatrixFailure {
    match error {
        ClientBuildError::MissingHomeserver
        | ClientBuildError::InvalidServerName
        | ClientBuildError::Url(_)
        | ClientBuildError::SlidingSyncVersion(_) => {
            MatrixFailure::new(operation, MatrixFailureKind::InvalidConfiguration)
        }
        ClientBuildError::Http(http_error) => map_http_error(operation, http_error),
        ClientBuildError::AutoDiscovery(_) => {
            MatrixFailure::new(operation, MatrixFailureKind::InvalidResponse)
        }
        ClientBuildError::SqliteStore(store_error) => {
            dependency_unavailable(operation, store_error)
        }
    }
}

pub(crate) fn invalid_response<T>(operation: MatrixOperation) -> MatrixResult<T> {
    Err(MatrixFailure::new(
        operation,
        MatrixFailureKind::InvalidResponse,
    ))
}

pub(crate) fn map_http_error(operation: MatrixOperation, error: &HttpError) -> MatrixFailure {
    if let Some(api_error) = error.as_client_api_error() {
        return map_api_error(operation, api_error);
    }
    match error {
        HttpError::Reqwest(request_error) => {
            if request_error.is_connect() {
                return dependency_unavailable(operation, request_error);
            }
            let failure = transport_failure(operation, request_error.is_timeout());
            if failure.kind() == MatrixFailureKind::DependencyUnavailable {
                log_dependency_cause(operation, request_error);
            }
            failure
        }
        HttpError::Api(_) | HttpError::IntoHttp(_) => {
            MatrixFailure::new(operation, MatrixFailureKind::InvalidResponse)
        }
        HttpError::RefreshToken(_) => {
            MatrixFailure::new(operation, MatrixFailureKind::Unauthenticated)
        }
        HttpError::Cached(inner) => map_http_error(operation, inner),
    }
}

fn map_api_error(
    operation: MatrixOperation,
    error: &matrix_sdk::ruma::api::error::Error,
) -> MatrixFailure {
    if operation == MatrixOperation::Sync && is_duplicate_one_time_key_error(error) {
        return MatrixFailure::new(operation, MatrixFailureKind::CryptographicIdentityConflict);
    }
    if error.status_code == reqwest::StatusCode::FORBIDDEN {
        let kind = if operation == MatrixOperation::Login {
            MatrixFailureKind::AuthenticationRejected
        } else {
            MatrixFailureKind::Forbidden
        };
        return MatrixFailure::new(operation, kind);
    }
    let Some(kind) = error.error_kind() else {
        return map_status(operation, error.status_code.as_u16());
    };
    match kind {
        ErrorKind::LimitExceeded(details) => {
            MatrixFailure::rate_limited(operation, retry_delay(details.retry_after.as_ref()))
        }
        ErrorKind::MissingToken | ErrorKind::UnknownToken(_) | ErrorKind::Unauthorized => {
            MatrixFailure::new(operation, MatrixFailureKind::Unauthenticated)
        }
        ErrorKind::Forbidden if operation == MatrixOperation::Login => {
            MatrixFailure::new(operation, MatrixFailureKind::AuthenticationRejected)
        }
        ErrorKind::Forbidden
        | ErrorKind::GuestAccessForbidden
        | ErrorKind::InviteBlocked
        | ErrorKind::UnableToAuthorizeJoin => {
            MatrixFailure::new(operation, MatrixFailureKind::Forbidden)
        }
        ErrorKind::NotFound => MatrixFailure::new(operation, MatrixFailureKind::NotFound),
        ErrorKind::RoomInUse | ErrorKind::BadState | ErrorKind::InvalidRoomState => {
            MatrixFailure::new(operation, MatrixFailureKind::Conflict)
        }
        ErrorKind::UnknownPos => MatrixFailure::new(operation, MatrixFailureKind::StaleSyncToken),
        ErrorKind::Unrecognized | ErrorKind::IncompatibleRoomVersion(_) => {
            MatrixFailure::new(operation, MatrixFailureKind::UnsupportedVersion)
        }
        _ => map_status(operation, error.status_code.as_u16()),
    }
}

fn is_duplicate_one_time_key_error(error: &matrix_sdk::ruma::api::error::Error) -> bool {
    if error.status_code != reqwest::StatusCode::BAD_REQUEST {
        return false;
    }
    let ErrorBody::Standard(StandardErrorBody { message, .. }) = &error.body else {
        return false;
    };
    DuplicateOneTimeKeyErrorMessage::from_str(message).is_ok()
}

fn map_status(operation: MatrixOperation, status: u16) -> MatrixFailure {
    let kind = match status {
        401 => MatrixFailureKind::Unauthenticated,
        403 if operation == MatrixOperation::Login => MatrixFailureKind::AuthenticationRejected,
        403 => MatrixFailureKind::Forbidden,
        404 => MatrixFailureKind::NotFound,
        409 => MatrixFailureKind::Conflict,
        429 => MatrixFailureKind::RateLimited,
        500..=599 => MatrixFailureKind::DependencyUnavailable,
        _ => MatrixFailureKind::InvalidResponse,
    };
    MatrixFailure::new(operation, kind)
}

fn transport_failure(operation: MatrixOperation, timed_out: bool) -> MatrixFailure {
    let kind = if matches!(
        operation,
        MatrixOperation::CreateRoom | MatrixOperation::SendEvent
    ) {
        MatrixFailureKind::UnknownCommit
    } else if timed_out {
        MatrixFailureKind::Timeout
    } else {
        MatrixFailureKind::DependencyUnavailable
    };
    MatrixFailure::new(operation, kind)
}

fn retry_delay(value: Option<&RetryAfter>) -> Option<DurationMillis> {
    let RetryAfter::Delay(delay) = value? else {
        return None;
    };
    duration_millis(*delay)
}

fn duration_millis(value: Duration) -> Option<DurationMillis> {
    let millis = u64::try_from(value.as_millis()).ok()?.max(1);
    DurationMillis::new(millis).ok()
}

#[cfg(test)]
mod tests {
    use agent_room_application::ports::{MatrixFailureKind, MatrixOperation};
    use matrix_sdk::{
        Error,
        reqwest::StatusCode,
        ruma::api::error::{Error as ApiError, ErrorBody, ErrorKind, StandardErrorBody},
    };

    use super::{map_api_error, map_sdk_error, root_cause};

    #[derive(Debug, thiserror::Error)]
    #[error("请求 http://synapse:8008/_matrix/client/v3/rooms/某个房间 失败")]
    struct Outer(#[source] std::io::Error);

    #[test]
    fn 依赖不可用只记最底下那层原因_不带请求地址() {
        let error = Outer(std::io::Error::other("Too many open files"));
        let cause = root_cause(&error).to_string();
        assert_eq!(cause, "Too many open files");
    }

    #[test]
    fn 没有下一层时记它自己() {
        let error = std::io::Error::other("打不开");
        assert_eq!(root_cause(&error).to_string(), "打不开");
    }

    #[test]
    fn 登录拒绝和房间权限拒绝不会混为一类() {
        let error = api_error(StatusCode::FORBIDDEN, ErrorKind::Forbidden);
        assert_eq!(
            map_api_error(MatrixOperation::Login, &error).kind(),
            MatrixFailureKind::AuthenticationRejected
        );
        assert_eq!(
            map_api_error(MatrixOperation::Invite, &error).kind(),
            MatrixFailureKind::Forbidden
        );
    }

    #[test]
    fn 被封禁用户的_bad_state_仍按权限拒绝处理() {
        let error = api_error(StatusCode::FORBIDDEN, ErrorKind::BadState);
        assert_eq!(
            map_api_error(MatrixOperation::Join, &error).kind(),
            MatrixFailureKind::Forbidden
        );
    }

    #[test]
    fn 失效同步位置要求重建游标() {
        let error = api_error(StatusCode::BAD_REQUEST, ErrorKind::UnknownPos);
        assert_eq!(
            map_api_error(MatrixOperation::Sync, &error).kind(),
            MatrixFailureKind::StaleSyncToken
        );
    }

    #[test]
    fn 重复一次性密钥被识别为加密身份冲突() {
        let message = concat!(
            r#"One time key signed_curve25519:AAAAAAAAAAA already exists. "#,
            r#"Old key: {"key":"dBcZBzQaiQYWf6rBPh2QypIOB/dxSoTeyaFaxNNbeHs","signatures":{"@example:matrix.org":{"ed25519:AAAAAAAAAA":"Fk45zHAbrd+1j9wZXLjL2Y/+DU/Mnz9yuvlfYBOOT7qExN2Jdud+5BAuNs8nZ/caS4wTF39Kg3zQpzaGERoCBg"}}};"#,
            r#" new key: {'key': 'CY0TWVK1/Kj3ZADuBcGe3UKvpT+IKAPMUsMeJhSDqno', 'signatures': {'@example:matrix.org': {'ed25519:AAAAAAAAAA': 'BQ9Gp0p+6srF+c8OyruqKKd9R4yaub3THYAyyBB/7X/rG8BwcAqFynzl1aGyFYun4Q+087a5OSiglCXI+/kQAA'}}}"#
        );
        let error = ApiError::new(
            StatusCode::BAD_REQUEST,
            ErrorBody::Standard(StandardErrorBody::new(
                ErrorKind::BadJson,
                message.to_owned(),
            )),
        );

        assert_eq!(
            map_api_error(MatrixOperation::Sync, &error).kind(),
            MatrixFailureKind::CryptographicIdentityConflict
        );
        assert_ne!(
            map_api_error(MatrixOperation::Login, &error).kind(),
            MatrixFailureKind::CryptographicIdentityConflict
        );
    }

    #[test]
    fn 非幂等写入超时必须进入未知提交对账而不是重试() {
        assert_eq!(
            map_sdk_error(MatrixOperation::CreateRoom, &Error::Timeout).kind(),
            MatrixFailureKind::UnknownCommit
        );
        assert_eq!(
            map_sdk_error(MatrixOperation::SendEvent, &Error::Timeout).kind(),
            MatrixFailureKind::UnknownCommit
        );
        assert_eq!(
            map_sdk_error(MatrixOperation::Sync, &Error::Timeout).kind(),
            MatrixFailureKind::Timeout
        );
        assert_eq!(
            map_sdk_error(MatrixOperation::SendStateEvent, &Error::Timeout).kind(),
            MatrixFailureKind::Timeout
        );
    }

    fn api_error(status: StatusCode, kind: ErrorKind) -> ApiError {
        ApiError::new(
            status,
            ErrorBody::Standard(StandardErrorBody::new(kind, "测试错误".to_owned())),
        )
    }
}
