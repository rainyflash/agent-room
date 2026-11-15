//! 子房间的分类（`specs/redesign/sub-rooms.md` 的“分类”，第 2e-1 步）：
//!
//! - `POST /private-rooms/{父}/sub-room-categories`：建一个，排在最后。和建子房间一样带幂等键
//!   （`Idempotency-Key` 是分类号），同一个请求重发不多建；
//! - `PUT /private-rooms/{父}/sub-room-categories/{分类}`：改名（`name`）、挪到第几个（`position`，
//!   从 0 起）；
//! - `DELETE` 同一个地址：删掉，里面的子房间回到“未分类”。删过的再删也算成功。
//!
//! 都回这间房此刻所有的分类（`{"categories": [...]}`），按先后。房间的房主和管理员能做，不要最近
//! 登录过。分类的名单另在子房间的名单里（`GET .../sub-rooms` 的 `categories`）。

use agent_room_application::{
    authentication::AuthenticationRequirement,
    ports::SubRoomCategory,
    private_rooms::{
        CreateSubRoomCategory, DeleteSubRoomCategory, PrivateRoomResult, ReviseSubRoomCategory,
    },
};
use agent_room_domain::ids::SubRoomCategoryId;
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
    handlers::{IDEMPOTENCY_KEY_HEADER, invalid, invalid_resource, required_write_context},
};
use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{authentication::no_store, resource_ids::parse_uuid_v7},
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CreateCategoryBody {
    name: String,
}

/// 没带的那一项不动；一项都没带就是没什么可改的。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ReviseCategoryBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    position: Option<u8>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CategoriesResponse {
    categories: Vec<CategoryResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CategoryResponse {
    category_id: String,
    name: String,
    /// 排在第几个，从 0 起、连着。
    position: u8,
}

impl From<SubRoomCategory> for CategoryResponse {
    fn from(category: SubRoomCategory) -> Self {
        Self {
            category_id: category.id.to_string(),
            name: category.name,
            position: category.position,
        }
    }
}

pub(super) fn category_responses(categories: Vec<SubRoomCategory>) -> Vec<CategoryResponse> {
    categories.into_iter().map(CategoryResponse::from).collect()
}

fn category_id(value: &str) -> Option<SubRoomCategoryId> {
    parse_uuid_v7(value).map(SubRoomCategoryId::from_uuid).ok()
}

pub(super) async fn create(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(parent): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<CreateCategoryBody>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return invalid("sub_room.invalid_category_body", correlation_id);
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
    let Some(category_id) = headers
        .get(IDEMPOTENCY_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(category_id)
    else {
        return invalid("private_room.invalid_idempotency_key", correlation_id);
    };
    respond(
        state
            .sub_rooms
            .create_sub_room_category(CreateSubRoomCategory {
                actor,
                parent_catalog_id,
                category_id,
                name: body.name,
            })
            .await,
        StatusCode::CREATED,
        correlation_id,
    )
}

pub(super) async fn revise(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((parent, category)): Path<(String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<ReviseCategoryBody>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return invalid("sub_room.invalid_category_body", correlation_id);
    };
    if body.name.is_none() && body.position.is_none() {
        return invalid("sub_room.invalid_category_body", correlation_id);
    }
    let Some(category_id) = category_id(&category) else {
        return invalid_resource(correlation_id);
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
    respond(
        state
            .sub_rooms
            .revise_sub_room_category(ReviseSubRoomCategory {
                actor,
                parent_catalog_id,
                category_id,
                name: body.name,
                position: body.position,
            })
            .await,
        StatusCode::OK,
        correlation_id,
    )
}

pub(super) async fn delete(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((parent, category)): Path<(String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let Some(category_id) = category_id(&category) else {
        return invalid_resource(correlation_id);
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
    respond(
        state
            .sub_rooms
            .delete_sub_room_category(DeleteSubRoomCategory {
                actor,
                parent_catalog_id,
                category_id,
            })
            .await,
        StatusCode::OK,
        correlation_id,
    )
}

fn respond(
    result: PrivateRoomResult<Vec<SubRoomCategory>>,
    status: StatusCode,
    correlation_id: CorrelationId,
) -> Response {
    match result {
        Ok(categories) => no_store(
            (
                status,
                Json(CategoriesResponse {
                    categories: category_responses(categories),
                }),
            )
                .into_response(),
        ),
        Err(failure) => no_store(ApiError::private_room(failure, correlation_id).into_response()),
    }
}
