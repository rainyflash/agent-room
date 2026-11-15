//! 子房间的请进来、移出和封禁（`specs/redesign/sub-rooms.md` 第 2d-3 步）。管得了这一间的人（这一间
//! 的管理员、父房间的房主和管理员，进没进来都一样）用：
//!
//! - `PUT /private-rooms/{父}/sub-rooms/{子}/members/{账户}`：把父房间里的人直接请进来，哪种门都行；
//! - `DELETE` 同一个地址：移出；
//! - `POST .../members/{账户}/ban`、`.../unban`：封禁、解封。
//!
//! 都回 204：动手的人不一定在这一间里，回整间房的样子不合适，网页要看就再读一次。

use agent_room_application::private_rooms::GovernSubRoomMember;
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

#[derive(Debug, Clone, Copy)]
enum MemberAction {
    Admit,
    Remove,
    Ban,
    Unban,
}

pub(super) async fn admit(
    state: State<PrivateRoomHttpState>,
    correlation_id: Extension<CorrelationId>,
    path: Path<(String, String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    member_action(
        state,
        correlation_id,
        path,
        headers,
        jar,
        MemberAction::Admit,
    )
    .await
}

pub(super) async fn remove(
    state: State<PrivateRoomHttpState>,
    correlation_id: Extension<CorrelationId>,
    path: Path<(String, String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    member_action(
        state,
        correlation_id,
        path,
        headers,
        jar,
        MemberAction::Remove,
    )
    .await
}

pub(super) async fn ban(
    state: State<PrivateRoomHttpState>,
    correlation_id: Extension<CorrelationId>,
    path: Path<(String, String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    member_action(state, correlation_id, path, headers, jar, MemberAction::Ban).await
}

pub(super) async fn unban(
    state: State<PrivateRoomHttpState>,
    correlation_id: Extension<CorrelationId>,
    path: Path<(String, String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    member_action(
        state,
        correlation_id,
        path,
        headers,
        jar,
        MemberAction::Unban,
    )
    .await
}

/// 和整间房的移出、封禁一样只要有效登录。
async fn member_action(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((parent, sub_room, principal)): Path<(String, String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
    action: MemberAction,
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
    let request = GovernSubRoomMember {
        actor,
        parent_catalog_id,
        catalog_id,
        target,
    };
    let result = match action {
        MemberAction::Admit => state.sub_rooms.admit_sub_room_member(request).await,
        MemberAction::Remove => state.sub_rooms.remove_sub_room_member(request).await,
        MemberAction::Ban => state.sub_rooms.ban_sub_room_member(request).await,
        MemberAction::Unban => state.sub_rooms.unban_sub_room_member(request).await,
    };
    match result {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => no_store(ApiError::private_room(failure, correlation_id).into_response()),
    }
}
