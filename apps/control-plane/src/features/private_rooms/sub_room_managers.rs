//! 子房间管理员（`specs/redesign/room-roles.md` 的 R5，`specs/redesign/sub-rooms.md` 第 2d-4 步）：
//!
//! - `PUT /private-rooms/{父}/sub-rooms/{子}/managers/{账户}`：设他当这一间的管理员。管得了这一间的人
//!   都能设，他得已经在这一间里；
//! - `DELETE` 同一个地址：撤掉。撤别人要父房间的房主或管理员，子房间管理员只能撤自己（不当了）。
//!
//! 都回 204，和请人进来、移出一样。要不要最近登录过由用例判断：自己不当了不用，别的要。

use agent_room_application::private_rooms::ChangeSubRoomManager;
use axum::{
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;

use super::{
    PrivateRoomHttpState, handlers::invalid_resource, models::principal_id,
    sub_rooms::sub_room_write_context,
};
use crate::{correlation::CorrelationId, error::ApiError, features::authentication::no_store};

pub(super) async fn grant(
    state: State<PrivateRoomHttpState>,
    correlation_id: Extension<CorrelationId>,
    path: Path<(String, String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    change(state, correlation_id, path, headers, jar, true).await
}

pub(super) async fn revoke(
    state: State<PrivateRoomHttpState>,
    correlation_id: Extension<CorrelationId>,
    path: Path<(String, String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    change(state, correlation_id, path, headers, jar, false).await
}

async fn change(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((parent, sub_room, principal)): Path<(String, String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
    grant: bool,
) -> Response {
    let Some(target) = principal_id(&principal) else {
        return invalid_resource(correlation_id);
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
        .change_sub_room_manager(ChangeSubRoomManager {
            actor,
            parent_catalog_id,
            catalog_id,
            target,
            grant,
        })
        .await;
    match result {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => no_store(ApiError::private_room(failure, correlation_id).into_response()),
    }
}
