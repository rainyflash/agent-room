//! 子房间的归档和恢复（`specs/redesign/sub-rooms.md` 第 2e-2 步）：
//!
//! - `DELETE /private-rooms/{父}/sub-rooms/{子}`：归档。这一间的管理员、父房间的房主和管理员能做；
//! - `POST /private-rooms/{父}/sub-rooms/{子}/restore`：恢复。能归档的人都能恢复，30 天没人说话
//!   自动归档的，原来在里面的人也能。
//!
//! 都回 204（动手的人不一定在这一间里，回整间房的样子不合适），归档过的再归档、没归档的再恢复也一样。
//! 只要有效登录：和归档整间房一样确认一下就做。

use agent_room_application::private_rooms::{ArchiveSubRoom, RestoreSubRoom};
use axum::{
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;

use super::{PrivateRoomHttpState, sub_rooms::sub_room_write_context};
use crate::{correlation::CorrelationId, error::ApiError, features::authentication::no_store};

pub(super) async fn archive(
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
    let result = state
        .sub_rooms
        .archive_sub_room(ArchiveSubRoom {
            actor,
            parent_catalog_id,
            catalog_id,
        })
        .await;
    match result {
        Ok(_) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => no_store(ApiError::private_room(failure, correlation_id).into_response()),
    }
}

pub(super) async fn restore(
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
    let result = state
        .sub_rooms
        .restore_sub_room(RestoreSubRoom {
            actor,
            parent_catalog_id,
            catalog_id,
        })
        .await;
    match result {
        Ok(_) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => no_store(ApiError::private_room(failure, correlation_id).into_response()),
    }
}
