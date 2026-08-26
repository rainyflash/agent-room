use std::sync::Arc;

use agent_room_application::{
    account_lifecycle::{
        AccountLifecycleUseCases, ExportAccount, InspectAccountDeletion, ReplayAccountDeletion,
        RequestAccountDeletion, StartedAccountDeletion,
    },
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    ports::{AccountDeletionStage, AccountDeletionStatus, AccountExportSnapshot, SecretValue},
};
use agent_room_domain::ids::AccountDeletionJobId;
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Extension, State, rejection::JsonRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::{authenticate_session, expired_session_jar, no_store, origin_matches},
        resource_ids::parse_uuid_v7,
    },
};

const MAX_ACCOUNT_BODY_BYTES: usize = 2 * 1_024;
const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";
const DELETION_RECEIPT_SCHEME: &str = "DeletionReceipt ";

#[derive(Clone)]
pub(crate) struct AccountHttpState {
    lifecycle: Arc<dyn AccountLifecycleUseCases>,
    authentication: Arc<dyn AuthenticationUseCases>,
    frontend_origin: String,
}

impl AccountHttpState {
    pub(crate) fn new(
        lifecycle: Arc<dyn AccountLifecycleUseCases>,
        authentication: Arc<dyn AuthenticationUseCases>,
        frontend_origin: &url::Url,
    ) -> Self {
        Self {
            lifecycle,
            authentication,
            frontend_origin: frontend_origin.origin().ascii_serialization(),
        }
    }
}

pub(crate) fn router(state: AccountHttpState) -> Router {
    Router::new()
        .route("/account/export", get(export_account))
        .route("/account", axum::routing::delete(request_deletion))
        .route("/account/deletion", get(inspect_deletion))
        .layer(DefaultBodyLimit::max(MAX_ACCOUNT_BODY_BYTES))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeleteAccountBody {
    confirmation: String,
    federation_residual_acknowledged: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountExportResponse {
    schema_version: u16,
    generated_at_unix_ms: i64,
    data: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartedDeletionResponse {
    receipt: String,
    progress: DeletionProgressResponse,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeletionProgressResponse {
    job_id: String,
    stage: &'static str,
    attempt_count: u16,
    requested_at_unix_ms: i64,
    updated_at_unix_ms: i64,
    retry_at_unix_ms: Option<i64>,
    completed_at_unix_ms: Option<i64>,
    failure_code: Option<String>,
    local_data: &'static str,
    matrix_account: &'static str,
    federated_copies: &'static str,
}

async fn export_account(
    State(state): State<AccountHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    jar: CookieJar,
) -> Response {
    let actor = match authenticate_session(
        state.authentication.as_ref(),
        &jar,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    match state.lifecycle.export(ExportAccount { actor }).await {
        Ok(snapshot) => {
            let mut response =
                no_store(Json(AccountExportResponse::from(snapshot)).into_response());
            response.headers_mut().insert(
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=agent-room-account-export.json"),
            );
            response
        }
        Err(failure) => no_store(ApiError::account(failure, correlation_id).into_response()),
    }
}

async fn request_deletion(
    State(state): State<AccountHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<DeleteAccountBody>, JsonRejection>,
) -> Response {
    if !origin_matches(&headers, &state.frontend_origin) {
        return no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "account.invalid_origin",
                ErrorCategory::Authorization,
                "账户删除请求来源无效。",
                correlation_id,
            )
            .into_response(),
        );
    }
    let Some(job_id) = headers
        .get(IDEMPOTENCY_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_uuid_v7(value).ok())
        .map(AccountDeletionJobId::from_uuid)
    else {
        return no_store(
            ApiError::invalid_request("account.invalid_idempotency_key", correlation_id)
                .into_response(),
        );
    };
    let Ok(Json(body)) = body else {
        return no_store(
            ApiError::invalid_request("account.invalid_deletion_body", correlation_id)
                .into_response(),
        );
    };
    let authentication = authenticate_session(
        state.authentication.as_ref(),
        &jar,
        AuthenticationRequirement::RecentAuthentication,
        correlation_id,
    )
    .await;
    let actor = match authentication {
        Ok(actor) => actor,
        Err(authentication_response) => {
            return match state
                .lifecycle
                .replay_deletion(ReplayAccountDeletion {
                    job_id,
                    confirmation: body.confirmation,
                    federation_residual_acknowledged: body.federation_residual_acknowledged,
                })
                .await
            {
                Ok(started) => started_deletion_response(jar, started),
                Err(_) => authentication_response,
            };
        }
    };
    match state
        .lifecycle
        .request_deletion(RequestAccountDeletion {
            actor,
            job_id,
            confirmation: body.confirmation,
            federation_residual_acknowledged: body.federation_residual_acknowledged,
        })
        .await
    {
        Ok(started) => started_deletion_response(jar, started),
        Err(failure) => no_store(ApiError::account(failure, correlation_id).into_response()),
    }
}

fn started_deletion_response(jar: CookieJar, started: StartedAccountDeletion) -> Response {
    let mut response = (
        expired_session_jar(jar),
        Json(StartedDeletionResponse {
            receipt: started.receipt.expose().to_owned(),
            progress: DeletionProgressResponse::from(started.status),
        }),
    )
        .into_response();
    *response.status_mut() = StatusCode::ACCEPTED;
    no_store(response)
}

async fn inspect_deletion(
    State(state): State<AccountHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
) -> Response {
    let Some(receipt) = deletion_receipt(&headers) else {
        return no_store(
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "account.invalid_deletion_receipt",
                ErrorCategory::Authentication,
                "账户删除回执无效。",
                correlation_id,
            )
            .into_response(),
        );
    };
    match state
        .lifecycle
        .inspect_deletion(InspectAccountDeletion { receipt })
        .await
    {
        Ok(status) => no_store(Json(DeletionProgressResponse::from(status)).into_response()),
        Err(failure) => no_store(ApiError::account(failure, correlation_id).into_response()),
    }
}

fn deletion_receipt(headers: &HeaderMap) -> Option<SecretValue> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix(DELETION_RECEIPT_SCHEME))
        .and_then(|value| SecretValue::new(value).ok())
}

