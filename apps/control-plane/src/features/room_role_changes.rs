//! 一间房的身份变动记录（`specs/redesign/room-roles.md` 的“记录”）：谁、什么时候、把谁、从什么改成了
//! 什么。只给房主和管理员看。
//!
//! 是新的接口，不往私人房间的回答里加字段：网页对它 `.strict()` 校验，旧版桌面端多看到一个字段整个
//! 房间都读不出来。身份是五项权限上面的一层名字，所以两头给的是成员状态、是不是房主和权限，叫什么
//! 身份由网页算，和成员一节同一套规则。

use std::sync::Arc;

use agent_room_application::{
    authentication::{AuthenticationRequirement, AuthenticationUseCases},
    ports::RoomRoleChangeRecord,
    room_role_changes::{ListRoomRoleChanges, RoomRoleChangeFailure, RoomRoleChangeUseCases},
};
use agent_room_domain::{ids::RoomCatalogId, room_role_changes::RoomStanding};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use axum_extra::extract::CookieJar;
use serde::Serialize;

use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::{authenticate_session, no_store},
        private_rooms::PermissionsResponse,
        resource_ids::parse_uuid_v7,
    },
};

#[derive(Clone)]
pub(crate) struct RoomRoleChangeHttpState {
    pub(crate) role_changes: Arc<dyn RoomRoleChangeUseCases>,
    pub(crate) authentication: Arc<dyn AuthenticationUseCases>,
}

pub(crate) fn router(state: RoomRoleChangeHttpState) -> Router {
    Router::new()
        .route(
            "/private-rooms/{catalog_id}/role-changes",
            get(list_role_changes),
        )
        .with_state(state)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RoleChangesResponse {
    /// 新的在前。
    changes: Vec<RoleChangeResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RoleChangeResponse {
    id: i64,
    /// 不是谁动的手时不写。
    #[serde(skip_serializing_if = "Option::is_none")]
    actor_principal_id: Option<String>,
    target_principal_id: String,
    /// 之前名单里没有这个人时不写。
    #[serde(skip_serializing_if = "Option::is_none")]
    previous: Option<StandingResponse>,
    next: StandingResponse,
    changed_at_unix_ms: i64,
    /// 是它下面哪一间子房间里的（R5）；这间房自己的不写。网页不严格校验这份回答，加字段旧版读得过去。
    #[serde(skip_serializing_if = "Option::is_none")]
    sub_room: Option<SubRoomResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubRoomResponse {
    catalog_id: String,
    name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StandingResponse {
    status: &'static str,
    owner: bool,
    permissions: PermissionsResponse,
}

impl From<RoomStanding> for StandingResponse {
    fn from(standing: RoomStanding) -> Self {
        Self {
            status: standing.status.as_str(),
            owner: standing.owner,
            permissions: PermissionsResponse::from(standing.permissions),
        }
    }
}

impl From<RoomRoleChangeRecord> for RoleChangeResponse {
    fn from(record: RoomRoleChangeRecord) -> Self {
        Self {
            id: record.id,
            actor_principal_id: record.actor.map(|actor| actor.to_string()),
            target_principal_id: record.target.to_string(),
            previous: record.previous.map(StandingResponse::from),
            next: StandingResponse::from(record.next),
            changed_at_unix_ms: record.changed_at.value(),
            sub_room: record.sub_room.map(|sub_room| SubRoomResponse {
                catalog_id: sub_room.catalog_id.to_string(),
                name: sub_room.name,
            }),
        }
    }
}

async fn list_role_changes(
    State(state): State<RoomRoleChangeHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog_id): Path<String>,
    jar: CookieJar,
) -> Response {
    let Ok(catalog_id) = parse_uuid_v7(&catalog_id).map(RoomCatalogId::from_uuid) else {
        return no_store(
            ApiError::invalid_request("room_role_changes.invalid_catalog", correlation_id)
                .into_response(),
        );
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
        .role_changes
        .recent(ListRoomRoleChanges { actor, catalog_id })
        .await
    {
        Ok(changes) => no_store(
            Json(RoleChangesResponse {
                changes: changes.into_iter().map(RoleChangeResponse::from).collect(),
            })
            .into_response(),
        ),
        Err(failure) => no_store(failure_response(failure, correlation_id)),
    }
}

fn failure_response(failure: RoomRoleChangeFailure, correlation_id: CorrelationId) -> Response {
    let (status, code, category, message) = match failure {
        RoomRoleChangeFailure::Forbidden => (
            StatusCode::FORBIDDEN,
            "room_role_changes.forbidden",
            ErrorCategory::Authorization,
            "登录已失效，请重新登录。",
        ),
        RoomRoleChangeFailure::ManagersOnly => (
            StatusCode::FORBIDDEN,
            "room_role_changes.managers_only",
            ErrorCategory::Authorization,
            "只有房主和管理员能看身份变动记录。",
        ),
        RoomRoleChangeFailure::NotFound => (
            StatusCode::NOT_FOUND,
            "room_role_changes.not_found",
            ErrorCategory::Validation,
            "没有这个房间，或者你不在这个房间里。",
        ),
        RoomRoleChangeFailure::DependencyUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "room_role_changes.dependency_unavailable",
            ErrorCategory::DependencyUnavailable,
            "暂时查不到，请稍后再试。",
        ),
        RoomRoleChangeFailure::Internal => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "room_role_changes.internal",
            ErrorCategory::Transient,
            "服务器内部错误。",
        ),
    };
    ApiError::new(status, code, category, message, correlation_id).into_response()
}

#[cfg(test)]
mod tests;
