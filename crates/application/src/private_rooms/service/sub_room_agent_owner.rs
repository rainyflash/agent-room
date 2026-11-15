//! 子房间第 2f-1a 步（`specs/redesign/sub-rooms.md` 的“进门”）：本机 Agent 进子房间时，它的主人还
//! 不在里面的，没有门的子房间先把主人加进去。
//!
//! 进门的规矩（`admits_agent_of`）不变，还是要主人在这一间里已加入、能发言：你的 Agent 在哪间子房间，
//! 你就在哪间，左栏里看得到，也对它在里面说的话负责。把主人加进去走人自己点“加入”的那一段
//! （[`PrivateRoomService::admit_into_sub_room`]），连禁言一起照算。有门的不替主人加，父房间的房主和
//! 管理员也一样：门是给人自己决定进不进的。

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    room_listing::RoomJoinMode,
    sub_rooms::{authorize_sub_room_admission, authorize_sub_room_join},
};

use crate::ports::PortFuture;

use super::{
    super::{PrivateRoomFailureKind, PrivateRoomResult},
    PrivateRoomService,
    sub_room_membership::{LoadedSubRoom, SubRoomEntrant},
    sub_rooms::has_joined,
};

/// 本机 Agent 要进一间子房间时，它的主人在不在里面。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentOwnerSeat {
    /// 在里面、能发言：本来就在，或者刚替他加进去。
    Inside,
    /// 这一间有门，他不在里面：要他自己先申请，或者等管理员把他请进来。
    DoorClosed,
    /// 进不来：不是子房间、他不在父房间里或者说不了话、被封禁了、归档了，或者在里面却只能看。
    Refused,
}

/// 本机 Agent 进子房间之前，替它的主人占好位置（没有门的才替他加）。
pub trait SubRoomAgentOwnerSeating: Send + Sync {
    fn seat_agent_owner(
        &self,
        catalog_id: RoomCatalogId,
        owner: PrincipalId,
    ) -> PortFuture<'_, PrivateRoomResult<AgentOwnerSeat>>;
}

impl SubRoomAgentOwnerSeating for PrivateRoomService {
    fn seat_agent_owner(
        &self,
        catalog_id: RoomCatalogId,
        owner: PrincipalId,
    ) -> PortFuture<'_, PrivateRoomResult<AgentOwnerSeat>> {
        Box::pin(self.seat_agent_owner_internal(catalog_id, owner))
    }
}

impl PrivateRoomService {
    async fn seat_agent_owner_internal(
        &self,
        catalog_id: RoomCatalogId,
        owner: PrincipalId,
    ) -> PrivateRoomResult<AgentOwnerSeat> {
        const OPERATION: &str = "sub_room.seat_agent_owner";
        let Some(parent_catalog_id) = self
            .find(catalog_id, OPERATION)
            .await?
            .and_then(|room| room.parent_catalog_id())
        else {
            return Ok(AgentOwnerSeat::Refused);
        };
        // 看不到的（他不在父房间里、藏起来的他又进不去）和不存在的一样，都是进不来。
        let LoadedSubRoom {
            parent,
            sub_room,
            link,
        } = match self
            .load_sub_room(parent_catalog_id, catalog_id, owner, OPERATION)
            .await
        {
            Ok(loaded) => loaded,
            Err(failure) if failure.kind() == PrivateRoomFailureKind::NotFound => {
                return Ok(AgentOwnerSeat::Refused);
            }
            Err(failure) => return Err(failure),
        };
        if sub_room.room().admits_agent_of(owner) {
            return Ok(AgentOwnerSeat::Inside);
        }
        // 在里面却只能看的，和父房间里说不了话的，都带不了 Agent 进来，也不替他加。
        if has_joined(sub_room.room(), owner) || !parent.room().admits_agent_of(owner) {
            return Ok(AgentOwnerSeat::Refused);
        }
        if link.access != RoomJoinMode::Open {
            // 被封禁、归档的照实说进不来；只是有门，就请他先申请。
            return Ok(
                match authorize_sub_room_admission(parent.room(), sub_room.room(), owner) {
                    Ok(_) => AgentOwnerSeat::DoorClosed,
                    Err(_) => AgentOwnerSeat::Refused,
                },
            );
        }
        let Ok(permissions) =
            authorize_sub_room_join(parent.room(), sub_room.room(), owner, link.access)
        else {
            return Ok(AgentOwnerSeat::Refused);
        };
        let matrix_user = self.principal_matrix_user(owner, OPERATION).await?;
        let seated = self
            .admit_into_sub_room(
                parent_catalog_id,
                sub_room,
                &SubRoomEntrant {
                    principal: owner,
                    matrix_user: &matrix_user,
                    permissions,
                },
                self.clock.now(),
                OPERATION,
            )
            .await;
        match seated {
            Ok(seated) if seated.room().admits_agent_of(owner) => Ok(AgentOwnerSeat::Inside),
            Ok(_) => Ok(AgentOwnerSeat::Refused),
            // 读到他在父房间里以后、存之前他被移出去了。
            Err(failure) if failure.kind() == PrivateRoomFailureKind::NotFound => {
                Ok(AgentOwnerSeat::Refused)
            }
            Err(failure) => Err(failure),
        }
    }
}
