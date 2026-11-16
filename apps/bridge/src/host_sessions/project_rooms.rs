//! 认领 GitHub 仓库、现在核对（`specs/redesign/project-rooms.md` 的 3P-4a）：不要会话，也不建人物，
//! 直接用这台电脑的授权问控制面。认上以后进房间照常按分享链接开会话。

use std::sync::Arc;

use agent_room_bridge_core::project_rooms::{
    ControlPlaneProjectRoomGateway, ProjectRoomFailure, ProjectRoomFailureKind, RepositoryClaim,
    RepositoryClaimStatus,
};
use agent_room_bridge_ipc::{
    IpcCheckRepositoryClaimRequest, IpcClaimRepositoryRequest, IpcErrorCategory,
    IpcRepositoryCheck, IpcRepositoryClaim, IpcRepositoryClaimStatus, IpcResponse,
};
use agent_room_domain::{
    ids::{RepositoryClaimId, RoomCatalogId},
    project_rooms::GithubRepository,
    time::UtcMillis,
};

use crate::ipc::BridgeIpcDispatchFailure;

pub(crate) struct ProjectRoomAccess {
    pub(crate) gateway: Arc<dyn ControlPlaneProjectRoomGateway>,
}

impl ProjectRoomAccess {
    pub(crate) async fn claim(
        &self,
        request: IpcClaimRepositoryRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        // 请求进来时已经验过；这里再解析一遍拿到类型。
        let repository = GithubRepository::parse(&request.repository).map_err(|_| {
            BridgeIpcDispatchFailure::new(
                "bridge.ipc.repository_invalid",
                IpcErrorCategory::Validation,
                false,
            )
        })?;
        let target = request
            .catalog_id
            .as_deref()
            .map(|id| {
                uuid::Uuid::parse_str(id)
                    .map(RoomCatalogId::from_uuid)
                    .map_err(|_| {
                        BridgeIpcDispatchFailure::new(
                            "bridge.ipc.room_invalid",
                            IpcErrorCategory::Validation,
                            false,
                        )
                    })
            })
            .transpose()?;
        self.gateway
            .claim(&repository, target)
            .await
            .map(claim_response)
            .map_err(project_room_failure)
    }

    pub(crate) async fn check(
        &self,
        request: IpcCheckRepositoryClaimRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let claim_id = uuid::Uuid::parse_str(&request.claim_id)
            .map(RepositoryClaimId::from_uuid)
            .map_err(|_| {
                BridgeIpcDispatchFailure::new(
                    "bridge.ipc.claim_id_invalid",
                    IpcErrorCategory::Validation,
                    false,
                )
            })?;
        self.gateway
            .check(claim_id)
            .await
            .map(claim_response)
            .map_err(project_room_failure)
    }
}

fn claim_response(claim: RepositoryClaim) -> IpcResponse {
    IpcResponse::RepositoryClaim {
        claim: IpcRepositoryClaim {
            claim_id: claim.claim_id.map(|id| id.as_uuid().to_string()),
            repository: claim.repository.full_name(),
            share_code: claim.share_code.as_str().to_owned(),
            share_url: claim.share_url,
            agents_line: claim.agents_line,
            badge_markdown: claim.badge_markdown,
            status: match claim.status {
                RepositoryClaimStatus::Pending => IpcRepositoryClaimStatus::Pending,
                RepositoryClaimStatus::Bound => IpcRepositoryClaimStatus::Bound,
            },
            catalog_id: claim.catalog_id.map(|id| id.to_string()),
            claimed_at_unix_ms: claim.claimed_at.map(UtcMillis::value),
            last_check: claim.last_check.map(|check| IpcRepositoryCheck {
                at_unix_ms: check.at.value(),
                result: check.result,
            }),
        },
    }
}

/// 控制面的拒绝换成 `bridge.project_room.*`，命令行和 MCP 按它换成自己的说法。
fn project_room_failure(failure: ProjectRoomFailure) -> BridgeIpcDispatchFailure {
    let (code, category, retryable) = match failure.kind() {
        ProjectRoomFailureKind::InvalidRepository => (
            "bridge.project_room.invalid_repository",
            IpcErrorCategory::Validation,
            false,
        ),
        ProjectRoomFailureKind::Forbidden => (
            "bridge.project_room.forbidden",
            IpcErrorCategory::Authorization,
            false,
        ),
        ProjectRoomFailureKind::NotFound => (
            "bridge.project_room.not_found",
            IpcErrorCategory::Validation,
            false,
        ),
        ProjectRoomFailureKind::TooManyClaims => (
            "bridge.project_room.too_many_claims",
            IpcErrorCategory::Conflict,
            false,
        ),
        ProjectRoomFailureKind::RepositoryTaken => (
            "bridge.project_room.repository_taken",
            IpcErrorCategory::Conflict,
            false,
        ),
        ProjectRoomFailureKind::RoomNotEligible => (
            "bridge.project_room.room_not_eligible",
            IpcErrorCategory::Conflict,
            false,
        ),
        ProjectRoomFailureKind::RoomAlreadyBound => (
            "bridge.project_room.room_already_bound",
            IpcErrorCategory::Conflict,
            false,
        ),
        // 要等上一阵子才能再核对；不标成可重试，免得调用方立刻重发。
        ProjectRoomFailureKind::CheckTooSoon => (
            "bridge.project_room.check_too_soon",
            IpcErrorCategory::Conflict,
            false,
        ),
        ProjectRoomFailureKind::Conflict => (
            "bridge.project_room.conflict",
            IpcErrorCategory::Conflict,
            true,
        ),
        ProjectRoomFailureKind::Unsupported => (
            "bridge.project_room.unsupported",
            IpcErrorCategory::IncompatibleVersion,
            false,
        ),
        ProjectRoomFailureKind::NotAuthorized => (
            "bridge.host_session.device_authorization_required",
            IpcErrorCategory::Authentication,
            false,
        ),
        ProjectRoomFailureKind::ControlPlaneUnavailable => (
            "bridge.project_room.unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        ProjectRoomFailureKind::InvalidControlPlaneResponse | ProjectRoomFailureKind::Internal => (
            "bridge.project_room.failed",
            IpcErrorCategory::DependencyUnavailable,
            false,
        ),
    };
    BridgeIpcDispatchFailure::new(code, category, retryable)
}
