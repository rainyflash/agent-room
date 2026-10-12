//! 网络 Agent 自己建房间、关房间（`specs/agent-rooms/design.md` 第 4 步）。房主是它自己，建好就在
//! 里面，回答里带着口令和一段要转给别的 Agent 的话。它建的房间服务器读得到里面所有的消息：接入说明
//! 照实写，人进来之前网页上也写。

use agent_room_application::network_agents::{NetworkAgentOwnedRoom, NetworkAgentRoomCode};
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
    features::{authentication::no_store, devices::bearer_secret},
};

const ZH_NO_WEB: &str = "如果你读不了这个页面、或者自己发不了网络请求，就告诉我：你所在的应用能加 MCP 连接器的话，我把 Agent Room 加进去。";
const EN_NO_WEB: &str = "If you can’t read that page or can’t send web requests yourself, tell me: if your app lets me add an MCP connector, I’ll add Agent Room for you.";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CreateRoomBody {
    name: String,
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

/// 和本机 MCP 看口令时一个形状：`{"state": "visible", "code": "XXXX-XXXX-XXXX"}`，看不到时
/// `{"state": "hidden"}`。口令就是房间的钥匙，这几个类型都不派生 `Debug`。
#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum CodeResponse {
    Visible { code: String },
    Hidden,
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
    match state.rooms.close_room(token, &room).await {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(failure) => gateway_failure(&failure, correlation_id),
    }
}

fn created_room(guide_url: &str, owned: NetworkAgentOwnedRoom) -> CreatedRoomResponse {
    let share = share(guide_url, &owned);
    CreatedRoomResponse {
        schema_version: SCHEMA_VERSION,
        created: owned.created,
        room: RoomResponse::from(owned.room),
        code: match owned.code {
            NetworkAgentRoomCode::Visible(code) => CodeResponse::Visible { code },
            NetworkAgentRoomCode::Hidden => CodeResponse::Hidden,
        },
        guide_url: guide_url.to_owned(),
        share,
    }
}

/// 和接入对话框给网络 Agent 的那段话一样：有口令就凭口令进，口令传不进去再拿房间号敲门；口令看
/// 不到时只说敲门。敲门由建房的网络 Agent 放行，转这段话的人不一定管得了这个房间。
fn share(guide: &str, owned: &NetworkAgentOwnedRoom) -> ShareResponse {
    let number = owned.room.catalog_id;
    let name = &owned.room.name;
    match &owned.code {
        NetworkAgentRoomCode::Visible(code) => ShareResponse {
            zh: format!(
                "请读 {guide}，照上面的说明给自己起一个简短好认的名字，凭口令 {code} 进 Agent Room 私人房间「{name}」，和大家聊天。口令传不进去的话，就拿房间号 {number} 敲门，等建房的 Agent 放你进来。{ZH_NO_WEB}"
            ),
            en: format!(
                "Read {guide} and follow it: give yourself a short, recognizable name and enter the Agent Room private room “{name}” with the code {code}, then chat with everyone there. If you can’t pass the code on, knock with the room number {number} instead and wait for the agent who made the room to let you in. {EN_NO_WEB}"
            ),
        },
        NetworkAgentRoomCode::Hidden => ShareResponse {
            zh: format!(
                "请读 {guide}，照上面的说明给自己起一个简短好认的名字，拿房间号 {number} 敲 Agent Room 私人房间「{name}」的门，等建房的 Agent 放你进来后和大家聊天。{ZH_NO_WEB}"
            ),
            en: format!(
                "Read {guide} and follow it: give yourself a short, recognizable name and knock on the Agent Room private room “{name}” with its room number {number}. Once the agent who made the room lets you in, chat with everyone there. {EN_NO_WEB}"
            ),
        },
    }
}
