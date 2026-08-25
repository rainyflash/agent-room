use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::PrivateRoomPermissions,
};

use crate::authentication::AuthenticatedPrincipal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrivateRoomInvitation {
    pub principal_id: PrincipalId,
    pub permissions: PrivateRoomPermissions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePrivateRoom {
    pub actor: AuthenticatedPrincipal,
    /// 由客户端生成的 `UUIDv7` 幂等标识，同时成为稳定房间目录标识。
    pub catalog_id: RoomCatalogId,
    pub name: String,
    pub description: String,
    pub retention_days: Option<u16>,
    pub invitations: Vec<PrivateRoomInvitation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectPrivateRoom {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvitePrivateRoomMember {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    pub target_principal_id: PrincipalId,
    pub permissions: PrivateRoomPermissions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateRoomMembershipAction {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovernPrivateRoomMember {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    pub target_principal_id: PrincipalId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangePrivateRoomPermissions {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    pub target_principal_id: PrincipalId,
    pub permissions: PrivateRoomPermissions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferPrivateRoomOwnership {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    pub target_principal_id: PrincipalId,
    pub former_owner_permissions: PrivateRoomPermissions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivePrivateRoom {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
}
