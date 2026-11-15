use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId, SubRoomCategoryId},
    private_rooms::PrivateRoomPermissions,
    room_listing::{RoomJoinMode, RoomListingSettings},
    sub_rooms::SubRoomPolicy,
    time::UtcMillis,
};

use crate::{
    authentication::AuthenticatedPrincipal,
    devices::AuthenticatedDevice,
    ports::{PrivateRoomSnapshot, SubRoomCategory, SubRoomEntry},
};

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
    /// 新建时就公开（`specs/public-rooms/design.md`）：房间不加密，建好就进目录。给了就得是公开的
    /// 设置；只有人建房时能给。
    pub listing: Option<RoomListingSettings>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectPrivateRoom {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
}

/// 房间是谁建的（`specs/agent-rooms/design.md`“网页”）：建房的 Agent，以及房主是不是网络 Agent。
/// 网络 Agent 当房主的房间服务器读得到，人进来之前要看到。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateRoomOrigin {
    /// 人建的房间为空。
    pub created_by_agent: Option<PrivateRoomCreatorAgent>,
    pub owner_is_network_agent: bool,
}

/// 建房的 Agent。查不到它的记录时没有名字。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateRoomCreatorAgent {
    pub agent_id: AgentId,
    pub display_name: Option<String>,
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

/// 邀请一个人。Agent 替主体邀请时只在 Agent 建的房间里，权限只能是查看、发言和自动发言
/// （`PrivateRoomPermissions::INVITED_BY_AGENT`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvitePrivateRoomMember {
    pub actor: PrivateRoomActor,
    pub catalog_id: RoomCatalogId,
    pub target_principal_id: PrincipalId,
    pub permissions: PrivateRoomPermissions,
}

/// 以邀请链接创建人的名义邀请拿着链接的人。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteByLink {
    pub catalog_id: RoomCatalogId,
    pub link_creator: PrincipalId,
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

/// 归档房间。人要最近验证过身份（由网页登录的路由查）；Agent 里只有网络 Agent 能关自己的房间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivePrivateRoom {
    pub actor: PrivateRoomActor,
    pub catalog_id: RoomCatalogId,
}

/// 在一间房下面建子房间（`specs/redesign/sub-rooms.md`）。`catalog_id` 是子房间的房间号，由请求的
/// 幂等键给出：同一个请求重发，回的是同一间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSubRoom {
    pub actor: PrivateRoomActor,
    pub parent_catalog_id: RoomCatalogId,
    pub catalog_id: RoomCatalogId,
    pub name: String,
    pub description: String,
    /// 门（“谁能进”）：`Open` 是没有门。
    pub access: RoomJoinMode,
    /// 不在名单里显示。只有有门的能勾。
    pub hidden: bool,
    /// 放进哪个分类（2e-1b），得是这间房里的。不选是“未分类”。
    pub category_id: Option<SubRoomCategoryId>,
    /// Agent 替它的主人建时，是哪个 Agent。
    pub created_by_agent: Option<AgentId>,
}

/// 本机 Agent 替它的主人建子房间（2g-1）。和 [`CreateSubRoom`] 一样，只是分类按名字给：Agent 只知道
/// 名字。没有用途、不藏：要的话主人在网页上改。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateAgentSubRoom {
    pub actor: PrivateRoomActor,
    pub parent_catalog_id: RoomCatalogId,
    pub catalog_id: RoomCatalogId,
    pub name: String,
    pub access: RoomJoinMode,
    /// 分类的名字，不分大小写、不看两头的空白。这间房里没有这个分类时放进“未分类”，不替它新建。
    pub category: Option<String>,
    pub created_by_agent: AgentId,
}

/// 建好的子房间，和 Agent 要知道的几样：它在哪间房下面、放进了哪个分类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSubRoomCreated {
    pub room: PrivateRoomSnapshot,
    pub parent_name: String,
    /// 放进的分类的名字（照这间房里分类本来的写法）；“未分类”时为 `None`。
    pub category: Option<String>,
    /// 要了分类，这间房里却没有叫这个名字的：放进了“未分类”。
    pub category_missing: bool,
    /// 新建的；为 false 时回到了同一个 Agent 成员在这间房下面建过、没归档的同名子房间（2h-1b）。
    pub created: bool,
}

/// 读一间房下面的子房间。`catalog_id` 是父房间，或者它的任何一间子房间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListSubRooms {
    pub actor: AuthenticatedPrincipal,
    pub catalog_id: RoomCatalogId,
    /// 要“已归档”的那份（2e-3）：只有归档了的，最近归档的在前。不要时只有没归档的，按建的先后。
    pub archived: bool,
}

/// 子房间的名单：父房间是谁，和看的人看得到的那些。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubRoomList {
    pub parent_catalog_id: RoomCatalogId,
    pub parent_name: String,
    /// 看的人自己现在能不能在这间房下面建子房间（不算他的 Agent）。
    pub can_create: bool,
    /// 这间房的分类，按先后（2e-1）。整间房共用，每个人看到的一样。
    pub categories: Vec<SubRoomCategory>,
    pub sub_rooms: Vec<SubRoomView>,
}

