//! 本机 Bridge 替这台电脑的账号建私人房间、邀请人（`specs/agent-rooms/design.md` 第 3 步）：
//! 设备签名。建房要说是哪个 Agent 建的，房主照旧是这个账号；邀请的人能查看、发言和自动发言。
//! Agent 只管 Agent 建的房间，由用例把关。
//!
//! 带上 `parentRoomId` 建的是那间房下面的子房间（`specs/redesign/sub-rooms.md` 第 2g-1 步），还能带
//! 门（`access`）和分类的名字（`category`）；规矩和人在网页上建一样，由子房间的用例把关。

use agent_room_application::private_rooms::{
    AgentSubRoomCreated, CreateAgentSubRoom, CreatePrivateRoom, InvitePrivateRoomMember,
    PrivateRoomActor,
};
use agent_room_domain::{
    ids::{AgentId, RoomCatalogId},
    private_rooms::PrivateRoomPermissions,
    room_listing::RoomJoinMode,
    sub_room_categories::validate_sub_room_category_name,
};
use axum::{
    Json,
    body::Bytes,
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use super::{
    PrivateRoomHttpState,
    handlers::{IDEMPOTENCY_KEY_HEADER, invalid, invalid_resource},
    models::{catalog_id, principal_id},
    sub_rooms::AccessValue,
};
use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{
        authentication::no_store, devices::authenticate_signed_device_request,
        resource_ids::parse_uuid_v7,
    },
};

