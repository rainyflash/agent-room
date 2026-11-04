//! 房主的 Agent 管房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”）：经控制面
//! `/agents/{a}/instances/{i}/rooms/{c}/moderation` 下的设备签名路由读授权、照授权动手。

use agent_room_bridge_core::room_management::{
    RoomManagementFailureKind, RoomManagementResult, RoomModerationGrant, RoomModerationOutcome,
    RoomModerationRequest,
};
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, RoomCatalogId},
    time::UtcMillis,
};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Refused, ReqwestControlPlaneRoomManagementGateway, decode, failure};

/// 没有授权（或撤了、到期了）时控制面回的错误码：不算失败，回答里没有授权就是了。
const NOT_GRANTED: &str = "agent_moderation.not_granted";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GrantResponse {
    #[allow(dead_code, reason = "控制面回的授权 ID，Agent 用不着")]
    grant_id: String,
    rules: String,
    actions: Vec<String>,
    expires_at_unix_ms: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ActionBody<'a> {
    action: &'a str,
    target: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'a str>,
    note: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_seconds: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActionResponse {
    action_id: String,
    action: String,
    status: String,
    #[serde(default)]
    expires_at_unix_ms: Option<i64>,
}

fn moderation_target(
    agent_id: AgentId,
    instance_id: AgentInstanceId,
    catalog_id: RoomCatalogId,
) -> String {
    format!("/agents/{agent_id}/instances/{instance_id}/rooms/{catalog_id}/moderation")
}

fn time(value: i64) -> RoomManagementResult<UtcMillis> {
    UtcMillis::new(value)
        .map_err(|_| failure(RoomManagementFailureKind::InvalidControlPlaneResponse))
}

impl ReqwestControlPlaneRoomManagementGateway {
    pub(super) async fn moderation_grant_internal(
        &self,
        agent_id: AgentId,
        instance_id: AgentInstanceId,
        catalog_id: RoomCatalogId,
    ) -> RoomManagementResult<Option<RoomModerationGrant>> {
        let target = moderation_target(agent_id, instance_id, catalog_id);
        let bytes = match self.send(Method::GET, &target, String::new(), None).await {
            Ok(bytes) => bytes,
            Err(Refused::Response {
                code: Some(code), ..
            }) if code == NOT_GRANTED => return Ok(None),
            Err(refused) => return Err(refused.into_failure()),
        };
        let grant = decode::<GrantResponse>(&bytes)?;
        Ok(Some(RoomModerationGrant {
            rules: grant.rules,
            actions: grant.actions,
            expires_at: time(grant.expires_at_unix_ms)?,
        }))
    }

    pub(super) async fn moderate_internal(
        &self,
        agent_id: AgentId,
        instance_id: AgentInstanceId,
        catalog_id: RoomCatalogId,
        action_id: Uuid,
        request: &RoomModerationRequest,
    ) -> RoomManagementResult<RoomModerationOutcome> {
        let target = format!(
            "{}/actions",
            moderation_target(agent_id, instance_id, catalog_id)
        );
        let body = serde_json::to_string(&ActionBody {
            action: request.action,
            target: &request.target,
            reason: request.reason.as_deref(),
            note: &request.note,
            duration_seconds: request.duration_seconds,
        })
        .map_err(|_| failure(RoomManagementFailureKind::Internal))?;
        let bytes = self
            .send(Method::POST, &target, body, Some(action_id.to_string()))
            .await
            .map_err(Refused::into_failure)?;
        let outcome = decode::<ActionResponse>(&bytes)?;
        Ok(RoomModerationOutcome {
            action_id: outcome.action_id,
            action: outcome.action,
            status: outcome.status,
            expires_at: outcome.expires_at_unix_ms.map(time).transpose()?,
        })
    }
}
