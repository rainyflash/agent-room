//! 房主的 Agent 管房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”）：会话的 Agent 在主人给了
//! 管理授权的房间里读规则、照规则动手。能不能做由控制面按授权把关，这里只把请求转过去。

use agent_room_bridge_core::room_management::RoomModerationRequest;
use agent_room_bridge_ipc::{
    IpcModerateRoomRequest, IpcModerationAction, IpcModerationGrant, IpcModerationOutcome,
    IpcResponse, IpcSessionRoom,
};
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, RoomCatalogId},
    time::UtcMillis,
};
use uuid::Uuid;

use super::room_management::{RoomManagementAccess, room_failure};
use crate::ipc::BridgeIpcDispatchFailure;

/// 会话的 Agent 要管的房间是怎么说的。
#[derive(Debug, Clone, Copy)]
pub(crate) enum RoomRef<'a> {
    /// 没说：开会话时进的那个。
    Session,
    /// 目录 ID。
    Catalog(&'a str),
    /// 消息里的 Matrix 房间 ID（`roomId`）。
    Matrix(&'a str),
}

impl<'a> RoomRef<'a> {
    /// 管房间的请求说的房间：两样只会给一样（IPC 那一层查过）。
    pub(crate) fn of(request: &'a IpcModerateRoomRequest) -> Self {
        match (request.catalog_id.as_deref(), request.room_id.as_deref()) {
            (Some(catalog_id), _) => Self::Catalog(catalog_id),
            (None, Some(room_id)) => Self::Matrix(room_id),
            (None, None) => Self::Session,
        }
    }

    /// 会话的 Agent 进过的这个房间是不是说的这个。
    pub(crate) fn names(self, room: &IpcSessionRoom) -> bool {
        match self {
            Self::Session => false,
            Self::Catalog(catalog_id) => room.catalog_id == catalog_id,
            Self::Matrix(room_id) => room.room_id == room_id,
        }
    }
}

/// 会话的 Agent 和它要管的房间。
pub(crate) struct ModerationScope {
    pub(crate) agent: AgentId,
    pub(crate) instance: AgentInstanceId,
    pub(crate) room: RoomCatalogId,
}

impl RoomManagementAccess {
    pub(crate) async fn moderate(
        &self,
        scope: ModerationScope,
        request: IpcModerateRoomRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let ModerationScope {
            agent: agent_id,
            instance: instance_id,
            room: catalog_id,
        } = scope;
        let action = match request.action {
            IpcModerationAction::Rules => {
                let grant = self
                    .gateway
                    .moderation_grant(agent_id, instance_id, catalog_id)
                    .await
                    .map_err(room_failure)?;
                return Ok(IpcResponse::RoomModerationRules {
                    catalog_id: catalog_id.to_string(),
                    grant: grant.map(|grant| IpcModerationGrant {
                        rules: grant.rules,
                        actions: grant.actions,
                        expires_at_unix_ms: grant.expires_at.value(),
                    }),
                });
            }
            action => action.as_str(),
        };
        // IPC 那一层已经查过：动手的请求一定带着对象和理由。
        let request = RoomModerationRequest {
            action,
            target: request.target.unwrap_or_default(),
            reason: request.reason,
            note: request.note.unwrap_or_default(),
            duration_seconds: request.duration_seconds,
        };
        let outcome = self
            .gateway
            .moderate(agent_id, instance_id, catalog_id, Uuid::now_v7(), &request)
            .await
            .map_err(room_failure)?;
        Ok(IpcResponse::RoomModerated {
            catalog_id: catalog_id.to_string(),
            outcome: IpcModerationOutcome {
                action_id: outcome.action_id,
                action: outcome.action,
                status: outcome.status,
                expires_at_unix_ms: outcome.expires_at.map(UtcMillis::value),
            },
        })
    }
}
