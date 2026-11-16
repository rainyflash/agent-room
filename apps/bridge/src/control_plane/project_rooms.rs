//! 项目房间的认领（`specs/redesign/project-rooms.md` 的 3P-4a）：经控制面
//! `/devices/current/project-rooms/claims` 下的设备签名路由替这台电脑的账户认领仓库、现在核对。签名对着
//! 请求原样的方法、路径和正文。
//!
//! 回答里的那一行、徽章和分享链接是控制面照部署的站点拼好的，这里只查它们像样，原样转交。

use std::sync::Arc;

use agent_room_application::ports::PortFuture;
use agent_room_bridge_core::{
    project_rooms::{
        ControlPlaneProjectRoomGateway, ProjectRoomFailure, ProjectRoomFailureKind,
        ProjectRoomResult, RepositoryClaim, RepositoryClaimCheck, RepositoryClaimStatus,
    },
    session::{BridgeSessionFailure, BridgeSessionFailureKind, ControlPlaneRequestAuthorizer},
};
use agent_room_domain::{
    ids::{RepositoryClaimId, RoomCatalogId},
    project_rooms::GithubRepository,
    room_listing::RoomShareCode,
    time::UtcMillis,
};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{
    ControlPlaneHttpConfig, ControlPlaneHttpConfigurationError, configured_client,
    read_limited_response_body, signed_request_headers,
};

const CLAIMS: &str = "/devices/current/project-rooms/claims";
/// 那一行、徽章、链接的上限：控制面拼出来的都远比这短，长得离谱的不转交。
const MAX_LINE_BYTES: usize = 1_024;
/// 核对结果是小写字母和下划线的短词。
const MAX_RESULT_BYTES: usize = 64;

pub struct ReqwestControlPlaneProjectRoomGateway {
    client: Client,
    base_url: Url,
    authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
}

impl ReqwestControlPlaneProjectRoomGateway {
    /// 创建按设备签名认领仓库的 HTTP 网关。
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

    async fn post(&self, target: &str, body: String) -> ProjectRoomResult<RepositoryClaim> {
        let authorized = self
            .authorizer
            .authorize("POST", target, &body)
            .await
            .map_err(map_session_failure)?;
        let url = self
            .base_url
            .join(target.trim_start_matches('/'))
            .map_err(|_| failure(ProjectRoomFailureKind::Internal))?;
        let mut request = self.client.post(url);
        if !body.is_empty() {
            request = request.header(reqwest::header::CONTENT_TYPE, "application/json");
        }
        let request = signed_request_headers(request, &authorized, "POST", target)
            .map_err(|()| failure(ProjectRoomFailureKind::Internal))?;
        // 认领可以放心重试：同一个人同一个仓库再来，控制面回同一个认领。
        let response = request
            .body(body)
            .send()
            .await
            .map_err(|_| failure(ProjectRoomFailureKind::ControlPlaneUnavailable))?;
        let status = response.status();
        let bytes = read_limited_response_body(response).await.map_err(|()| {
            if status.is_success() {
                failure(ProjectRoomFailureKind::InvalidControlPlaneResponse)
            } else {
                status_failure(status, None)
            }
        })?;
        if !status.is_success() {
            let code = serde_json::from_slice::<ErrorCodeResponse>(&bytes)
                .ok()
                .map(|envelope| envelope.code);
            return Err(status_failure(status, code.as_deref()));
        }
        serde_json::from_slice::<ClaimResponse>(&bytes)
            .map_err(|_| invalid_response())?
            .try_into()
    }
}

