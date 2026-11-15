//! 子房间（`specs/redesign/sub-rooms.md`）：在一间房下面建子房间、读名单（2a），人自己加入、退出，
//! 改名字和用途（2b），门和“不在名单里显示”（2d-1），和一间房里子房间的三项设置
//! （`specs/redesign/room-roles.md` R4）。申请和批准（2d-2）在旁边的 `sub_room_requests`，分类（2e-1）
//! 在 `sub_room_categories`。
//!
//! 都是新的接口，不往私人房间的回答里加字段：网页对它 `.strict()` 校验，旧版桌面端多看到一个字段
//! 整个房间都读不出来。建好的子房间照私人房间的样子回。

use agent_room_application::{
    authentication::{AuthenticatedPrincipal, AuthenticationRequirement},
    private_rooms::{
        ChangeSubRoomPolicy, CreateSubRoom, InspectSubRoomPolicy, ListSubRooms, ReviseSubRoom,
        SubRoomList, SubRoomMembershipAction, SubRoomPolicyView, SubRoomView,
    },
};
use agent_room_domain::{
    ids::RoomCatalogId,
    private_rooms::PrivateRoomMembershipStatus,
    room_listing::RoomJoinMode,
    sub_rooms::{SubRoomCreators, SubRoomPolicy},
    time::UtcMillis,
};
use axum::{
    Json,
    extract::{
        Extension, Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
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
    sub_room_categories::{CategoryResponse, category_id, category_responses},
};
use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::authentication::{authenticate_session, no_store},
};

/// 门（“谁能进”），和公开房间的加入方式是同一套词：`open` 是没有门。
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AccessValue {
    Open,
    Request,
    InviteOnly,
}

impl From<AccessValue> for RoomJoinMode {
    fn from(access: AccessValue) -> Self {
        match access {
            AccessValue::Open => Self::Open,
            AccessValue::Request => Self::Request,
            AccessValue::InviteOnly => Self::InviteOnly,
        }
    }
}

/// 门、隐藏和分类可以不带：不带就是没有门、在名单里显示、“未分类”。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CreateSubRoomBody {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    access: Option<AccessValue>,
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    category_id: Option<String>,
}

/// 改名字、用途、门、隐藏、分类。没带的那一项不动；一项都没带就是没什么可改的。`categoryId`
/// 写成 `null` 是挪回“未分类”。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ReviseSubRoomBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    access: Option<AccessValue>,
    #[serde(default)]
    hidden: Option<bool>,
    #[serde(default)]
    category_id: CategoryField,
}

/// 改的请求里关于分类的那一项：没带（不动）、写成 `null`（挪回“未分类”）、写了分类号。
#[derive(Debug, Default)]
enum CategoryField {
    #[default]
    Absent,
    Cleared,
    Set(String),
}

impl<'de> Deserialize<'de> for CategoryField {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // 没带的那一项不会走到这里，由 `default` 给 `Absent`。
        Ok(Option::<String>::deserialize(deserializer)?.map_or(Self::Cleared, Self::Set))
    }
}