/// 在一间房里建一个子房间的分类（`specs/redesign/sub-rooms.md` 的“分类”，2e-1）。`category_id`
/// 由客户端给：同一个请求重发，认得出是同一个。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSubRoomCategory {
    pub actor: AuthenticatedPrincipal,
    pub parent_catalog_id: RoomCatalogId,
    pub category_id: SubRoomCategoryId,
    pub name: String,
}

/// 给分类改名、挪位置。没带的那一项不动。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviseSubRoomCategory {
    pub actor: AuthenticatedPrincipal,
    pub parent_catalog_id: RoomCatalogId,
    pub category_id: SubRoomCategoryId,
    pub name: Option<String>,
    /// 挪到第几个（从 0 起）；超过最后一个的放到最后。
    pub position: Option<u8>,
}

/// 删一个分类：里面的子房间回到“未分类”。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteSubRoomCategory {
    pub actor: AuthenticatedPrincipal,
    pub parent_catalog_id: RoomCatalogId,
    pub category_id: SubRoomCategoryId,
}

/// 名单里的一间。有门、看的人又不在里面时，只看得到名字、用途和谁在管，人数这些不给。在等的
/// 申请有几个只给管得了这一间的人（`manages`）看。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubRoomView {
    pub entry: SubRoomEntry,
    pub inside_visible: bool,
    pub manages: bool,
    /// 看的人此刻能不能把它恢复。没归档的为假。
    pub restorable: bool,
}

/// 人自己进一间子房间，或者从里面退出（`specs/redesign/sub-rooms.md` 第 2b 步）。地址里带着它
/// 所在的房间，对不上的当没有这间子房间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubRoomMembershipAction {
    pub actor: AuthenticatedPrincipal,
    pub parent_catalog_id: RoomCatalogId,
    pub catalog_id: RoomCatalogId,
}

/// 改子房间的名字、用途、门和“不在名单里显示”。没带的那一项不动。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviseSubRoom {
    pub actor: AuthenticatedPrincipal,
    pub parent_catalog_id: RoomCatalogId,
    pub catalog_id: RoomCatalogId,
    pub name: Option<String>,
    pub description: Option<String>,
    /// 换门：`Open` 是拆门。
    pub access: Option<RoomJoinMode>,
    pub hidden: Option<bool>,
    /// 挪到哪个分类（2e-1b）：`Some(None)` 是挪回“未分类”。
    pub category: Option<Option<SubRoomCategoryId>>,
}

/// 把一间子房间归档（`specs/redesign/sub-rooms.md` 第 2e-2 步）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveSubRoom {
    pub actor: AuthenticatedPrincipal,
    pub parent_catalog_id: RoomCatalogId,
    pub catalog_id: RoomCatalogId,
}

/// 把归档了的一间子房间恢复（第 2e-2 步）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreSubRoom {
    pub actor: AuthenticatedPrincipal,
    pub parent_catalog_id: RoomCatalogId,
    pub catalog_id: RoomCatalogId,
}

/// 申请进一间要批准的子房间（`specs/redesign/sub-rooms.md` 第 2d-2 步）。`message` 是留的那
/// 句话，可以是空的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestSubRoomJoin {
    pub actor: AuthenticatedPrincipal,
    pub parent_catalog_id: RoomCatalogId,
    pub catalog_id: RoomCatalogId,
    pub message: String,
}

/// 管得了这一间的人处理一个人的申请。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerSubRoomJoinRequest {
    pub actor: AuthenticatedPrincipal,
    pub parent_catalog_id: RoomCatalogId,
    pub catalog_id: RoomCatalogId,
    pub requester: PrincipalId,
    pub approve: bool,
}

/// 管得了这一间的人把父房间里的一个人请进来、移出、封禁或者解封（`specs/redesign/sub-rooms.md`
/// 第 2d-3 步）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovernSubRoomMember {
    pub actor: AuthenticatedPrincipal,
    pub parent_catalog_id: RoomCatalogId,
    pub catalog_id: RoomCatalogId,
    pub target: PrincipalId,
}

/// 设（`grant`）或者撤一位子房间管理员（`specs/redesign/room-roles.md` 的 R5，子房间第 2d-4 步）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSubRoomManager {
    pub actor: AuthenticatedPrincipal,
    pub parent_catalog_id: RoomCatalogId,
    pub catalog_id: RoomCatalogId,
    pub target: PrincipalId,
    pub grant: bool,
}

/// 申请以后他在这一间的处境。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubRoomJoinState {
    /// 在等批准。
    Pending { requested_at: UtcMillis },
    /// 不用等：这一间没有门了、他是父房间的房主或管理员、他本来就在里面。已经把他记成已加入，网页
    /// 照“服务器记着已加入、Matrix 上是受邀”那一套替他加入。
    Joined,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectSubRoomPolicy {
    pub actor: AuthenticatedPrincipal,
    pub parent_catalog_id: RoomCatalogId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSubRoomPolicy {
    pub actor: AuthenticatedPrincipal,
    pub parent_catalog_id: RoomCatalogId,
    pub policy: SubRoomPolicy,
}

/// 一间房里子房间的三项设置，和看的人改不改得动（只有房主）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubRoomPolicyView {
    pub policy: SubRoomPolicy,
    pub can_change: bool,
}