const CREATE_TARGET: &str = "/devices/current/private-rooms";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateRoomBody {
    name: String,
    /// 建房的 Agent：这个账号名下、还在用的。
    agent_id: String,
    /// 建子房间时，在哪间房下面。不带是建一间整的房间。
    #[serde(default)]
    parent_room_id: Option<String>,
    /// 子房间的门，不带是没有门。只有建子房间时能带。
    #[serde(default)]
    access: Option<AccessValue>,
    /// 子房间放进哪个分类（名字）。只有建子房间时能带。
    #[serde(default)]
    category: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InvitePersonBody {
    account_id: String,
}

/// Bridge 只要这几样：建好以后按房间 ID 进去，和凭口令进房间的回答一个样。子房间另带它在哪间房下面、
/// 放进了哪个分类；建整间房时不写这几项，已经装着的 Bridge 认的还是原来那三项。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CreatedRoomResponse {
    catalog_id: String,
    matrix_room_id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<ParentRoomResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<String>,
    /// 要了分类，这间房里却没有叫这个名字的：放进了“未分类”。
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    category_missing: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ParentRoomResponse {
    catalog_id: String,
    name: String,
}

impl From<AgentSubRoomCreated> for CreatedRoomResponse {
    fn from(created: AgentSubRoomCreated) -> Self {
        Self {
            catalog_id: created.room.catalog().id().to_string(),
            matrix_room_id: created.room.instance().matrix_room_id().as_str().to_owned(),
            name: created.room.catalog().name().to_owned(),
            parent: created
                .room
                .parent_catalog_id()
                .map(|parent| ParentRoomResponse {
                    catalog_id: parent.to_string(),
                    name: created.parent_name,
                }),
            category: created.category,
            category_missing: created.category_missing,
        }
    }
}

/// 建子房间要的：在哪间房下面、门、分类的名字。门和分类只有建子房间时能带；分类的名字照分类的
/// 规矩（不空、两头不带空白、不太长）。
struct SubRoomChoice {
    parent: RoomCatalogId,
    access: RoomJoinMode,
    category: Option<String>,
}

fn sub_room_choice(body: &mut CreateRoomBody) -> Result<Option<SubRoomChoice>, ()> {
    let Some(parent) = body.parent_room_id.take() else {
        return if body.access.is_none() && body.category.is_none() {
            Ok(None)
        } else {
            Err(())
        };
    };
    let parent = catalog_id(&parent).ok_or(())?;
    let category = body.category.take();
    if category
        .as_deref()
        .is_some_and(|name| validate_sub_room_category_name(name).is_err())
    {
        return Err(());
    }
    Ok(Some(SubRoomChoice {
        parent,
        access: body.access.take().map_or(RoomJoinMode::Open, Into::into),
        category,
    }))
}

/// 建房。`Idempotency-Key` 是 Bridge 生成的房间 ID（UUIDv7）：同一个 ID 重试拿回同一个房间，
/// 换了名字或建房的 Agent 算冲突。
pub(super) async fn create(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(catalog_id) = headers
        .get(IDEMPOTENCY_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(catalog_id)
    else {
        return invalid("private_room.invalid_idempotency_key", correlation_id);
    };
    let actor = match signed_actor(
        &state,
        &headers,
        "POST",
        CREATE_TARGET,
        &body,
        correlation_id,
    )
    .await
    {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Ok(mut body) = serde_json::from_slice::<CreateRoomBody>(&body) else {
        return invalid("private_room.invalid_creation_body", correlation_id);
    };
    let Ok(agent_id) = parse_uuid_v7(&body.agent_id).map(AgentId::from_uuid) else {
        return invalid("private_room.invalid_creation_body", correlation_id);
    };
    let Ok(sub_room) = sub_room_choice(&mut body) else {
        return invalid("private_room.invalid_creation_body", correlation_id);
    };
    if let Some(choice) = sub_room {
        let created = state
            .sub_rooms
            .create_agent_sub_room(CreateAgentSubRoom {
                actor,
                parent_catalog_id: choice.parent,
                catalog_id,
                name: body.name,
                access: choice.access,
                category: choice.category,
                created_by_agent: agent_id,
            })
            .await;
        return match created {
            Ok(created) => no_store(
                (
                    StatusCode::CREATED,
                    Json(CreatedRoomResponse::from(created)),
                )
                    .into_response(),
            ),
            Err(failure) => {
                no_store(ApiError::private_room(failure, correlation_id).into_response())
            }
        };
    }
    let created = state
        .rooms
        .create(CreatePrivateRoom {
            actor,
            catalog_id,
            name: body.name,
            description: String::new(),
            retention_days: None,
            invitations: Vec::new(),
            created_by_agent: Some(agent_id),
            listing: None,
        })
        .await;
    match created {
        Ok(snapshot) => no_store(
            (
                StatusCode::CREATED,
                Json(CreatedRoomResponse {
                    catalog_id: snapshot.catalog().id().to_string(),
                    matrix_room_id: snapshot.instance().matrix_room_id().as_str().to_owned(),
                    name: snapshot.catalog().name().to_owned(),
                    parent: None,
                    category: None,
                    category_missing: false,
                }),
            )
                .into_response(),
        ),
        Err(failure) => no_store(ApiError::private_room(failure, correlation_id).into_response()),
    }
}

/// 按账户 ID 邀请一个人，给查看、发言和自动发言。已经受邀或在房间里也回答成功。
pub(super) async fn invite(
    State(state): State<PrivateRoomHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(catalog): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(catalog_id) = catalog_id(&catalog) else {
        return invalid_resource(correlation_id);
    };
    let target = format!("{CREATE_TARGET}/{catalog}/invitations");
    let actor = match signed_actor(&state, &headers, "POST", &target, &body, correlation_id).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    let Some(target_principal_id) = serde_json::from_slice::<InvitePersonBody>(&body)
        .ok()
        .and_then(|body| principal_id(&body.account_id))
    else {
        return invalid("private_room.invalid_invitation_body", correlation_id);
    };
    let invited = state
        .rooms
        .invite(InvitePrivateRoomMember {
            actor,
            catalog_id,
            target_principal_id,
            permissions: PrivateRoomPermissions::INVITED_BY_AGENT,
        })
        .await;
    match invited {
        Ok(_) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => no_store(ApiError::private_room(failure, correlation_id).into_response()),
    }
}

async fn signed_actor(
    state: &PrivateRoomHttpState,
    headers: &HeaderMap,
    method: &str,
    target: &str,
    body: &Bytes,
    correlation_id: CorrelationId,
) -> Result<PrivateRoomActor, Response> {
    let Ok(body_text) = std::str::from_utf8(body) else {
        return Err(invalid("private_room.invalid_body", correlation_id));
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
    Ok(PrivateRoomActor::Device(device))
}
