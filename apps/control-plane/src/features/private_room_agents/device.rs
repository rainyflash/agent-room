//! 本机 Bridge 替这台电脑的账号管 Agent 建的私人房间（`specs/agent-rooms/design.md` 第 3 步）：
//! 口令、敲门、移出 Agent。和网页上是同一组用例，只是换成设备签名；Agent 只管 Agent 建的房间，
//! 由用例把关。签名对着请求原样的方法、路径和正文，没有正文的签空串。

use agent_room_application::private_rooms::{
    AnswerKnock, InspectAgentAccess, ManageJoinCode, PrivateRoomActor, RemoveAgentMember,
};
use agent_room_domain::ids::{AgentId, RoomCatalogId};
use axum::{
    body::Bytes,
    extract::{Extension, Path, State},
    http::HeaderMap,
    response::Response,
};

use super::{
    PrivateRoomAgentHttpState, admit, code_response, declined, empty, invalid_body,
    invalid_resource, knocks_response,
};
use crate::{
    correlation::CorrelationId,
    features::{devices::authenticate_signed_device_request, resource_ids::parse_uuid_v7},
};

const ROOMS: &str = "/devices/current/private-rooms";

/// 看房间现在的口令；没有口令回答 `join_code.not_found`，由调用方再生成一个。
pub(super) async fn reveal_code(
    State(state): State<PrivateRoomAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let target = format!("{ROOMS}/{catalog}/code");
    let (actor, catalog_id) = match signed_room(
        &state,
        &headers,
        "GET",
        &target,
        &catalog,
        &body,
        correlation_id,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    code_response(
        state
            .access
            .reveal_code(ManageJoinCode { actor, catalog_id })
            .await,
        correlation_id,
    )
}

/// 生成口令；已经有的换成新的，旧的马上失效。
pub(super) async fn generate_code(
    State(state): State<PrivateRoomAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let target = format!("{ROOMS}/{catalog}/code");
    let (actor, catalog_id) = match signed_room(
        &state,
        &headers,
        "PUT",
        &target,
        &catalog,
        &body,
        correlation_id,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    code_response(
        state
            .access
            .generate_code(ManageJoinCode { actor, catalog_id })
            .await,
        correlation_id,
    )
}

pub(super) async fn disable_code(
    State(state): State<PrivateRoomAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let target = format!("{ROOMS}/{catalog}/code");
    let (actor, catalog_id) = match signed_room(
        &state,
        &headers,
        "DELETE",
        &target,
        &catalog,
        &body,
        correlation_id,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    empty(
        state
            .access
            .disable_code(ManageJoinCode { actor, catalog_id })
            .await,
        correlation_id,
    )
}

pub(super) async fn waiting_knocks(
    State(state): State<PrivateRoomAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let target = format!("{ROOMS}/{catalog}/knocks");
    let (actor, catalog_id) = match signed_room(
        &state,
        &headers,
        "GET",
        &target,
        &catalog,
        &body,
        correlation_id,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    knocks_response(
        state
            .access
            .waiting_knocks(InspectAgentAccess { actor, catalog_id })
            .await,
        correlation_id,
    )
}

/// 让在敲门的 Agent 进来，和网页上一样由网关替它进房间。
pub(super) async fn admit_knock(
    State(state): State<PrivateRoomAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((catalog, agent)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Ok(agent_id) = parse_uuid_v7(&agent).map(AgentId::from_uuid) else {
        return invalid_resource(correlation_id);
    };
    let target = format!("{ROOMS}/{catalog}/knocks/{agent}");
    let (actor, catalog_id) = match signed_room(
        &state,
        &headers,
        "PUT",
        &target,
        &catalog,
        &body,
        correlation_id,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    admit(
        &state,
        AnswerKnock {
            actor,
            catalog_id,
            agent_id,
        },
        correlation_id,
    )
    .await
}

pub(super) async fn decline_knock(
    State(state): State<PrivateRoomAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((catalog, agent)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Ok(agent_id) = parse_uuid_v7(&agent).map(AgentId::from_uuid) else {
        return invalid_resource(correlation_id);
    };
    let target = format!("{ROOMS}/{catalog}/knocks/{agent}");
    let (actor, catalog_id) = match signed_room(
        &state,
        &headers,
        "DELETE",
        &target,
        &catalog,
        &body,
        correlation_id,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    declined(
        state
            .access
            .decline_knock(AnswerKnock {
                actor,
                catalog_id,
                agent_id,
            })
            .await,
        correlation_id,
    )
}

/// 移出凭口令或敲门进来的 Agent。
pub(super) async fn remove_agent(
    State(state): State<PrivateRoomAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((catalog, agent)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Ok(agent_id) = parse_uuid_v7(&agent).map(AgentId::from_uuid) else {
        return invalid_resource(correlation_id);
    };
    let target = format!("{ROOMS}/{catalog}/agents/{agent}");
    let (actor, catalog_id) = match signed_room(
        &state,
        &headers,
        "DELETE",
        &target,
        &catalog,
        &body,
        correlation_id,
    )
    .await
    {
        Ok(context) => context,
        Err(response) => return response,
    };
    empty(
        state
            .access
            .remove_agent(RemoveAgentMember {
                actor,
                catalog_id,
                agent_id,
            })
            .await,
        correlation_id,
    )
}

/// 先认出房间，再核对设备签名。
async fn signed_room(
    state: &PrivateRoomAgentHttpState,
    headers: &HeaderMap,
    method: &str,
    target: &str,
    catalog: &str,
    body: &Bytes,
    correlation_id: CorrelationId,
) -> Result<(PrivateRoomActor, RoomCatalogId), Response> {
    let Ok(catalog_id) = parse_uuid_v7(catalog).map(RoomCatalogId::from_uuid) else {
        return Err(invalid_resource(correlation_id));
    };
    let Ok(body_text) = std::str::from_utf8(body) else {
        return Err(invalid_body(correlation_id));
    };
    let device = authenticate_signed_device_request(
        state.devices.as_ref(),
        state.secrets.as_ref(),
        headers,
        method,
        target,
        body_text,
        correlation_id,
    )
    .await?;
    Ok((PrivateRoomActor::Device(device), catalog_id))
}
