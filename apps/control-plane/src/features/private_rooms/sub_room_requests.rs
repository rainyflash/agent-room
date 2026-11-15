//! 子房间的申请和批准（`specs/redesign/sub-rooms.md` 第 2d-2 步）：
//!
//! - `POST /private-rooms/{父}/sub-rooms/{子}/join-requests`：申请进要批准的子房间，可以留一句话。
//!   不用批准的（没有门了、父房间的房主和管理员、本来就在里面）直接记成已加入，回 `joined`；网页
//!   接着照“服务器记着已加入、Matrix 上是受邀”那一套替他加入。`DELETE` 撤回自己的。
//! - `GET` 同一个地址：管得了这一间的人看在等的申请；`PUT .../join-requests/{账户}` 批准或拒绝。
//!
//! 回答和公开房间的申请一个样子，网页能用同一套组件。

use agent_room_application::{
    authentication::AuthenticationRequirement,
    ports::PendingJoinRequest,
    private_rooms::{
        AnswerSubRoomJoinRequest, RequestSubRoomJoin, SubRoomJoinState, SubRoomMembershipAction,
    },
};
use agent_room_domain::{ids::RoomCatalogId, room_join_requests::JoinRequestStatus};
use axum::{
    Json,
    body::Bytes,
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};

use super::{
    PrivateRoomHttpState,
    handlers::{invalid, invalid_resource},
    models::{catalog_id, principal_id},
    sub_rooms::sub_room_write_context,
};
use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::authentication::{authenticate_session, no_store},
};

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RequestBody {
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AnswerBody {
    decision: Decision,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Decision {
    Approve,
    Reject,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JoinStateResponse {
    catalog_id: String,
    /// `pending`（在等批准）或者 `joined`（不用等，已经记成已加入）。
    membership: &'static str,
    /// 在等批准时，什么时候申请的。
    #[serde(skip_serializing_if = "Option::is_none")]
    requested_at_unix_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingResponse {
    requests: Vec<PendingRequestResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingRequestResponse {
    principal_id: String,
    display_name: String,
    message: String,
    requested_at_unix_ms: i64,
}

impl From<PendingJoinRequest> for PendingRequestResponse {
    fn from(pending: PendingJoinRequest) -> Self {
        Self {
            principal_id: pending.record.requester.to_string(),
            display_name: pending.requester_display_name,
            message: pending.record.message.as_str().to_owned(),
            requested_at_unix_ms: pending.record.requested_at.value(),
        }
    }
}

/// 申请。正文可以不带（不留话）。
pub(super) async fn request(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((parent, sub_room)): Path<(String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let body = if body.is_empty() {
        RequestBody::default()
    } else {
        match serde_json::from_slice::<RequestBody>(&body) {
            Ok(body) => body,
            Err(_) => return invalid("sub_room.invalid_join_request_body", correlation_id),
        }
    };
    let (actor, parent_catalog_id, catalog_id) =
        match sub_room_write_context(&state, &headers, &jar, &parent, &sub_room, correlation_id)
            .await
        {
            Ok(context) => context,
            Err(response) => return response,
        };
    let result = state
        .sub_rooms
        .request_sub_room_join(RequestSubRoomJoin {
            actor,
            parent_catalog_id,
            catalog_id,
            message: body.message,
        })
        .await;
    match result {
        Ok(joined) => no_store(Json(join_state(catalog_id, joined)).into_response()),
        Err(failure) => no_store(ApiError::private_room(failure, correlation_id).into_response()),
    }
}

/// 撤回自己在等的申请。没有在等的也回 204。
pub(super) async fn withdraw(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((parent, sub_room)): Path<(String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let (actor, parent_catalog_id, catalog_id) =
        match sub_room_write_context(&state, &headers, &jar, &parent, &sub_room, correlation_id)
            .await
        {
            Ok(context) => context,
            Err(response) => return response,
        };
    match state
        .sub_rooms
        .withdraw_sub_room_join_request(SubRoomMembershipAction {
            actor,
            parent_catalog_id,
            catalog_id,
        })
        .await
    {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => no_store(ApiError::private_room(failure, correlation_id).into_response()),
    }
}

/// 管得了这一间的人看在等的申请，先来的在前。只要有效登录：读不改东西。
pub(super) async fn pending(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((parent, sub_room)): Path<(String, String)>,
    jar: CookieJar,
) -> Response {
    let (Some(parent_catalog_id), Some(catalog_id)) = (catalog_id(&parent), catalog_id(&sub_room))
    else {
        return invalid_resource(correlation_id);
    };
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
    match state
        .sub_rooms
        .sub_room_join_requests(SubRoomMembershipAction {
            actor,
            parent_catalog_id,
            catalog_id,
        })
        .await
    {
        Ok(requests) => no_store(
            Json(PendingResponse {
                requests: requests.into_iter().map(Into::into).collect(),
            })
            .into_response(),
        ),
        Err(failure) => no_store(ApiError::private_room(failure, correlation_id).into_response()),
    }
}

/// 批准或拒绝。路径里是申请人的账户 ID：一个人在一间子房间同时只有一个在等的申请。
pub(super) async fn answer(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((parent, sub_room, principal)): Path<(String, String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Bytes,
) -> Response {
    let Ok(body) = serde_json::from_slice::<AnswerBody>(&body) else {
        return invalid("sub_room.invalid_join_request_answer", correlation_id);
    };
    let Some(requester) = principal_id(&principal) else {
        return invalid_resource(correlation_id);
    };
    let (actor, parent_catalog_id, catalog_id) =
        match sub_room_write_context(&state, &headers, &jar, &parent, &sub_room, correlation_id)
            .await
        {
            Ok(context) => context,
            Err(response) => return response,
        };
    match state
        .sub_rooms
        .answer_sub_room_join_request(AnswerSubRoomJoinRequest {
            actor,
            parent_catalog_id,
            catalog_id,
            requester,
            approve: matches!(body.decision, Decision::Approve),
        })
        .await
    {
        Ok(status) => no_store(
            Json(serde_json::json!({
                "status": match status {
                    JoinRequestStatus::Approved => "approved",
                    _ => "rejected",
                }
            }))
            .into_response(),
        ),
        Err(failure) => no_store(ApiError::private_room(failure, correlation_id).into_response()),
    }
}

fn join_state(catalog_id: RoomCatalogId, state: SubRoomJoinState) -> JoinStateResponse {
    let (membership, requested_at_unix_ms) = match state {
        SubRoomJoinState::Pending { requested_at } => ("pending", Some(requested_at.value())),
        SubRoomJoinState::Joined => ("joined", None),
    };
    JoinStateResponse {
        catalog_id: catalog_id.to_string(),
        membership,
        requested_at_unix_ms,
    }
}
