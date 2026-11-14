//! 子房间（`specs/redesign/sub-rooms.md` 第 2a 步）：在一间房下面建子房间、读名单，和一间房里
//! 子房间的三项设置（`specs/redesign/room-roles.md` R4）。
//!
//! 都是新的接口，不往私人房间的回答里加字段：网页对它 `.strict()` 校验，旧版桌面端多看到一个字段
//! 整个房间都读不出来。建好的子房间照私人房间的样子回。

use agent_room_application::{
    authentication::AuthenticationRequirement,
    private_rooms::{
        ChangeSubRoomPolicy, CreateSubRoom, InspectSubRoomPolicy, ListSubRooms, SubRoomList,
        SubRoomPolicyView, SubRoomView,
    },
};
use agent_room_domain::{
    private_rooms::PrivateRoomMembershipStatus,
    sub_rooms::{SubRoomCreators, SubRoomPolicy},
    time::UtcMillis,
};
use axum::{
    Json,
    extract::{Extension, Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};

use super::{
    PrivateRoomHttpState,
    handlers::{
        IDEMPOTENCY_KEY_HEADER, invalid, invalid_resource, required_write_context, respond,
    },
    models::catalog_id,
};
use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::authentication::{authenticate_session, no_store},
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CreateSubRoomBody {
    name: String,
    #[serde(default)]
    description: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubRoomListResponse {
    parent: SubRoomParentResponse,
    /// 看的人自己现在能不能在这间房下面建子房间。
    can_create: bool,
    /// 按建的先后。归档的不在这里。
    sub_rooms: Vec<SubRoomResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubRoomParentResponse {
    catalog_id: String,
    name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubRoomResponse {
    catalog_id: String,
    name: String,
    description: String,
    /// `open`、`request`、`invite_only`，和公开房间的加入方式是同一套词。
    access: &'static str,
    hidden: bool,
    created_by_principal_id: String,
    created_at_unix_ms: i64,
    /// 这一间的子房间管理员。房间的房主和管理员本来就管每一间，不在这里。
    manager_principal_ids: Vec<String>,
    /// 看的人在不在里面。
    joined: bool,
    /// 看的人是不是被这一间封禁了。
    banned: bool,
    /// 里面有几个人、几个 Agent、最近有人说话的时间：有门、看的人又不在里面时不写。
    #[serde(skip_serializing_if = "Option::is_none")]
    member_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_message_at_unix_ms: Option<i64>,
}

impl From<SubRoomView> for SubRoomResponse {
    fn from(view: SubRoomView) -> Self {
        let SubRoomView {
            entry,
            inside_visible,
        } = view;
        Self {
            catalog_id: entry.catalog_id.to_string(),
            name: entry.name,
            description: entry.description,
            access: entry.access.as_str(),
            hidden: entry.hidden,
            created_by_principal_id: entry.created_by.to_string(),
            created_at_unix_ms: entry.created_at.value(),
            manager_principal_ids: entry.managers.iter().map(ToString::to_string).collect(),
            joined: entry.viewer_status == Some(PrivateRoomMembershipStatus::Joined),
            banned: entry.viewer_status == Some(PrivateRoomMembershipStatus::Banned),
            member_count: inside_visible.then_some(entry.member_count),
            agent_count: inside_visible.then_some(entry.agent_count),
            last_message_at_unix_ms: entry
                .last_message_at
                .filter(|_| inside_visible)
                .map(UtcMillis::value),
        }
    }
}

impl From<SubRoomList> for SubRoomListResponse {
    fn from(list: SubRoomList) -> Self {
        Self {
            parent: SubRoomParentResponse {
                catalog_id: list.parent_catalog_id.to_string(),
                name: list.parent_name,
            },
            can_create: list.can_create,
            sub_rooms: list
                .sub_rooms
                .into_iter()
                .map(SubRoomResponse::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum CreatorsValue {
    Members,
    Admins,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SubRoomPolicyBody {
    creators: CreatorsValue,
    agents_may_create: bool,
    auto_archive: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubRoomPolicyResponse {
    /// 谁能建子房间：`members`（已加入、能发言的人）或 `admins`（只有房主和管理员）。
    creators: CreatorsValue,
    agents_may_create: bool,
    /// 30 天没人说话的子房间要不要自动归档。
    auto_archive: bool,
    /// 看的人改不改得动：只有房主。
    can_change: bool,
}

impl From<SubRoomPolicyView> for SubRoomPolicyResponse {
    fn from(view: SubRoomPolicyView) -> Self {
        Self {
            creators: match view.policy.creators {
                SubRoomCreators::Members => CreatorsValue::Members,
                SubRoomCreators::Admins => CreatorsValue::Admins,
            },
            agents_may_create: view.policy.agents_may_create,
            auto_archive: view.policy.auto_archive,
            can_change: view.can_change,
        }
    }
}

/// 在一间房下面建子房间。幂等键就是子房间的房间号，同一个请求重发回的是同一间。
pub(super) async fn create(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(parent): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<CreateSubRoomBody>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return invalid("sub_room.invalid_creation_body", correlation_id);
    };
    let (actor, parent_catalog_id) = match required_write_context(
        &state,
        &headers,
        &jar,
        &parent,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    let Some(catalog_id) = headers
        .get(IDEMPOTENCY_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(catalog_id)
    else {
        return invalid("private_room.invalid_idempotency_key", correlation_id);
    };
    respond(
        state
            .sub_rooms
            .create_sub_room(CreateSubRoom {
                actor: actor.into(),
                parent_catalog_id,
                catalog_id,
                // 两头的空白多半是手滑带上的，不为它拒绝。
                name: body.name.trim().to_owned(),
                description: body.description,
                created_by_agent: None,
            })
            .await,
        StatusCode::CREATED,
        correlation_id,
    )
}

/// 一间房下面看的人看得到的子房间。地址里可以是父房间，也可以是它的任何一间子房间：子房间的页面
/// 靠它知道自己挂在谁下面。
pub(super) async fn list(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    jar: CookieJar,
) -> Response {
    let Some(catalog_id) = catalog_id(&catalog) else {
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
        .list_sub_rooms(ListSubRooms { actor, catalog_id })
        .await
    {
        Ok(list) => no_store(Json(SubRoomListResponse::from(list)).into_response()),
        Err(failure) => no_store(ApiError::private_room(failure, correlation_id).into_response()),
    }
}

pub(super) async fn policy(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(parent): Path<String>,
    jar: CookieJar,
) -> Response {
    let Some(parent_catalog_id) = catalog_id(&parent) else {
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
    respond_policy(
        state
            .sub_rooms
            .sub_room_policy(InspectSubRoomPolicy {
                actor,
                parent_catalog_id,
            })
            .await,
        correlation_id,
    )
}

/// 房主改“谁能建子房间”这几项。
pub(super) async fn change_policy(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(parent): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<SubRoomPolicyBody>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return invalid("sub_room.invalid_policy_body", correlation_id);
    };
    let (actor, parent_catalog_id) = match required_write_context(
        &state,
        &headers,
        &jar,
        &parent,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    respond_policy(
        state
            .sub_rooms
            .change_sub_room_policy(ChangeSubRoomPolicy {
                actor,
                parent_catalog_id,
                policy: SubRoomPolicy {
                    creators: match body.creators {
                        CreatorsValue::Members => SubRoomCreators::Members,
                        CreatorsValue::Admins => SubRoomCreators::Admins,
                    },
                    agents_may_create: body.agents_may_create,
                    auto_archive: body.auto_archive,
                },
            })
            .await,
        correlation_id,
    )
}

fn respond_policy(
    result: agent_room_application::private_rooms::PrivateRoomResult<SubRoomPolicyView>,
    correlation_id: CorrelationId,
) -> Response {
    match result {
        Ok(view) => no_store(Json(SubRoomPolicyResponse::from(view)).into_response()),
        Err(failure) => no_store(ApiError::private_room(failure, correlation_id).into_response()),
    }
}
