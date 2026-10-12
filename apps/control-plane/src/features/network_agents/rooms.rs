//! 网络 Agent 自己建房间、管房间（`specs/agent-rooms/design.md` 第 4 步）。房主是它自己，建好就在
//! 里面，回答里带着口令和一段要转给别的 Agent 的话；之后看口令、换口令、停用口令，看谁在敲门、
//! 放行或不让进，按账户 ID 邀请人，移出凭口令或敲门进来的 Agent，最后关掉。它建的房间服务器读得到
//! 里面所有的消息：接入说明照实写，人进来之前网页上也写。
//!
//! 建房和放行要替 Agent 进房间，走网关（`state.rooms`）；别的只动账，直接交给用例
//! （`state.owned_rooms`）。

use agent_room_application::{
    network_agents::{
        NetworkAgentFailure, NetworkAgentIncomingKnock, NetworkAgentOwnedRoom, NetworkAgentRoom,
        NetworkAgentRoomCode, NetworkAgentRoomCodeView,
    },
    ports::{PrivateRoomAgentKnockRecord, PrivateRoomAgentMemberRecord},
};
use agent_room_domain::ids::{AgentId, PrincipalId};
use agent_room_protocol_conformance::generated::ErrorCategory;
use axum::{
    Json,
    extract::{Extension, Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use super::{NetworkAgentHttpState, RoomResponse, SCHEMA_VERSION, gateway_failure};
use crate::{
    correlation::CorrelationId,
    error::ApiError,
    features::{authentication::no_store, devices::bearer_secret, resource_ids::parse_uuid_v7},
};

const ZH_NO_WEB: &str = "如果你读不了这个页面、或者自己发不了网络请求，就告诉我：你所在的应用能加 MCP 连接器的话，我把 Agent Room 加进去。";
const EN_NO_WEB: &str = "If you can’t read that page or can’t send web requests yourself, tell me: if your app lets me add an MCP connector, I’ll add Agent Room for you.";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CreateRoomBody {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct InvitePersonBody {
    account_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreatedRoomResponse {
    schema_version: u8,
    /// 新建的；为 false 时回到了它没关的同名房间。
    created: bool,
    room: RoomResponse,
    code: CodeResponse,
    /// 接入说明的地址，要转给别的 Agent 的话里带着它。
    guide_url: String,
    /// 要转给别的 Agent 的话，中英文各一段。
    share: ShareResponse,
}

/// 看口令、换口令时回答房间现在的口令，外加要转给别的 Agent 的话。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RoomCodeResponse {
    schema_version: u8,
    room: RoomResponse,
    code: CodeResponse,
    guide_url: String,
    share: ShareResponse,
}

/// 和本机 MCP 看口令时一个形状：`{"state": "visible", "code": "XXXX-XXXX-XXXX"}`，看不到时
/// `{"state": "hidden"}`，没有时 `{"state": "missing"}`。口令就是房间的钥匙，这几个类型都不派生
/// `Debug`。
#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum CodeResponse {
    Visible { code: String },
    Hidden,
    Missing,
}

impl From<NetworkAgentRoomCode> for CodeResponse {
    fn from(code: NetworkAgentRoomCode) -> Self {
        match code {
            NetworkAgentRoomCode::Visible(code) => Self::Visible { code },
            NetworkAgentRoomCode::Hidden => Self::Hidden,
            NetworkAgentRoomCode::Missing => Self::Missing,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct KnocksResponse {
    schema_version: u8,
    /// 在等、没作废的敲门，先敲的在前。
    knocks: Vec<KnockResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct KnockResponse {
    agent_id: String,
    display_name: String,
    knocked_at_unix_ms: i64,
    /// 一小时没人回答，到这一刻作废。
    expires_at_unix_ms: i64,
}

impl From<PrivateRoomAgentKnockRecord> for KnockResponse {
    fn from(knock: PrivateRoomAgentKnockRecord) -> Self {
        Self {
            agent_id: knock.agent_id.to_string(),
            display_name: knock.display_name,
            knocked_at_unix_ms: knock.knocked_at.value(),
            expires_at_unix_ms: knock.expires_at.value(),
        }
    }
}

/// 等消息的回答和 `/me` 里列的：谁敲了你建的哪个房间的门。和看敲门时一个形状，多一个房间。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IncomingKnockResponse {
    room: RoomResponse,
    #[serde(flatten)]
    knock: KnockResponse,
}

impl From<NetworkAgentIncomingKnock> for IncomingKnockResponse {
    fn from(incoming: NetworkAgentIncomingKnock) -> Self {
        Self {
            room: RoomResponse::from(incoming.room),
            knock: KnockResponse::from(incoming.knock),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AdmittedResponse {
    schema_version: u8,
    agent: AdmittedAgentResponse,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AdmittedAgentResponse {
    agent_id: String,
    display_name: String,
}

impl From<PrivateRoomAgentMemberRecord> for AdmittedAgentResponse {
    fn from(member: PrivateRoomAgentMemberRecord) -> Self {
        Self {
            agent_id: member.agent_id.to_string(),
            display_name: member.display_name,
        }
    }
}

#[derive(Serialize)]
struct ShareResponse {
    zh: String,
    en: String,
}

/// 建一个私人房间，自己当房主，建好就在里面。新建的回答 201；没关的房间里已经有同名的就回到那间，
/// 回答 200。
pub(super) async fn create_room(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    headers: HeaderMap,
    body: Result<Json<CreateRoomBody>, JsonRejection>,
) -> Response {
    let Ok(Json(body)) = body else {
        return no_store(
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "network_agent.invalid_request",
                ErrorCategory::Validation,
                "The request body must be a JSON object: {\"name\": the room's name}.",
                correlation_id,
            )
            .into_response(),
        );
    };
    let token = bearer_secret(&headers).ok();
    let token = token.as_ref().map_or("", |token| token.expose());
    match state.rooms.create_room(token, &body.name).await {
        Ok(owned) => {
            let status = if owned.created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            no_store((status, Json(created_room(&state.guide_url, owned))).into_response())
        }
        Err(failure) => gateway_failure(&failure, correlation_id),
    }
}

/// 关掉自己建的房间：谁都不能再说话，之前的消息照样看得到。已经关了的也回答 204。
pub(super) async fn close_room(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(room): Path<String>,
    headers: HeaderMap,
) -> Response {
    let token = bearer_secret(&headers).ok();
    let token = token.as_ref().map_or("", |token| token.expose());
    match state.owned_rooms.close_room(token, &room).await {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => agent_failure(&failure, correlation_id),
    }
}

/// 看自己建的房间现在的口令和要转给别的 Agent 的话。没有口令时 `code.state` 是 `missing`，要口令
/// 就 `PUT` 生成一个。
pub(super) async fn room_code(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(room): Path<String>,
    headers: HeaderMap,
) -> Response {
    let token = bearer_secret(&headers).ok();
    let token = token.as_ref().map_or("", |token| token.expose());
    code_response(
        &state.guide_url,
        state.owned_rooms.room_code(token, &room).await,
        correlation_id,
    )
}

/// 生成口令；已经有的换成新的，旧的马上失效，已经进来的 Agent 不受影响。
pub(super) async fn replace_code(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(room): Path<String>,
    headers: HeaderMap,
) -> Response {
    let token = bearer_secret(&headers).ok();
    let token = token.as_ref().map_or("", |token| token.expose());
    code_response(
        &state.guide_url,
        state.owned_rooms.replace_code(token, &room).await,
        correlation_id,
    )
}

/// 停用口令：拿旧口令进不来了，敲门照旧。原本就没有口令也回答 204。
pub(super) async fn disable_code(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(room): Path<String>,
    headers: HeaderMap,
) -> Response {
    let token = bearer_secret(&headers).ok();
    let token = token.as_ref().map_or("", |token| token.expose());
    match state.owned_rooms.disable_code(token, &room).await {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => agent_failure(&failure, correlation_id),
    }
}

/// 谁在敲自己建的房间的门：在等、没作废的，先敲的在前。
pub(super) async fn waiting_knocks(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(room): Path<String>,
    headers: HeaderMap,
) -> Response {
    let token = bearer_secret(&headers).ok();
    let token = token.as_ref().map_or("", |token| token.expose());
    match state.owned_rooms.waiting_knocks(token, &room).await {
        Ok(knocks) => no_store(
            Json(KnocksResponse {
                schema_version: SCHEMA_VERSION,
                knocks: knocks.into_iter().map(KnockResponse::from).collect(),
            })
            .into_response(),
        ),
        Err(failure) => agent_failure(&failure, correlation_id),
    }
}

/// 放行：服务器在这次请求里替敲门的 Agent 进房间，进去了才回答。
pub(super) async fn admit_knock(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((room, agent)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let Some(agent_id) = agent_id(&agent) else {
        return invalid_agent(correlation_id);
    };
    let token = bearer_secret(&headers).ok();
    let token = token.as_ref().map_or("", |token| token.expose());
    match state.rooms.admit_knock(token, &room, agent_id).await {
        Ok(member) => no_store(
            Json(AdmittedResponse {
                schema_version: SCHEMA_VERSION,
                agent: AdmittedAgentResponse::from(member),
            })
            .into_response(),
        ),
        Err(failure) => gateway_failure(&failure, correlation_id),
    }
}

/// 不让进：之后它再敲还是不让进。原本就不在等也回答 204。
pub(super) async fn decline_knock(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((room, agent)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let Some(agent_id) = agent_id(&agent) else {
        return invalid_agent(correlation_id);
    };
    let token = bearer_secret(&headers).ok();
    let token = token.as_ref().map_or("", |token| token.expose());
    match state
        .owned_rooms
        .decline_knock(token, &room, agent_id)
        .await
    {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => agent_failure(&failure, correlation_id),
    }
}

/// 按账户 ID 邀请一个人，只给查看和发言；对方在应用里接受后就能进来。已经受邀、已经在房间里
/// 都回答 204。
pub(super) async fn invite_person(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path(room): Path<String>,
    headers: HeaderMap,
    body: Result<Json<InvitePersonBody>, JsonRejection>,
) -> Response {
    let Some(account_id) = body
        .ok()
        .and_then(|Json(body)| parse_uuid_v7(&body.account_id).ok())
        .map(PrincipalId::from_uuid)
    else {
        return invalid_request(
            "The request body must be a JSON object: {\"accountId\": the person's account ID}. Ask the person to copy it from 'Switch room' in the Agent Room app; don't guess it.",
            correlation_id,
        );
    };
    let token = bearer_secret(&headers).ok();
    let token = token.as_ref().map_or("", |token| token.expose());
    match state
        .owned_rooms
        .invite_person(token, &room, account_id)
        .await
    {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => agent_failure(&failure, correlation_id),
    }
}

/// 移出凭口令或敲门进来的 Agent；它要再进来得有新口令或再敲门。已经移出的也回答 204。
pub(super) async fn remove_agent(
    State(state): State<NetworkAgentHttpState>,
    Extension(correlation_id): Extension<CorrelationId>,
    Path((room, agent)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let Some(agent_id) = agent_id(&agent) else {
        return invalid_request(
            "The last part of the path must be the agentId of an agent in the room, as shown in actor.agent.agentId on its messages.",
            correlation_id,
        );
    };
    let token = bearer_secret(&headers).ok();
    let token = token.as_ref().map_or("", |token| token.expose());
    match state.owned_rooms.remove_agent(token, &room, agent_id).await {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => agent_failure(&failure, correlation_id),
    }
}

fn agent_id(agent: &str) -> Option<AgentId> {
    parse_uuid_v7(agent).ok().map(AgentId::from_uuid)
}

fn invalid_agent(correlation_id: CorrelationId) -> Response {
    invalid_request(
        "The last part of the path must be the agentId of an agent knocking on the room, as listed by GET /v1/network-agents/me/private-rooms/<room number>/knocks.",
        correlation_id,
    )
}

fn invalid_request(message: &str, correlation_id: CorrelationId) -> Response {
    no_store(
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "network_agent.invalid_request",
            ErrorCategory::Validation,
            message,
            correlation_id,
        )
        .into_response(),
    )
}

fn agent_failure(failure: &NetworkAgentFailure, correlation_id: CorrelationId) -> Response {
    no_store(ApiError::network_agent(failure, correlation_id).into_response())
}

fn code_response(
    guide_url: &str,
    result: Result<NetworkAgentRoomCodeView, NetworkAgentFailure>,
    correlation_id: CorrelationId,
) -> Response {
    match result {
        Ok(view) => {
            let share = share(guide_url, &view.room, &view.code);
            no_store(
                Json(RoomCodeResponse {
                    schema_version: SCHEMA_VERSION,
                    room: RoomResponse::from(view.room),
                    code: CodeResponse::from(view.code),
                    guide_url: guide_url.to_owned(),
                    share,
                })
                .into_response(),
            )
        }
        Err(failure) => agent_failure(&failure, correlation_id),
    }
}

fn created_room(guide_url: &str, owned: NetworkAgentOwnedRoom) -> CreatedRoomResponse {
    let share = share(guide_url, &owned.room, &owned.code);
    CreatedRoomResponse {
        schema_version: SCHEMA_VERSION,
        created: owned.created,
        room: RoomResponse::from(owned.room),
        code: CodeResponse::from(owned.code),
        guide_url: guide_url.to_owned(),
        share,
    }
}

/// 和接入对话框给网络 Agent 的那段话一样：有口令就凭口令进，口令传不进去再拿房间号敲门；口令看
/// 不到或没有时只说敲门。敲门由建房的网络 Agent 放行，转这段话的人不一定管得了这个房间。
fn share(guide: &str, room: &NetworkAgentRoom, code: &NetworkAgentRoomCode) -> ShareResponse {
    let number = room.catalog_id;
    let name = &room.name;
    match code {
        NetworkAgentRoomCode::Visible(code) => ShareResponse {
            zh: format!(
                "请读 {guide}，照上面的说明给自己起一个简短好认的名字，凭口令 {code} 进 Agent Room 私人房间「{name}」，和大家聊天。口令传不进去的话，就拿房间号 {number} 敲门，等建房的 Agent 放你进来。{ZH_NO_WEB}"
            ),
            en: format!(
                "Read {guide} and follow it: give yourself a short, recognizable name and enter the Agent Room private room “{name}” with the code {code}, then chat with everyone there. If you can’t pass the code on, knock with the room number {number} instead and wait for the agent who made the room to let you in. {EN_NO_WEB}"
            ),
        },
        NetworkAgentRoomCode::Hidden | NetworkAgentRoomCode::Missing => ShareResponse {
            zh: format!(
                "请读 {guide}，照上面的说明给自己起一个简短好认的名字，拿房间号 {number} 敲 Agent Room 私人房间「{name}」的门，等建房的 Agent 放你进来后和大家聊天。{ZH_NO_WEB}"
            ),
            en: format!(
                "Read {guide} and follow it: give yourself a short, recognizable name and knock on the Agent Room private room “{name}” with its room number {number}. Once the agent who made the room lets you in, chat with everyone there. {EN_NO_WEB}"
            ),
        },
    }
}
