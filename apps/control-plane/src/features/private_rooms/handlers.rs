use agent_room_application::{
    authentication::{AuthenticatedPrincipal, AuthenticationRequirement},
    private_rooms::{
        ArchivePrivateRoom, ChangePrivateRoomPermissions, GovernPrivateRoomMember,
        InspectPrivateRoom, InvitePrivateRoomMember, PrivateRoomMembershipAction,
        TransferPrivateRoomOwnership,
    },
};
use agent_room_domain::ids::RoomCatalogId;
use axum::{
    Json,
    extract::{Extension, Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;

use super::{
    PrivateRoomHttpState,
    models::{
        CreatePrivateRoomBody, InviteMemberBody, PermissionsBody, PrivateRoomResponse,
        TransferOwnershipBody, catalog_id, principal_id,
    },
};
use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::authentication::{authenticate_session, no_store, origin_matches},
};

const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";

pub(super) async fn create(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<CreatePrivateRoomBody>, JsonRejection>,
) -> Response {
    let actor = match authenticate_write(
        &state,
        &headers,
        &jar,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Some(catalog_id) = headers
        .get(IDEMPOTENCY_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(catalog_id)
    else {
        return invalid("private_room.invalid_idempotency_key", correlation_id);
    };
    let Ok(Json(body)) = body else {
        return invalid("private_room.invalid_creation_body", correlation_id);
    };
    let Some(request) = body.into_request(actor, catalog_id) else {
        return invalid("private_room.invalid_creation_body", correlation_id);
    };
    respond(
        state.rooms.create(request).await,
        StatusCode::CREATED,
        correlation_id,
    )
}

pub(super) async fn inspect(
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
    respond(
        state
            .rooms
            .inspect(InspectPrivateRoom { actor, catalog_id })
            .await,
        StatusCode::OK,
        correlation_id,
    )
}

pub(super) async fn invite(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<InviteMemberBody>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return invalid("private_room.invalid_invitation_body", correlation_id);
    };
    let Some(target_principal_id) = principal_id(&body.target_principal_id) else {
        return invalid("private_room.invalid_invitation_body", correlation_id);
    };
    let Some(permissions) = body.permissions.into_domain() else {
        return invalid("private_room.invalid_permissions", correlation_id);
    };
    let (actor, catalog_id) = match required_write_context(
        &state,
        &headers,
        &jar,
        &catalog,
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
            .rooms
            .invite(InvitePrivateRoomMember {
                actor,
                catalog_id,
                target_principal_id,
                permissions,
            })
            .await,
        StatusCode::OK,
        correlation_id,
    )
}

pub(super) async fn accept(
    state: State<PrivateRoomHttpState>,
    correlation: Extension<CorrelationId>,
    path: Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    membership_action(
        state,
        correlation,
        path,
        headers,
        jar,
        MembershipAction::Accept,
    )
    .await
}

pub(super) async fn decline(
    state: State<PrivateRoomHttpState>,
    correlation: Extension<CorrelationId>,
    path: Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    membership_action(
        state,
        correlation,
        path,
        headers,
        jar,
        MembershipAction::Decline,
    )
    .await
}

pub(super) async fn leave_room(
    state: State<PrivateRoomHttpState>,
    correlation: Extension<CorrelationId>,
    path: Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    membership_action(
        state,
        correlation,
        path,
        headers,
        jar,
        MembershipAction::Leave,
    )
    .await
}

pub(super) async fn remove(
    state: State<PrivateRoomHttpState>,
    correlation: Extension<CorrelationId>,
    path: Path<(String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    governance_action(
        state,
        correlation,
        path,
        headers,
        jar,
        GovernanceAction::Remove,
    )
    .await
}

pub(super) async fn ban(
    state: State<PrivateRoomHttpState>,
    correlation: Extension<CorrelationId>,
    path: Path<(String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    governance_action(
        state,
        correlation,
        path,
        headers,
        jar,
        GovernanceAction::Ban,
    )
    .await
}

pub(super) async fn update_permissions(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((catalog, target)): Path<(String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<PermissionsBody>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return invalid("private_room.invalid_permissions", correlation_id);
    };
    let Some(target_principal_id) = principal_id(&target) else {
        return invalid_resource(correlation_id);
    };
    let Some(permissions) = body.into_domain() else {
        return invalid("private_room.invalid_permissions", correlation_id);
    };
    let (actor, catalog_id) = match required_write_context(
        &state,
        &headers,
        &jar,
        &catalog,
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
            .rooms
            .update_permissions(ChangePrivateRoomPermissions {
                actor,
                catalog_id,
                target_principal_id,
                permissions,
            })
            .await,
        StatusCode::OK,
        correlation_id,
    )
}

pub(super) async fn transfer_ownership(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    body: Result<Json<TransferOwnershipBody>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return invalid("private_room.invalid_transfer_body", correlation_id);
    };
    let Some(target_principal_id) = principal_id(&body.target_principal_id) else {
        return invalid("private_room.invalid_transfer_body", correlation_id);
    };
    let Some(former_owner_permissions) = body.former_owner_permissions.into_domain() else {
        return invalid("private_room.invalid_permissions", correlation_id);
    };
    let (actor, catalog_id) = match required_write_context(
        &state,
        &headers,
        &jar,
        &catalog,
        AuthenticationRequirement::RecentAuthentication,
        correlation_id,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    respond(
        state
            .rooms
            .transfer_ownership(TransferPrivateRoomOwnership {
                actor,
                catalog_id,
                target_principal_id,
                former_owner_permissions,
            })
            .await,
        StatusCode::OK,
        correlation_id,
    )
}

pub(super) async fn archive(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Response {
    let (actor, catalog_id) = match required_write_context(
        &state,
        &headers,
        &jar,
        &catalog,
        AuthenticationRequirement::RecentAuthentication,
        correlation_id,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    respond(
        state
            .rooms
            .archive(ArchivePrivateRoom { actor, catalog_id })
            .await,
        StatusCode::OK,
        correlation_id,
    )
}

#[derive(Clone, Copy)]
enum MembershipAction {
    Accept,
    Decline,
    Leave,
}

async fn membership_action(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
    action: MembershipAction,
) -> Response {
    let (actor, catalog_id) = match required_write_context(
        &state,
        &headers,
        &jar,
        &catalog,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    let request = PrivateRoomMembershipAction { actor, catalog_id };
    let result = match action {
        MembershipAction::Accept => state.rooms.accept(request).await,
        MembershipAction::Decline => state.rooms.decline(request).await,
        MembershipAction::Leave => state.rooms.leave(request).await,
    };
    respond(result, StatusCode::OK, correlation_id)
}

#[derive(Clone, Copy)]
enum GovernanceAction {
    Remove,
    Ban,
}

async fn governance_action(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((catalog, target)): Path<(String, String)>,
    headers: HeaderMap,
    jar: CookieJar,
    action: GovernanceAction,
) -> Response {
    let Some(target_principal_id) = principal_id(&target) else {
        return invalid_resource(correlation_id);
    };
    let (actor, catalog_id) = match required_write_context(
        &state,
        &headers,
        &jar,
        &catalog,
        AuthenticationRequirement::ActiveSession,
        correlation_id,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    let request = GovernPrivateRoomMember {
        actor,
        catalog_id,
        target_principal_id,
    };
    let result = match action {
        GovernanceAction::Remove => state.rooms.remove(request).await,
        GovernanceAction::Ban => state.rooms.ban(request).await,
    };
    respond(result, StatusCode::OK, correlation_id)
}

async fn required_write_context(
    state: &PrivateRoomHttpState,
    headers: &HeaderMap,
    jar: &CookieJar,
    catalog: &str,
    requirement: AuthenticationRequirement,
    correlation_id: CorrelationId,
) -> Result<(AuthenticatedPrincipal, RoomCatalogId), Response> {
    if !origin_matches(headers, &state.frontend_origin) {
        return Err(no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "private_room.invalid_origin",
                agent_room_protocol_conformance::generated::ErrorCategory::Authorization,
                "私人房间写请求来源无效。",
                correlation_id,
            )
            .into_response(),
        ));
    }
    let Some(catalog_id) = catalog_id(catalog) else {
        return Err(invalid_resource(correlation_id));
    };
    let actor = authenticate_session(
        state.authentication.as_ref(),
        jar,
        requirement,
        correlation_id,
    )
    .await?;
    Ok((actor, catalog_id))
}

async fn authenticate_write(
    state: &PrivateRoomHttpState,
    headers: &HeaderMap,
    jar: &CookieJar,
    requirement: AuthenticationRequirement,
    correlation_id: CorrelationId,
) -> Result<AuthenticatedPrincipal, Response> {
    if !origin_matches(headers, &state.frontend_origin) {
        return Err(no_store(
            ApiError::new(
                StatusCode::FORBIDDEN,
                "private_room.invalid_origin",
                agent_room_protocol_conformance::generated::ErrorCategory::Authorization,
                "私人房间写请求来源无效。",
                correlation_id,
            )
            .into_response(),
        ));
    }
    authenticate_session(
        state.authentication.as_ref(),
        jar,
        requirement,
        correlation_id,
    )
    .await
}

fn respond(
    result: agent_room_application::private_rooms::PrivateRoomResult<
        agent_room_application::ports::PrivateRoomSnapshot,
    >,
    success: StatusCode,
    correlation_id: CorrelationId,
) -> Response {
    match result {
        Ok(snapshot) => {
            no_store((success, Json(PrivateRoomResponse::from(snapshot))).into_response())
        }
        Err(failure) => no_store(ApiError::private_room(failure, correlation_id).into_response()),
    }
}

fn invalid(code: &'static str, correlation_id: CorrelationId) -> Response {
    no_store(ApiError::invalid_request(code, correlation_id).into_response())
}

fn invalid_resource(correlation_id: CorrelationId) -> Response {
    invalid("private_room.invalid_resource_id", correlation_id)
}
