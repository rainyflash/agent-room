//! Agent 自己建房间、管房间（`specs/agent-rooms/design.md`）：经控制面 `/devices/current/private-rooms`
//! 下的设备签名路由建房、管口令和敲门、邀请人、移出 Agent。签名对着请求原样的方法、路径和正文，
//! 没有正文的签空串。

use std::sync::Arc;

use agent_room_application::ports::PortFuture;
use agent_room_bridge_core::{
    room_management::{
        ControlPlaneRoomManagementGateway, CreatedRoom, RoomCodeState, RoomJoinCode, RoomKnock,
        RoomManagementFailure, RoomManagementFailureKind, RoomManagementResult,
    },
    session::{BridgeSessionFailure, BridgeSessionFailureKind, ControlPlaneRequestAuthorizer},
};
use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId},
    rooms::MatrixRoomReference,
    time::UtcMillis,
};
use reqwest::{Client, Method, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use url::Url;

use super::{
    ControlPlaneHttpConfig, ControlPlaneHttpConfigurationError, IDEMPOTENCY_KEY_HEADER,
    configured_client, read_limited_response_body, signed_request_headers,
};

const ROOMS: &str = "/devices/current/private-rooms";
/// 口令给人看的形式 `XXXX-XXXX-XXXX`；回答里的口令长得不像就不交给调用方。
const MAX_CODE_CHARS: usize = 32;

pub struct ReqwestControlPlaneRoomManagementGateway {
    client: Client,
    base_url: Url,
    authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
}

/// 控制面没答应：要么请求没发出去（已经换成失败），要么拿回了错误码。
enum Refused {
    Failure(RoomManagementFailure),
    Response {
        status: StatusCode,
        code: Option<String>,
    },
}

impl Refused {
    fn into_failure(self) -> RoomManagementFailure {
        match self {
            Self::Failure(failure) => failure,
            Self::Response { status, code } => status_failure(status, code.as_deref()),
        }
    }
}

impl ReqwestControlPlaneRoomManagementGateway {
    /// 创建按设备签名建房、管房间的 HTTP 网关。
    ///
    /// # Errors
    ///
    /// 控制面地址、明文传输边界、超时或 HTTP 客户端配置无效时返回错误。
    pub fn new(
        config: &ControlPlaneHttpConfig,
        authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
    ) -> Result<Self, ControlPlaneHttpConfigurationError> {
        let (client, base_url) = configured_client(config)?;
        Ok(Self {
            client,
            base_url,
            authorizer,
        })
    }

    /// 发一个设备签名的请求，拿回成功时的正文（可能为空）。
    async fn send(
        &self,
        method: Method,
        target: &str,
        body: String,
        idempotency_key: Option<String>,
    ) -> Result<Vec<u8>, Refused> {
        let authorized = self
            .authorizer
            .authorize(method.as_str(), target, &body)
            .await
            .map_err(|error| Refused::Failure(map_session_failure(error)))?;
        let url = self
            .base_url
            .join(target.trim_start_matches('/'))
            .map_err(|_| Refused::Failure(failure(RoomManagementFailureKind::Internal)))?;
        let mut request = self.client.request(method.clone(), url);
        if !body.is_empty() {
            request = request.header(reqwest::header::CONTENT_TYPE, "application/json");
        }
        if let Some(key) = idempotency_key {
            request = request.header(IDEMPOTENCY_KEY_HEADER, key);
        }
        let request = signed_request_headers(request, &authorized, method.as_str(), target)
            .map_err(|()| Refused::Failure(failure(RoomManagementFailureKind::Internal)))?;
        let response = request.body(body).send().await.map_err(|_| {
            Refused::Failure(failure(RoomManagementFailureKind::ControlPlaneUnavailable))
        })?;
        let status = response.status();
        let bytes = read_limited_response_body(response).await.map_err(|()| {
            Refused::Failure(if status.is_success() {
                failure(RoomManagementFailureKind::InvalidControlPlaneResponse)
            } else {
                status_failure(status, None)
            })
        })?;
        if status.is_success() {
            return Ok(bytes);
        }
        let code = serde_json::from_slice::<ErrorCodeResponse>(&bytes)
            .ok()
            .map(|envelope| envelope.code);
        Err(Refused::Response { status, code })
    }

    async fn call(
        &self,
        method: Method,
        target: &str,
        body: String,
    ) -> RoomManagementResult<Vec<u8>> {
        self.send(method, target, body, None)
            .await
            .map_err(Refused::into_failure)
    }

    async fn create_room(
        &self,
        catalog_id: RoomCatalogId,
        name: &str,
        agent_id: AgentId,
    ) -> RoomManagementResult<CreatedRoom> {
        let body = serde_json::to_string(&CreateRoomBody {
            name,
            agent_id: agent_id.to_string(),
        })
        .map_err(|_| failure(RoomManagementFailureKind::Internal))?;
        // 房间 ID 当幂等键：结果没回来时带同一个 ID 重试，拿回的是同一个房间。
        let bytes = self
            .send(Method::POST, ROOMS, body, Some(catalog_id.to_string()))
            .await
            .map_err(Refused::into_failure)?;
        decode::<CreatedRoomResponse>(&bytes)?.try_into()
    }

    async fn current_code(&self, catalog_id: RoomCatalogId) -> RoomManagementResult<RoomCodeState> {
        let target = format!("{ROOMS}/{catalog_id}/code");
        match self.send(Method::GET, &target, String::new(), None).await {
            Ok(bytes) => decode::<JoinCodeResponse>(&bytes)?
                .try_into()
                .map(RoomCodeState::Visible),
            Err(Refused::Response { code, .. })
                if code.as_deref() == Some("join_code.not_found") =>
            {
                Ok(RoomCodeState::Missing)
            }
            Err(Refused::Response { code, .. })
                if code.as_deref() == Some("join_code.not_revealable") =>
            {
                Ok(RoomCodeState::Hidden)
            }
            Err(refused) => Err(refused.into_failure()),
        }
    }

    async fn new_code(&self, catalog_id: RoomCatalogId) -> RoomManagementResult<RoomJoinCode> {
        let target = format!("{ROOMS}/{catalog_id}/code");
        let bytes = self.call(Method::PUT, &target, String::new()).await?;
        decode::<JoinCodeResponse>(&bytes)?.try_into()
    }

    async fn waiting_knocks(
        &self,
        catalog_id: RoomCatalogId,
    ) -> RoomManagementResult<Vec<RoomKnock>> {
        let target = format!("{ROOMS}/{catalog_id}/knocks");
        let bytes = self.call(Method::GET, &target, String::new()).await?;
        decode::<KnocksResponse>(&bytes)?
            .knocks
            .into_iter()
            .map(RoomKnock::try_from)
            .collect()
    }

    async fn invite_person(
        &self,
        catalog_id: RoomCatalogId,
        account_id: PrincipalId,
    ) -> RoomManagementResult<()> {
        let body = serde_json::to_string(&InvitePersonBody {
            account_id: account_id.to_string(),
        })
        .map_err(|_| failure(RoomManagementFailureKind::Internal))?;
        let target = format!("{ROOMS}/{catalog_id}/invitations");
        self.call(Method::POST, &target, body).await.map(drop)
    }
}

impl ControlPlaneRoomManagementGateway for ReqwestControlPlaneRoomManagementGateway {
    fn create<'a>(
        &'a self,
        catalog_id: RoomCatalogId,
        name: &'a str,
        agent_id: AgentId,
    ) -> PortFuture<'a, RoomManagementResult<CreatedRoom>> {
        Box::pin(self.create_room(catalog_id, name, agent_id))
    }

    fn code(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RoomManagementResult<RoomCodeState>> {
        Box::pin(self.current_code(catalog_id))
    }

    fn generate_code(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RoomManagementResult<RoomJoinCode>> {
        Box::pin(self.new_code(catalog_id))
    }

    fn disable_code(&self, catalog_id: RoomCatalogId) -> PortFuture<'_, RoomManagementResult<()>> {
        Box::pin(async move {
            let target = format!("{ROOMS}/{catalog_id}/code");
            self.call(Method::DELETE, &target, String::new())
                .await
                .map(drop)
        })
    }

    fn knocks(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RoomManagementResult<Vec<RoomKnock>>> {
        Box::pin(self.waiting_knocks(catalog_id))
    }

    fn answer_knock(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
        admit: bool,
    ) -> PortFuture<'_, RoomManagementResult<()>> {
        Box::pin(async move {
            let target = format!("{ROOMS}/{catalog_id}/knocks/{agent_id}");
            let method = if admit { Method::PUT } else { Method::DELETE };
            self.call(method, &target, String::new()).await.map(drop)
        })
    }

    fn invite(
        &self,
        catalog_id: RoomCatalogId,
        account_id: PrincipalId,
    ) -> PortFuture<'_, RoomManagementResult<()>> {
        Box::pin(self.invite_person(catalog_id, account_id))
    }

    fn remove_agent(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RoomManagementResult<()>> {
        Box::pin(async move {
            let target = format!("{ROOMS}/{catalog_id}/agents/{agent_id}");
            self.call(Method::DELETE, &target, String::new())
                .await
                .map(drop)
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateRoomBody<'a> {
    name: &'a str,
    agent_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InvitePersonBody {
    account_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatedRoomResponse {
    catalog_id: String,
    matrix_room_id: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JoinCodeResponse {
    code: String,
    created_at_unix_ms: i64,
}

#[derive(Deserialize)]
struct KnocksResponse {
    knocks: Vec<KnockResponse>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct KnockResponse {
    agent_id: String,
    display_name: String,
    knocked_at_unix_ms: i64,
    expires_at_unix_ms: i64,
}

#[derive(Deserialize)]
struct ErrorCodeResponse {
    code: String,
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> RoomManagementResult<T> {
    serde_json::from_slice(bytes).map_err(|_| invalid_response())
}

impl TryFrom<CreatedRoomResponse> for CreatedRoom {
    type Error = RoomManagementFailure;

    fn try_from(value: CreatedRoomResponse) -> Result<Self, Self::Error> {
        let catalog_id = uuid_v7(&value.catalog_id).map(RoomCatalogId::from_uuid)?;
        let matrix_room_id =
            MatrixRoomReference::new(value.matrix_room_id).map_err(|_| invalid_response())?;
        if value.name.trim().is_empty() {
            return Err(invalid_response());
        }
        Ok(Self {
            catalog_id,
            matrix_room_id,
            name: value.name,
        })
    }
}

impl TryFrom<JoinCodeResponse> for RoomJoinCode {
    type Error = RoomManagementFailure;

    fn try_from(value: JoinCodeResponse) -> Result<Self, Self::Error> {
        let looks_like_code = !value.code.is_empty()
            && value.code.chars().count() <= MAX_CODE_CHARS
            && value
                .code
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-');
        if !looks_like_code {
            return Err(invalid_response());
        }
        Ok(Self {
            code: value.code,
            created_at: UtcMillis::new(value.created_at_unix_ms).map_err(|_| invalid_response())?,
        })
    }
}

impl TryFrom<KnockResponse> for RoomKnock {
    type Error = RoomManagementFailure;

    fn try_from(value: KnockResponse) -> Result<Self, Self::Error> {
        if value.display_name.trim().is_empty() {
            return Err(invalid_response());
        }
        Ok(Self {
            agent_id: uuid_v7(&value.agent_id).map(AgentId::from_uuid)?,
            display_name: value.display_name,
            knocked_at: UtcMillis::new(value.knocked_at_unix_ms).map_err(|_| invalid_response())?,
            expires_at: UtcMillis::new(value.expires_at_unix_ms).map_err(|_| invalid_response())?,
        })
    }
}

fn uuid_v7(value: &str) -> RoomManagementResult<uuid::Uuid> {
    uuid::Uuid::parse_str(value)
        .ok()
        .filter(|id| id.get_version() == Some(uuid::Version::SortRand))
        .ok_or_else(invalid_response)
}

/// 错误码按点号后面那段认：口令、敲门和私人房间的错误码前缀不一样，意思是同一套。没有错误码的
/// 404 说明控制面还没有这些接口。
fn status_failure(status: StatusCode, code: Option<&str>) -> RoomManagementFailure {
    if code.is_some_and(|code| code.starts_with("device.")) {
        return failure(RoomManagementFailureKind::NotAuthorized);
    }
    let reason = code
        .and_then(|code| code.split_once('.'))
        .map(|(_, reason)| reason);
    let kind = match reason {
        Some(reason) if reason == "invalid" || reason.starts_with("invalid_") => {
            RoomManagementFailureKind::InvalidRequest
        }
        Some("forbidden") => RoomManagementFailureKind::Forbidden,
        Some("not_found" | "not_revealable") => RoomManagementFailureKind::NotFound,
        Some("conflict") => RoomManagementFailureKind::Conflict,
        Some("rate_limited") => RoomManagementFailureKind::RateLimited,
        _ => match status {
            StatusCode::UNAUTHORIZED => RoomManagementFailureKind::NotAuthorized,
            StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED if code.is_none() => {
                RoomManagementFailureKind::Unsupported
            }
            StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT => RoomManagementFailureKind::ControlPlaneUnavailable,
            _ if status.is_server_error() => RoomManagementFailureKind::ControlPlaneUnavailable,
            _ => RoomManagementFailureKind::InvalidControlPlaneResponse,
        },
    };
    failure(kind)
}

fn map_session_failure(session_failure: BridgeSessionFailure) -> RoomManagementFailure {
    let kind = match session_failure.kind() {
        BridgeSessionFailureKind::NotAuthorized => RoomManagementFailureKind::NotAuthorized,
        BridgeSessionFailureKind::RefreshOutcomeUnknown
        | BridgeSessionFailureKind::SecureStorageUnavailable
        | BridgeSessionFailureKind::ControlPlaneUnavailable => {
            RoomManagementFailureKind::ControlPlaneUnavailable
        }
        BridgeSessionFailureKind::CorruptSecureStorage
        | BridgeSessionFailureKind::InvalidControlPlaneResponse
        | BridgeSessionFailureKind::Internal => RoomManagementFailureKind::Internal,
    };
    failure(kind)
}

const fn invalid_response() -> RoomManagementFailure {
    failure(RoomManagementFailureKind::InvalidControlPlaneResponse)
}

const fn failure(kind: RoomManagementFailureKind) -> RoomManagementFailure {
    RoomManagementFailure::new(kind)
}

#[cfg(test)]
mod tests;
