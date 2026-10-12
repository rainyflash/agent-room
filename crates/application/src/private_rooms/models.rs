use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId},
    private_rooms::PrivateRoomPermissions,
    time::UtcMillis,
};

use crate::{authentication::AuthenticatedPrincipal, devices::AuthenticatedDevice};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrivateRoomInvitation {
    pub principal_id: PrincipalId,
    pub permissions: PrivateRoomPermissions,
}

/// 谁在操作私人房间（`specs/agent-rooms/design.md`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrivateRoomActor {
    /// 人的网页登录。
    Person(AuthenticatedPrincipal),
    /// 本机 Bridge 替这台电脑登录的账号行动：设备签名的请求。
    Device(AuthenticatedDevice),
    /// 网络 Agent 替自己行动：令牌请求由网关换成一次性的设备身份，主体是它的合成主体。
    /// 只有网络 Agent 的路由造得出这一种。
    NetworkAgent(AuthenticatedDevice),
}

impl PrivateRoomActor {
    pub const fn principal_id(&self) -> PrincipalId {
        match self {
            Self::Person(person) => person.principal_id,
            Self::Device(device) | Self::NetworkAgent(device) => device.account.principal.id(),
        }
    }

    pub const fn expires_at(&self) -> UtcMillis {
        match self {
            Self::Person(person) => person.expires_at,
            Self::Device(device) | Self::NetworkAgent(device) => device.access_token_expires_at,
        }
    }

    /// 操作者自己的 Matrix 账号。网络 Agent 的合成主体没有（那个用户名从来没注册过），
    /// 只有它的 Agent 有。
    pub fn own_matrix_user_id(&self) -> Option<&str> {
        match self {
            Self::Person(person) => Some(&person.matrix_user_id),
            Self::Device(device) => Some(&device.account.matrix_user_id),
            Self::NetworkAgent(_) => None,
        }
    }

    /// Agent 替主体操作：本机 Bridge 或网络 Agent，不是人的网页登录。
    pub const fn is_agent(&self) -> bool {
        !matches!(self, Self::Person(_))
    }
}

impl From<AuthenticatedPrincipal> for PrivateRoomActor {
    fn from(person: AuthenticatedPrincipal) -> Self {
        Self::Person(person)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePrivateRoom {
    pub actor: PrivateRoomActor,
    /// 由客户端生成的 `UUIDv7` 幂等标识，同时成为稳定房间目录标识。
    pub catalog_id: RoomCatalogId,
    pub name: String,
    pub description: String,
    pub retention_days: Option<u16>,
    /// 人建房时可以顺带邀请；Agent 建房时为空，之后再一个个邀请。
    pub invitations: Vec<PrivateRoomInvitation>,
    /// 建房的 Agent：Agent 建房时必须给，且是操作者账号名下的；人建房时为空。
    pub created_by_agent: Option<AgentId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectPrivateRoom {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPrivateRooms {
    pub actor: AuthenticatedPrincipal,
}

/// 以账号（而不是人的登录会话）为主体列出私人房间：设备已通过签名认证，Bridge 替账号的
/// Agent 查询它能进哪些房间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPrivateRoomsForAccount {
    pub principal_id: PrincipalId,
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

/// 房主给房间改名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamePrivateRoom {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivePrivateRoom {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
}