impl ReviseSubRoomBody {
    const fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.description.is_none()
            && self.access.is_none()
            && self.hidden.is_none()
            && matches!(self.category_id, CategoryField::Absent)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubRoomListResponse {
    parent: SubRoomParentResponse,
    /// 看的人自己现在能不能在这间房下面建子房间。
    can_create: bool,
    /// 这间房的分类，按先后（2e-1）。整间房共用；里面有几间由网页照下面的子房间数，看不到的
    /// 子房间不从这里露出来。
    categories: Vec<CategoryResponse>,
    /// 没归档的，按建的先后；带 `archived=1` 时只有归档的，最近归档的在前（2e-3）。
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
    /// 在哪个分类里；“未分类”的不写。
    #[serde(skip_serializing_if = "Option::is_none")]
    category_id: Option<String>,
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
    /// 看的人在等批准时，什么时候申请的。
    #[serde(skip_serializing_if = "Option::is_none")]
    requested_at_unix_ms: Option<i64>,
    /// 在等的申请有几个：只给管得了这一间的人。
    #[serde(skip_serializing_if = "Option::is_none")]
    pending_request_count: Option<u32>,
    /// 下面四项只在“已归档”的名单里写（2e-3）：什么时候、谁、为什么归档的，看的人能不能恢复。
    #[serde(skip_serializing_if = "Option::is_none")]
    archived_at_unix_ms: Option<i64>,
    /// 自动归档的、跟着父房间归档的没有。
    #[serde(skip_serializing_if = "Option::is_none")]
    archived_by_principal_id: Option<String>,
    /// `manual`（有人收起来的）、`inactive`（30 天没人说话）、`parent`（跟着父房间）。
    #[serde(skip_serializing_if = "Option::is_none")]
    archived_reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    can_restore: Option<bool>,
}

impl From<SubRoomView> for SubRoomResponse {
    fn from(view: SubRoomView) -> Self {
        let SubRoomView {
            entry,
            inside_visible,
            manages,
            restorable,
        } = view;
        let record = entry.archive_record.filter(|_| entry.archived);
        Self {
            catalog_id: entry.catalog_id.to_string(),
            name: entry.name,
            description: entry.description,
            access: entry.access.as_str(),
            hidden: entry.hidden,
            category_id: entry.category_id.map(|category| category.to_string()),
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
            requested_at_unix_ms: entry.viewer_requested_at.map(UtcMillis::value),
            pending_request_count: manages.then_some(entry.pending_requests),
            archived_at_unix_ms: record.map(|record| record.at.value()),
            archived_by_principal_id: record.and_then(|record| record.by).map(|by| by.to_string()),
            archived_reason: record.map(|record| record.reason.as_str()),
            can_restore: entry.archived.then_some(restorable),
        }
    }
}

/// 名单的查询参数：`archived=1` 要“已归档”的那份（2e-3），不带或者 `0` 是没归档的。
#[derive(Debug, Deserialize)]
pub(super) struct ListQuery {
    archived: Option<String>,
}

impl ListQuery {
    fn archived(&self) -> Option<bool> {
        match self.archived.as_deref() {
            None | Some("0") => Some(false),
            Some("1") => Some(true),
            Some(_) => None,
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
            categories: category_responses(list.categories),
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
    let category = match body.category_id.as_deref() {
        None => None,
        Some(value) => match category_id(value) {
            Some(category) => Some(category),
            None => return invalid("sub_room.invalid_creation_body", correlation_id),
        },
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
                access: body.access.map_or(RoomJoinMode::Open, RoomJoinMode::from),
                hidden: body.hidden,
                category_id: category,
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
    query: Result<Query<ListQuery>, QueryRejection>,
    jar: CookieJar,
) -> Response {
    let Some(catalog_id) = catalog_id(&catalog) else {
        return invalid_resource(correlation_id);
    };
    let Some(archived) = query.ok().and_then(|Query(query)| query.archived()) else {
        return invalid("private_room.invalid_request", correlation_id);
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
        .list_sub_rooms(ListSubRooms {
            actor,
            catalog_id,
            archived,
        })
        .await
    {
        Ok(list) => no_store(Json(SubRoomListResponse::from(list)).into_response()),
        Err(failure) => no_store(ApiError::private_room(failure, correlation_id).into_response()),
    }
}

/// 人自己进一间子房间。回的是照私人房间样子的这一间；网页接着照“服务器记着已加入、Matrix 上是
/// 受邀”那一套替他加入。已经在里面的再来一次也回这一间。
pub(super) async fn join(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((parent, sub_room)): Path<(String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let (actor, parent_catalog_id, sub_room_catalog_id) =
        match sub_room_write_context(&state, &headers, &jar, &parent, &sub_room, correlation_id)
            .await
        {
            Ok(context) => context,
            Err(response) => return response,
        };
    respond(
        state
            .sub_rooms
            .join_sub_room(SubRoomMembershipAction {
                actor,
                parent_catalog_id,
                catalog_id: sub_room_catalog_id,
            })
            .await,
        StatusCode::OK,
        correlation_id,
    )
}

/// 人从一间子房间退出。只要这一个请求：Matrix 那边由控制面替他离开。
pub(super) async fn leave(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((parent, sub_room)): Path<(String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let (actor, parent_catalog_id, sub_room_catalog_id) =
        match sub_room_write_context(&state, &headers, &jar, &parent, &sub_room, correlation_id)
            .await
        {
            Ok(context) => context,
            Err(response) => return response,
        };
    match state
        .sub_rooms
        .leave_sub_room(SubRoomMembershipAction {
            actor,
            parent_catalog_id,
            catalog_id: sub_room_catalog_id,
        })
        .await
    {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => no_store(ApiError::private_room(failure, correlation_id).into_response()),
    }
}

/// 改子房间的名字、用途、门和“不在名单里显示”。
pub(super) async fn revise(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((parent, sub_room)): Path<(String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<ReviseSubRoomBody>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return invalid("sub_room.invalid_revision_body", correlation_id);
    };
    if body.is_empty() {
        return invalid("sub_room.invalid_revision_body", correlation_id);
    }
    let category = match &body.category_id {
        CategoryField::Absent => None,
        CategoryField::Cleared => Some(None),
        CategoryField::Set(value) => match category_id(value) {
            Some(category) => Some(Some(category)),
            None => return invalid("sub_room.invalid_revision_body", correlation_id),
        },
    };
    let (actor, parent_catalog_id, sub_room_catalog_id) =
        match sub_room_write_context(&state, &headers, &jar, &parent, &sub_room, correlation_id)
            .await
        {
            Ok(context) => context,
            Err(response) => return response,
        };
    respond(
        state
            .sub_rooms
            .revise_sub_room(ReviseSubRoom {
                actor,
                parent_catalog_id,
                catalog_id: sub_room_catalog_id,
                // 两头的空白多半是手滑带上的，不为它拒绝。
                name: body.name.map(|name| name.trim().to_owned()),
                description: body.description,
                access: body.access.map(RoomJoinMode::from),
                hidden: body.hidden,
                category,
            })
            .await,
        StatusCode::OK,
        correlation_id,
    )
}

/// 动一间子房间的请求：同源、有效登录，地址里的两个房间号都得像样。
pub(super) async fn sub_room_write_context(
    state: &PrivateRoomHttpState,
    headers: &HeaderMap,
    jar: &CookieJar,
    parent: &str,
    sub_room: &str,
    correlation_id: CorrelationId,
) -> Result<(AuthenticatedPrincipal, RoomCatalogId, RoomCatalogId), Response> {
    let (actor, parent_catalog_id) = required_write_context(
        state,
        headers,
        jar,
        parent,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await?;
    let Some(sub_room_catalog_id) = catalog_id(sub_room) else {
        return Err(invalid_resource(correlation_id));
    };
    Ok((actor, parent_catalog_id, sub_room_catalog_id))
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
