//! 私人房间口令：`POST /join-codes/resolve` 只查看房间，`POST /agents/{agentId}/join-codes/redeem`
//! 让这个 Agent 加入。两者都用设备签名，正文只有口令。

use std::sync::Arc;

use agent_room_application::ports::PortFuture;
use agent_room_bridge_core::{
    join_codes::{
        ControlPlaneJoinCodeGateway, JoinCodeFailure, JoinCodeFailureKind, JoinCodeResult,
        JoinCodeRoom,
    },
    session::{BridgeSessionFailure, BridgeSessionFailureKind, ControlPlaneRequestAuthorizer},
};
use agent_room_domain::{
    ids::{AgentId, RoomCatalogId},
    rooms::MatrixRoomReference,
};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{
    ControlPlaneHttpConfig, ControlPlaneHttpConfigurationError, configured_client,
    read_limited_response_body, signed_request_headers,
};

const RESOLVE_TARGET: &str = "/join-codes/resolve";

pub struct ReqwestControlPlaneJoinCodeGateway {
    client: Client,
    base_url: Url,
    authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
}

impl ReqwestControlPlaneJoinCodeGateway {
    /// 创建按设备签名查看与兑换私人房间口令的 HTTP 网关。
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

    async fn post(&self, request_target: &str, code: &str) -> JoinCodeResult<JoinCodeRoom> {
        let body = serde_json::to_string(&CodeBody { code })
            .map_err(|_| failure(JoinCodeFailureKind::Internal))?;
        let authorized = self
            .authorizer
            .authorize("POST", request_target, &body)
            .await
            .map_err(map_session_failure)?;
        let request_url = self
            .base_url
            .join(request_target.trim_start_matches('/'))
            .map_err(|_| failure(JoinCodeFailureKind::Internal))?;
        let request = signed_request_headers(
            self.client
                .post(request_url)
                .header(reqwest::header::CONTENT_TYPE, "application/json"),
            &authorized,
            "POST",
            request_target,
        )
        .map_err(|()| failure(JoinCodeFailureKind::Internal))?;
        // 兑换可以放心重试：同一个 Agent 再兑换一次只是再确认一遍成员身份。
        let response = request
            .body(body)
            .send()
            .await
            .map_err(|_| failure(JoinCodeFailureKind::ControlPlaneUnavailable))?;
        decode_response(response).await
    }
}

impl ControlPlaneJoinCodeGateway for ReqwestControlPlaneJoinCodeGateway {
    fn resolve<'a>(&'a self, code: &'a str) -> PortFuture<'a, JoinCodeResult<JoinCodeRoom>> {
        Box::pin(self.post(RESOLVE_TARGET, code))
    }

    fn redeem<'a>(
        &'a self,
        agent_id: AgentId,
        code: &'a str,
    ) -> PortFuture<'a, JoinCodeResult<JoinCodeRoom>> {
        Box::pin(async move {
            self.post(&format!("/agents/{agent_id}/join-codes/redeem"), code)
                .await
        })
    }
}

#[derive(Serialize)]
struct CodeBody<'a> {
    code: &'a str,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JoinCodeRoomResponse {
    catalog_id: String,
    matrix_room_id: String,
    name: String,
}

#[derive(Deserialize)]
struct ErrorCodeResponse {
    code: String,
}

async fn decode_response(response: reqwest::Response) -> JoinCodeResult<JoinCodeRoom> {
    let status = response.status();
    let body = read_limited_response_body(response).await.map_err(|()| {
        if status.is_success() {
            failure(JoinCodeFailureKind::InvalidControlPlaneResponse)
        } else {
            status_failure(status, None)
        }
    })?;
    if !status.is_success() {
        let code = serde_json::from_slice::<ErrorCodeResponse>(&body)
            .ok()
            .map(|envelope| envelope.code);
        return Err(status_failure(status, code.as_deref()));
    }
    serde_json::from_slice::<JoinCodeRoomResponse>(&body)
        .map_err(|_| failure(JoinCodeFailureKind::InvalidControlPlaneResponse))?
        .try_into()
}

impl TryFrom<JoinCodeRoomResponse> for JoinCodeRoom {
    type Error = JoinCodeFailure;

    fn try_from(value: JoinCodeRoomResponse) -> Result<Self, Self::Error> {
        let invalid = || failure(JoinCodeFailureKind::InvalidControlPlaneResponse);
        let catalog_id = uuid::Uuid::parse_str(&value.catalog_id)
            .ok()
            .filter(|id| id.get_version() == Some(uuid::Version::SortRand))
            .map(RoomCatalogId::from_uuid)
            .ok_or_else(invalid)?;
        let matrix_room_id =
            MatrixRoomReference::new(value.matrix_room_id).map_err(|_| invalid())?;
        if value.name.trim().is_empty() {
            return Err(invalid());
        }
        Ok(Self {
            catalog_id,
            matrix_room_id,
            name: value.name,
        })
    }
}

/// 口令自己的错误按错误码区分；没有口令错误码的 404 说明控制面还没有这个接口。
fn status_failure(status: StatusCode, code: Option<&str>) -> JoinCodeFailure {
    let kind = match code {
        Some("join_code.invalid") => JoinCodeFailureKind::InvalidCode,
        Some("join_code.not_found") => JoinCodeFailureKind::NotFound,
        Some("join_code.forbidden") => JoinCodeFailureKind::Forbidden,
        Some("join_code.conflict") => JoinCodeFailureKind::RoomUnavailable,
        Some("join_code.rate_limited") => JoinCodeFailureKind::RateLimited,
        _ => match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => JoinCodeFailureKind::NotAuthorized,
            StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED => {
                JoinCodeFailureKind::Unsupported
            }
            StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT => JoinCodeFailureKind::ControlPlaneUnavailable,
            _ if status.is_server_error() => JoinCodeFailureKind::ControlPlaneUnavailable,
            _ => JoinCodeFailureKind::InvalidControlPlaneResponse,
        },
    };
    failure(kind)
}

fn map_session_failure(session_failure: BridgeSessionFailure) -> JoinCodeFailure {
    let kind = match session_failure.kind() {
        BridgeSessionFailureKind::NotAuthorized => JoinCodeFailureKind::NotAuthorized,
        BridgeSessionFailureKind::RefreshOutcomeUnknown
        | BridgeSessionFailureKind::SecureStorageUnavailable
        | BridgeSessionFailureKind::ControlPlaneUnavailable => {
            JoinCodeFailureKind::ControlPlaneUnavailable
        }
        BridgeSessionFailureKind::CorruptSecureStorage
        | BridgeSessionFailureKind::InvalidControlPlaneResponse
        | BridgeSessionFailureKind::Internal => JoinCodeFailureKind::Internal,
    };
    failure(kind)
}

const fn failure(kind: JoinCodeFailureKind) -> JoinCodeFailure {
    JoinCodeFailure::new(kind)
}

#[cfg(test)]
mod tests;