impl ControlPlaneProjectRoomGateway for ReqwestControlPlaneProjectRoomGateway {
    fn claim<'a>(
        &'a self,
        repository: &'a GithubRepository,
        target: Option<RoomCatalogId>,
    ) -> PortFuture<'a, ProjectRoomResult<RepositoryClaim>> {
        Box::pin(async move {
            let body = serde_json::to_string(&ClaimBody {
                repository: repository.full_name(),
                catalog_id: target.map(|id| id.to_string()),
            })
            .map_err(|_| failure(ProjectRoomFailureKind::Internal))?;
            self.post(CLAIMS, body).await
        })
    }

    fn check(
        &self,
        claim_id: RepositoryClaimId,
    ) -> PortFuture<'_, ProjectRoomResult<RepositoryClaim>> {
        Box::pin(async move {
            let target = format!("{CLAIMS}/{}/check", claim_id.as_uuid());
            self.post(&target, String::new()).await
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClaimBody {
    repository: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    catalog_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaimResponse {
    claim_id: Option<String>,
    repository: String,
    share_code: String,
    share_url: String,
    agents_line: String,
    badge_markdown: String,
    status: String,
    catalog_id: Option<String>,
    claimed_at_unix_ms: Option<i64>,
    last_check: Option<CheckResponse>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckResponse {
    at_unix_ms: i64,
    result: String,
}

#[derive(Deserialize)]
struct ErrorCodeResponse {
    code: String,
}

impl TryFrom<ClaimResponse> for RepositoryClaim {
    type Error = ProjectRoomFailure;

    fn try_from(value: ClaimResponse) -> Result<Self, Self::Error> {
        let status = match value.status.as_str() {
            "pending" => RepositoryClaimStatus::Pending,
            "bound" => RepositoryClaimStatus::Bound,
            _ => return Err(invalid_response()),
        };
        let claim_id = value
            .claim_id
            .as_deref()
            .map(|id| uuid_v7(id).map(RepositoryClaimId::from_uuid))
            .transpose()?;
        let catalog_id = value
            .catalog_id
            .as_deref()
            .map(|id| uuid_v7(id).map(RoomCatalogId::from_uuid))
            .transpose()?;
        // 还在等的有认领号；认上了的认领已经删了，绑在一间房上。
        let consistent = match status {
            RepositoryClaimStatus::Pending => claim_id.is_some(),
            RepositoryClaimStatus::Bound => claim_id.is_none() && catalog_id.is_some(),
        };
        let lines_valid = [&value.share_url, &value.agents_line, &value.badge_markdown]
            .into_iter()
            .all(|line| line_valid(line))
            && value.share_url.contains(&value.share_code)
            && value.agents_line.contains(&value.share_url);
        if !consistent || !lines_valid {
            return Err(invalid_response());
        }
        Ok(Self {
            claim_id,
            repository: GithubRepository::parse(&value.repository)
                .map_err(|_| invalid_response())?,
            share_code: RoomShareCode::parse(&value.share_code).map_err(|_| invalid_response())?,
            share_url: value.share_url,
            agents_line: value.agents_line,
            badge_markdown: value.badge_markdown,
            status,
            catalog_id,
            claimed_at: value
                .claimed_at_unix_ms
                .map(|at| UtcMillis::new(at).map_err(|_| invalid_response()))
                .transpose()?,
            last_check: value.last_check.map(TryInto::try_into).transpose()?,
        })
    }
}

impl TryFrom<CheckResponse> for RepositoryClaimCheck {
    type Error = ProjectRoomFailure;

    fn try_from(value: CheckResponse) -> Result<Self, Self::Error> {
        let result_valid = !value.result.is_empty()
            && value.result.len() <= MAX_RESULT_BYTES
            && value
                .result
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_');
        if !result_valid {
            return Err(invalid_response());
        }
        Ok(Self {
            at: UtcMillis::new(value.at_unix_ms).map_err(|_| invalid_response())?,
            result: value.result,
        })
    }
}

fn line_valid(line: &str) -> bool {
    !line.is_empty() && line.len() <= MAX_LINE_BYTES && !line.chars().any(char::is_control)
}

fn uuid_v7(value: &str) -> ProjectRoomResult<uuid::Uuid> {
    uuid::Uuid::parse_str(value)
        .ok()
        .filter(|id| id.get_version() == Some(uuid::Version::SortRand))
        .ok_or_else(invalid_response)
}

/// 认领自己的错误按错误码区分；没有错误码的 404、405 说明控制面还没有这个接口。
fn status_failure(status: StatusCode, code: Option<&str>) -> ProjectRoomFailure {
    if code.is_some_and(|code| code.starts_with("device.")) {
        return failure(ProjectRoomFailureKind::NotAuthorized);
    }
    let kind = match code {
        Some("project_room.invalid_repository") => ProjectRoomFailureKind::InvalidRepository,
        Some("project_room.forbidden") => ProjectRoomFailureKind::Forbidden,
        Some("project_room.not_found") => ProjectRoomFailureKind::NotFound,
        Some("project_room.too_many_claims") => ProjectRoomFailureKind::TooManyClaims,
        Some("project_room.repository_taken") => ProjectRoomFailureKind::RepositoryTaken,
        Some("project_room.room_not_eligible") => ProjectRoomFailureKind::RoomNotEligible,
        Some("room_listing.repository_bound") => ProjectRoomFailureKind::RoomAlreadyBound,
        Some("project_room.check_too_soon") => ProjectRoomFailureKind::CheckTooSoon,
        Some("project_room.conflict") => ProjectRoomFailureKind::Conflict,
        Some("project_room.dependency_unavailable") => {
            ProjectRoomFailureKind::ControlPlaneUnavailable
        }
        Some("project_room.invalid_request" | "project_room.invalid_resource") => {
            ProjectRoomFailureKind::Internal
        }
        _ => match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                ProjectRoomFailureKind::NotAuthorized
            }
            StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED => {
                ProjectRoomFailureKind::Unsupported
            }
            StatusCode::REQUEST_TIMEOUT
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT => ProjectRoomFailureKind::ControlPlaneUnavailable,
            _ if status.is_server_error() => ProjectRoomFailureKind::ControlPlaneUnavailable,
            _ => ProjectRoomFailureKind::InvalidControlPlaneResponse,
        },
    };
    failure(kind)
}

fn map_session_failure(session_failure: BridgeSessionFailure) -> ProjectRoomFailure {
    let kind = match session_failure.kind() {
        BridgeSessionFailureKind::NotAuthorized => ProjectRoomFailureKind::NotAuthorized,
        BridgeSessionFailureKind::RefreshOutcomeUnknown
        | BridgeSessionFailureKind::SecureStorageUnavailable
        | BridgeSessionFailureKind::ControlPlaneUnavailable => {
            ProjectRoomFailureKind::ControlPlaneUnavailable
        }
        BridgeSessionFailureKind::CorruptSecureStorage
        | BridgeSessionFailureKind::InvalidControlPlaneResponse
        | BridgeSessionFailureKind::Internal => ProjectRoomFailureKind::Internal,
    };
    failure(kind)
}

const fn invalid_response() -> ProjectRoomFailure {
    failure(ProjectRoomFailureKind::InvalidControlPlaneResponse)
}

const fn failure(kind: ProjectRoomFailureKind) -> ProjectRoomFailure {
    ProjectRoomFailure::new(kind)
}

#[cfg(test)]
mod tests;