impl From<AccountExportSnapshot> for AccountExportResponse {
    fn from(value: AccountExportSnapshot) -> Self {
        Self {
            schema_version: value.schema_version,
            generated_at_unix_ms: value.generated_at.value(),
            data: value.data,
        }
    }
}

impl From<AccountDeletionStatus> for DeletionProgressResponse {
    fn from(value: AccountDeletionStatus) -> Self {
        let (local_data, matrix_account) = match value.stage {
            AccountDeletionStage::Queued | AccountDeletionStage::RetryScheduled => {
                ("revoked_pending_erasure", "pending_deactivation")
            }
            AccountDeletionStage::FederatedDeactivation => {
                ("revoked_pending_erasure", "deactivation_in_progress")
            }
            AccountDeletionStage::LocalErasure => ("erasure_in_progress", "deactivated_and_erased"),
            AccountDeletionStage::Completed => (
                "anonymized_objects_queued_for_erasure_or_audit_tombstone",
                "deactivated_erased_sso_unlinked_and_media_deleted",
            ),
        };
        Self {
            job_id: value.job_id.to_string(),
            stage: value.stage.as_str(),
            attempt_count: value.attempt_count,
            requested_at_unix_ms: value.requested_at.value(),
            updated_at_unix_ms: value.updated_at.value(),
            retry_at_unix_ms: value
                .retry_at
                .map(agent_room_domain::time::UtcMillis::value),
            completed_at_unix_ms: value
                .completed_at
                .map(agent_room_domain::time::UtcMillis::value),
            failure_code: value.failure_code,
            local_data,
            matrix_account,
            federated_copies: "remote_deletion_not_guaranteed",
        }
    }
}
