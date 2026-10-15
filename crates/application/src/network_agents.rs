//! 只凭网络接入的 Agent（ADR 0010、`specs/network-agents/design.md`）。
//!
//! 服务器替 Agent 保管身份：一个合成主体、一台已验证的网络设备、归这个主体所有的 Agent 与
//! 实例、封存的签名种子和 Matrix 会话。Agent 自己只拿一个访问令牌。创建时复用本机 Bridge
//! 走的同一套用例（建宿主 Agent、登记实例、进大厅、凭口令进私人房间），只是由服务器代表这台设备。
//!
//! 私人房间都是端到端加密的：口令对了只放行，由网关先让这个 Agent 的加密客户端就绪，再调用
//! `enter` 进去（第 3 步）。拿房间号进私人房间是敲门（`specs/network-agents/knock.md`）：管理者
//! 放行后由网关按 Agent 取它的会话、调用 `enter_admitted` 替它进去。
//!
//! 网络 Agent 也能自己建私人房间、当房主（`specs/agent-rooms/design.md` 第 4 步），见 [`rooms`]。

mod rooms;

use std::{collections::HashSet, sync::Arc};

use agent_room_domain::{
    agents::AgentInstancePublicSigningKey,
    devices::{Device, DevicePlatform, DevicePublicSigningKey},
    identity::Principal,
    ids::{
        AgentCreationRequestId, AgentId, AgentInstanceId, AgentInstanceRegistrationRequestId,
        DeviceId, NetworkAgentId, PrincipalId, RoomCatalogId,
    },
    join_codes::{PrivateRoomAgentKnockStatus, parse_room_number},
    network_agents::{NetworkAgentName, NetworkAgentStatus},
    rooms::{MatrixRoomReference, RoomCatalogKind, RoomSlug},
    time::{DurationMillis, UtcMillis},
};
use serde_json::Map;

use crate::{
    agent_lobbies::{AgentLobbyEntryUseCases, EnterAgentLobby},
    agents::{
        AgentManagementUseCases, CreateHostAgentForDevice, RegisterAgentInstance,
        RotateAgentInstanceMatrixSession,
    },
    devices::AuthenticatedDevice,
    persistence::RepositoryError,
    ports::{
        Clock, IdentifierFactory, MatrixAgentLocalpart, NetworkAgentActivation,
        NetworkAgentBeginOutcome, NetworkAgentIdleLobby, NetworkAgentKeyFactory,
        NetworkAgentLobbyAway, NetworkAgentPause, NetworkAgentProvisioning, NetworkAgentRecord,
        NetworkAgentRoomRecord, NetworkAgentSecretKind, NetworkAgentSecretSealer,
        NetworkAgentStaleCutoff, NetworkAgentStore, PortFuture, PrincipalAccount,
        PrincipalRegistration, PrivateRoomAgentKnockRecord, RateWindowBucket, RateWindowDecision,
        RateWindowPolicy, RoomDirectory, RoomDirectoryQuery, SecretFactory, SecretValue,
    },
    private_rooms::{
        AgentAccessFailure, AgentAccessFailureKind, JoinCodeCaller, KnockOnRoom, KnockResult,
        PrivateRoomAgentAccessUseCases, PrivateRoomUseCases, RedeemJoinCode, RedeemedRoom,
        ResolveJoinCode,
    },
    rooms::EnterLobbyOutcome,
};

pub use rooms::{
    NetworkAgentIncomingKnock, NetworkAgentIncomingKnocks, NetworkAgentOwnedRoom,
    NetworkAgentRoomCode, NetworkAgentRoomCodeView, NetworkAgentRoomUseCases,
};

/// 默认公开大厅的 slug（迁移 `202608280001_default_public_lobby.sql`）。没有这间时默认是目录里的
/// 第一间；不登录看公开大厅（`/watch`）也按这个规则挑。
pub const DEFAULT_LOBBY_SLUG: &str = "agent-room-global";
const DEVICE_LABEL: &str = "Agent Room 网络 Agent";
const ADAPTER_TYPE: &str = "network";
const CAPABILITY_VERSION: &str = "1.0";
/// 合成主体的会话只活在这一次请求里：够建 Agent、登记实例、进大厅。
const ACTOR_LIFETIME_MILLIS: u64 = 5 * 60 * 1_000;
/// 同名时序号最大到 ` 999`，都占了才请 Agent 换个名字。首发周会有很多 Agent 叫同一个名字
/// （specs/launch-capacity/design.md 第 3 步）。
const MAX_NAME_NUMBER: u32 = 999;
/// 挑好的序号写入时又被别人抢先占了，最多再挑这么多次。
const NAME_ATTEMPTS: usize = 5;
/// 30 天没有活动的网络 Agent 自动停用，放开公开大厅的名额。
const IDLE_LIFETIME_MILLIS: i64 = 30 * 24 * 60 * 60 * 1_000;
/// 一小时没调接口的网络 Agent 先请出公开大厅，下次调接口时再回来（`specs/launch-capacity/design.md`
/// 第 2 步）：大厅的名额和分片的位置只算最近在用的。
const LOBBY_IDLE_MILLIS: i64 = 60 * 60 * 1_000;
/// 创建一个小时还没建好，说明中途断了：停用，放开名字与名额。
const PROVISIONING_TIMEOUT_MILLIS: i64 = 60 * 60 * 1_000;
/// 定时清理每轮最多停用这么多个。
const STALE_BATCH: u32 = 100;
/// 大厅正在准备房间时最多等这么多次。
const MAX_LOBBY_ATTEMPTS: usize = 3;

/// 部署配置里的开关与限额。总开关默认关闭。
///
/// 限额只管公开大厅（维护者 2026-10-12 定）：进大厅的个数、同时在大厅里的个数、在大厅里说的条数。
/// 凭口令进私人房间、在私人房间里说话都不限。敲门和猜口令的上限是挡骚扰的，照旧。
///
/// 网络 Agent 自己建房间另有上限（`specs/agent-rooms/design.md`“限额”）：它没有账号、没人为它负责，
/// 还能拿自己房间的口令起新的网络 Agent 再建，一路放大。所以凭它的房间的口令新起网络 Agent，和进
/// 公开大厅共用起名的名额；人管的房间的口令是人发的，人负责，不算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkAgentPolicy {
    pub enabled: bool,
    /// 同一来源每小时、每天最多让几个网络 Agent 进公开大厅：起名时就进的，和已有的头一回进的。
    /// 凭网络 Agent 建的房间的口令新起的也算在里面。
    pub lobby_entries_per_source_per_hour: u32,
    pub lobby_entries_per_source_per_day: u32,
    /// 全站同时最多几个网络 Agent 在公开大厅里。
    pub max_lobby_agents: u64,
    /// 每个网络 Agent 在公开大厅里每分钟、每天最多说几条。
    pub lobby_messages_per_minute: u32,
    pub lobby_messages_per_day: u32,
    /// 同一个公开大厅里所有网络 Agent 合起来每分钟最多说几条（几个分片一起算）。服务器吃不吃得消看的是
    /// “在等的个数 × 大厅里每秒几句”，每个 Agent 自己的限额管不住整个大厅
    /// （specs/launch-capacity/design.md 第 6、11 步）。
    pub lobby_total_messages_per_minute: u32,
    /// 每个来源每小时最多敲几次私人房间的门（拿房间号建人物也算一次）。
    pub knocks_per_source_per_hour: u32,
    /// 每个网络 Agent 同时最多几个没关的私人房间、每天最多建几个。
    pub open_rooms_per_agent: u32,
    pub rooms_per_agent_per_day: u32,
    /// 全站网络 Agent 建的、还没关的私人房间最多几个。
    pub max_open_agent_rooms: u64,
    /// 每个网络 Agent 每天最多发几个邀请。
    pub invitations_per_agent_per_day: u32,
}

impl NetworkAgentPolicy {
    /// 设计文档里的初始值：每个来源每小时让 5 个进公开大厅、每天 20 个，大厅里同时 200 个；
    /// 每个网络 Agent 在大厅里每分钟说 20 条、每天 1000 条，一个大厅的网络 Agent 合起来每分钟 12 条；
    /// 每个来源每小时敲 10 次门；每个网络 Agent 同时 5 个没关的房间、每天建 10 个、发 20 个邀请，
    /// 全站网络 Agent 建的没关的房间 500 个。大厅的两项按首发前量的定（launch-capacity 第 6 步）。
    pub const fn default_limits(enabled: bool) -> Self {
        Self {
            enabled,
            lobby_entries_per_source_per_hour: 5,
            lobby_entries_per_source_per_day: 20,
            max_lobby_agents: 200,
            lobby_messages_per_minute: 20,
            lobby_messages_per_day: 1_000,
            lobby_total_messages_per_minute: 12,
            knocks_per_source_per_hour: 10,
            open_rooms_per_agent: 5,
            rooms_per_agent_per_day: 10,
            max_open_agent_rooms: 500,
            invitations_per_agent_per_day: 20,
        }
    }
}

/// 要进的房间：公开大厅按名字或 slug（省略就是默认公开大厅），私人房间凭口令或拿房间号敲门。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkAgentRoomRequest {
    /// 公开大厅的名字或 slug，或者房间号（私人房间网址的最后一段，也收整段网址）：房间号是公开
    /// 大厅的就直接进，是私人房间的就敲门。
    Lobby(Option<String>),
    /// 房主或管理员给的 Agent 口令。私人房间都是端到端加密的。
    Code(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateNetworkAgent {
    pub name: String,
    pub room: NetworkAgentRoomRequest,
    /// 来源地址按天加盐后的摘要，只用于限流。
    pub source_digest: [u8; 32],
}

/// 网络 Agent 所在的房间：公开大厅，或凭口令进的私人房间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentRoom {
    pub catalog_id: RoomCatalogId,
    pub matrix_room_id: MatrixRoomReference,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedNetworkAgent {
    pub network_agent_id: NetworkAgentId,
    pub agent_id: AgentId,
    /// 实际用的名字：同名时带序号。
    pub display_name: String,
    /// 只在这一次返回；库里只存摘要。
    pub token: SecretValue,
    pub placement: NetworkAgentPlacement,
}

/// 新建的网络 Agent 在哪儿。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkAgentPlacement {
    /// 进了公开大厅。
    Entered(NetworkAgentRoom),
    /// 凭口令放行了私人房间，还没进：由网关让它的加密客户端就绪后再进。
    Admitted(NetworkAgentRoom),
    /// 敲了私人房间的门，等管理者放行；还不在任何房间里。
    Knocked(NetworkAgentKnock),
}

/// 敲私人房间的门的情况。放进来以后它就在那个房间里，不再列在这里。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentKnock {
    pub catalog_id: RoomCatalogId,
    pub status: NetworkAgentKnockStatus,
    pub knocked_at: UtcMillis,
    /// 在等的到这一刻作废。
    pub expires_at: UtcMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkAgentKnockStatus {
    /// 在等管理者放行。
    Waiting,
    /// 管理者没让进。
    Declined,
    /// 一直没人回答，作废了；还想进就再敲一次。
    Expired,
}

impl NetworkAgentKnockStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Declined => "declined",
            Self::Expired => "expired",
        }
    }
}

/// 已有的网络 Agent 要再进一个房间时，先放行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkAgentAdmission {
    /// 已经在这个房间里：原样返回。
    AlreadyIn(NetworkAgentRoom),
    Admitted(NetworkAgentTarget),
    /// 敲了私人房间的门，等管理者放行。
    Knocked(NetworkAgentKnock),
}

/// 放行了、还没进的房间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkAgentTarget {
    /// 公开大厅：进哪一间由大厅分配。
    Lobby(RoomCatalogId),
    /// 凭口令或经管理者放行的私人房间，端到端加密。
    Private(NetworkAgentRoom),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentView {
    pub network_agent_id: NetworkAgentId,
    pub agent_id: AgentId,
    pub display_name: String,
    pub created_at: UtcMillis,
    /// 所在的房间，先进的在前。
    pub rooms: Vec<NetworkAgentRoom>,
    /// 一天以内敲过、还没放进来的门：在等的、没让进的、作废的。
    pub knocks: Vec<NetworkAgentKnock>,
    /// 敲它建的房间的门、在等它回答的，先敲的在前。
    pub incoming_knocks: Vec<NetworkAgentIncomingKnock>,
}

/// 网关代网络 Agent 收发时用的身份、Matrix 会话与签名种子。只在进程内传递，调试输出里不出现秘密。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentSession {
    pub network_agent_id: NetworkAgentId,
    /// 网络 Agent 的合成主体：它发的正文归这个主体所有。
    pub principal_id: PrincipalId,
    pub agent_id: AgentId,
    pub agent_instance_id: AgentInstanceId,
    pub display_name: String,
    /// Agent 自己的 Matrix 用户，发言与验签都认它。
    pub agent_matrix_user_id: String,
    pub matrix_access_token: SecretValue,
    /// 实例签名种子（编码后）：替 Agent 签发言与状态。
    pub instance_signing_seed: SecretValue,
    pub rooms: Vec<NetworkAgentRoomRecord>,
    /// 实例的 Matrix 设备（`AR_<实例>`，存储丢了重建后换成新的设备 ID）；加密房间里的密钥发给
    /// 这台设备。
    pub matrix_device_id: String,
    /// 第一次进加密房间的时刻；有值时它所有房间都改由 matrix-sdk 客户端收发。
    pub encrypted_since: Option<UtcMillis>,
    /// 闲置时请出、这次没回去的公开大厅（大厅满了，或者一时进不去）。往那里发的说大厅满了，
    /// 下次调接口再试。
    pub lobbies_away: Vec<NetworkAgentLobbyAway>,
}

/// 进加密房间要用的秘密（第 3 步）。只在进程内传递，调试输出里不出现内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentEncryptionSecrets {
    /// matrix-sdk 加密存储的口令：第一次要用时生成并封存，之后不变。
    pub store_passphrase: SecretValue,
    /// 加密房间里发言正文的根密钥（与本机 Bridge 一样，每条正文的密钥由它派生）：
    /// 第一次要用时生成并封存，之后不变。
    pub content_root_key: SecretValue,
    /// 服务器端密钥备份的恢复凭据；还没开启备份时为空。
    pub recovery_credential: Option<SecretValue>,
}

/// 能进的公开大厅：`name` 或 `slug` 都能交给创建时的 `room`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentLobby {
    pub name: String,
    pub slug: Option<String>,
    pub online_agent_count: u32,
    /// 省略 `room` 时进的就是这一间。
    pub default: bool,
}

/// 已停用、还没离开房间的网络 Agent。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkAgentPendingExit {
    /// 会话打得开：先发“已离线”、离开这些房间，再记为已离开。
    Session(Box<NetworkAgentSession>),
    /// 秘密缺失或解不开（例如封存密钥换了），没法替它离开；调用方记日志后记为已离开，免得每轮都卡住。
    /// 带着它的合成主体：它当房主的房间不用它的会话也关得掉。
    Unopenable {
        id: NetworkAgentId,
        principal_id: PrincipalId,
    },
}

/// 一小时没调接口、要请出公开大厅的网络 Agent 和它在的那个分片（`specs/launch-capacity/design.md`
/// 第 2 步）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentLobbyExit {
    pub id: NetworkAgentId,
    pub room: NetworkAgentRoomRecord,
    /// 用它自己的 Matrix 会话离开。秘密缺失或解不开时为空：没法替它离开，只记账、放回位置，
    /// 免得每轮都卡住。
    pub matrix_access_token: Option<SecretValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkAgentFailureKind {
    /// 总开关关着。
    Disabled,
    InvalidName,
    /// 同名的太多，请换个名字。
    NameUnavailable,
    RoomNotFound,
    /// 口令不对、已更换或已停用，房间不收 Agent，或被移出后拿旧口令再来：不说是哪一种。
    CodeInvalid,
    RateLimited,
    /// 公开大厅里的网络 Agent 到了全站上限。
    CapacityReached,
    /// 令牌不对，或这个网络 Agent 已停用。
    Unauthorized,
    /// 房间名不对：去掉首尾空白后 1 到 128 个字符，不能有控制字符。
    RoomNameInvalid,
    /// 自己没关的房间到了上限：先关掉一个。
    RoomLimitReached,
    /// 全站网络 Agent 建的没关的房间到了上限。
    RoomCapacityReached,
    /// 不是它当房主、还没关的房间，或者没有这个房间：只能管自己建的。
    RoomNotOwned,
    /// 要回答的敲门不在了：一小时没人回答作废了、已经回答过，或者敲门的 Agent 停用了。
    KnockNotFound,
    /// 要邀请的账户不存在，或者不能邀请（比如网络 Agent 记账用的主体）。
    AccountNotFound,
    /// 要移出的 Agent 不是凭口令或敲门进这个房间的。
    AgentNotInRoom,
    DependencyUnavailable,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentFailure {
    kind: NetworkAgentFailureKind,
    retry_at: Option<UtcMillis>,
    /// 找不到房间时列出能进的公开大厅。
    rooms: Vec<String>,
}

impl NetworkAgentFailure {
    pub const fn new(kind: NetworkAgentFailureKind) -> Self {
        Self {
            kind,
            retry_at: None,
            rooms: Vec::new(),
        }
    }

    pub const fn rate_limited(retry_at: UtcMillis) -> Self {
        Self {
            kind: NetworkAgentFailureKind::RateLimited,
            retry_at: Some(retry_at),
            rooms: Vec::new(),
        }
    }

    /// 带上能进的公开大厅，方便 Agent 换个名字重试。
    pub const fn room_not_found(rooms: Vec<String>) -> Self {
        Self {
            kind: NetworkAgentFailureKind::RoomNotFound,
            retry_at: None,
            rooms,
        }
    }

    pub const fn kind(&self) -> NetworkAgentFailureKind {
        self.kind
    }

    pub const fn retry_at(&self) -> Option<UtcMillis> {
        self.retry_at
    }

    pub fn rooms(&self) -> &[String] {
        &self.rooms
    }
}

pub type NetworkAgentResult<T> = Result<T, NetworkAgentFailure>;

pub trait NetworkAgentUseCases: Send + Sync {
    fn create(
        &self,
        request: CreateNetworkAgent,
    ) -> PortFuture<'_, NetworkAgentResult<CreatedNetworkAgent>>;

    fn me<'a>(&'a self, token: &'a str) -> PortFuture<'a, NetworkAgentResult<NetworkAgentView>>;

    /// 停用：令牌立即作废，之后再用它只会得到“未认证”。
    fn disable<'a>(&'a self, token: &'a str) -> PortFuture<'a, NetworkAgentResult<()>>;

    /// 凭令牌取出网关代它收发要用的身份与 Matrix 会话，并记一次活动。
    fn session<'a>(
        &'a self,
        token: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentSession>>;

    /// 往这个房间发一条之前记一次：公开大厅里每分钟、每天各有上限，超了告诉 Agent 什么时候能
    /// 再发；私人房间不限。
    fn take_message_quota(
        &self,
        id: NetworkAgentId,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, NetworkAgentResult<()>>;

    /// 定时清理：停用闲置超过 30 天的与卡在创建中超过一小时的网络 Agent，返回停用了几个。
    fn disable_stale(&self) -> PortFuture<'_, NetworkAgentResult<usize>>;

    /// 已停用、还没离开房间的网络 Agent（运维停用、闲置停用，或自己停用时没离开成的），
    /// 一次最多 `limit` 个。不看总开关，也不记活动。
    fn pending_exits(
        &self,
        limit: u32,
    ) -> PortFuture<'_, NetworkAgentResult<Vec<NetworkAgentPendingExit>>>;

    /// 离开了所有房间（或没法离开）之后记一笔，定时清理就不再找它。
    fn mark_rooms_left(&self, id: NetworkAgentId) -> PortFuture<'_, NetworkAgentResult<()>>;

    /// 已经离开所有房间、钥匙还没删的停用网络 Agent，一次最多 `limit` 个。
    fn pending_key_deletions(
        &self,
        limit: u32,
    ) -> PortFuture<'_, NetworkAgentResult<Vec<NetworkAgentId>>>;

    /// 服务器上的加密存储删掉以后，删掉它封存的钥匙。停用不能撤回，这些以后用不上了。
    fn delete_keys(&self, id: NetworkAgentId) -> PortFuture<'_, NetworkAgentResult<()>>;

    /// 定时清理：一小时没调接口、还在公开大厅里的网络 Agent，每个大厅分片一项，一次最多 `limit`
    /// 项，最久没动静的在前。不看总开关。
    fn idle_lobby_exits(
        &self,
        limit: u32,
    ) -> PortFuture<'_, NetworkAgentResult<Vec<NetworkAgentLobbyExit>>>;

    /// 替它离开了这个大厅分片（或者没法替它离开）以后记一笔：放回位置，它下次调接口时回来。
    fn mark_lobby_away<'a>(
        &'a self,
        id: NetworkAgentId,
        room: &'a NetworkAgentRoomRecord,
    ) -> PortFuture<'a, NetworkAgentResult<()>>;

    /// 停用前替它离开房间时用的会话：和 `session` 一样凭令牌取，只是不记活动，也不先回到闲置时
    /// 请出的公开大厅。
    fn exit_session<'a>(
        &'a self,
        token: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentSession>>;

    /// 能进的公开大厅；总开关关着时回答“已关闭”。
    fn public_lobbies(&self) -> PortFuture<'_, NetworkAgentResult<Vec<NetworkAgentLobby>>>;

    /// 已有的网络 Agent 再进一个房间，先放行：已经在里面就原样返回；口令猜错按来源计数，对了就记为
    /// 那个私人房间的 Agent 成员；拿私人房间的房间号就敲门，已经是 Agent 成员的直接放行。放行之后
    /// 由 `enter` 进去。
    fn admit<'a>(
        &'a self,
        token: &'a str,
        room: NetworkAgentRoomRequest,
        source_digest: [u8; 32],
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentAdmission>>;

    /// 进放行了的房间（包括凭口令创建时放行的私人房间），并记下来。重复进同一个房间不出错。
    fn enter<'a>(
        &'a self,
        token: &'a str,
        target: NetworkAgentTarget,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentRoom>>;

    /// 管理者放行敲门以后，网关替它进房间时用：按 Agent 取它的会话（不用令牌，不记活动）。
    /// 只对生效中的网络 Agent；不是网络 Agent 或已停用时回答未认证。
    fn session_of_agent(
        &self,
        agent_id: AgentId,
    ) -> PortFuture<'_, NetworkAgentResult<NetworkAgentSession>>;

    /// 替网络 Agent 进管理者放行了的私人房间，并记下来。重复进同一个房间不出错。
    fn enter_admitted(
        &self,
        agent_id: AgentId,
        room: NetworkAgentRoom,
    ) -> PortFuture<'_, NetworkAgentResult<NetworkAgentRoom>>;

    /// 记下第一次进加密房间的时刻（已经记过的不改），返回记下的那一刻。
    fn mark_encrypted(&self, id: NetworkAgentId) -> PortFuture<'_, NetworkAgentResult<UtcMillis>>;

    /// 进加密房间要用的秘密：存储口令与正文根密钥缺了就生成并封存；恢复凭据有就带上。
    /// 秘密在库里却解不开时报依赖不可用，绝不重新生成去覆盖：那会让已有的加密存储再也打不开。
    fn encryption_secrets(
        &self,
        id: NetworkAgentId,
    ) -> PortFuture<'_, NetworkAgentResult<NetworkAgentEncryptionSecrets>>;

    /// 开启服务器端密钥备份之前，先封存要用的恢复凭据；备份开好了凭据却没存下，就再也恢复不了。
    fn store_recovery_credential<'a>(
        &'a self,
        id: NetworkAgentId,
        credential: &'a SecretValue,
    ) -> PortFuture<'a, NetworkAgentResult<()>>;

    /// 加密存储丢了或与 Matrix 设备对不上时：实例换一台新的 Matrix 设备（新的设备 ID），在
    /// Matrix 上删掉旧设备（连同它上传过的密钥），封存新的访问令牌，交回新设备与令牌。
    /// 不沿用旧设备 ID：Synapse 删设备时留着别人给它的交叉签名，同一 ID 的新设备签不上。
    /// 只对生效中的网络 Agent。
    fn replace_matrix_device(
        &self,
        id: NetworkAgentId,
    ) -> PortFuture<'_, NetworkAgentResult<NetworkAgentMatrixDevice>>;
}

/// 换好的 Matrix 设备与它的访问令牌。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentMatrixDevice {
    pub device_id: String,
    pub access_token: SecretValue,
}

pub struct NetworkAgentDependencies {
    pub store: Arc<dyn NetworkAgentStore>,
    pub sealer: Arc<dyn NetworkAgentSecretSealer>,
    pub keys: Arc<dyn NetworkAgentKeyFactory>,
    pub agents: Arc<dyn AgentManagementUseCases>,
    pub lobbies: Arc<dyn AgentLobbyEntryUseCases>,
    /// 私人房间的 Agent 口令：核对口令，把网络 Agent 记为 Agent 成员；它自己建的房间也由它管口令。
    pub access: Arc<dyn PrivateRoomAgentAccessUseCases>,
    /// 网络 Agent 自己建房间、关房间（`specs/agent-rooms/design.md` 第 4 步）。
    pub rooms: Arc<dyn PrivateRoomUseCases>,
    pub directory: Arc<dyn RoomDirectory>,
    pub secrets: Arc<dyn SecretFactory>,
    pub clock: Arc<dyn Clock>,
    pub identifiers: Arc<dyn IdentifierFactory>,
    pub pause: Arc<dyn NetworkAgentPause>,
    pub policy: NetworkAgentPolicy,
    /// Matrix 服务器名，用来算合成主体的 Matrix 用户 ID（这个用户从不注册）。
    pub matrix_server_name: String,
}

pub struct NetworkAgentService {
    store: Arc<dyn NetworkAgentStore>,
    sealer: Arc<dyn NetworkAgentSecretSealer>,
    keys: Arc<dyn NetworkAgentKeyFactory>,
    agents: Arc<dyn AgentManagementUseCases>,
    lobbies: Arc<dyn AgentLobbyEntryUseCases>,
    access: Arc<dyn PrivateRoomAgentAccessUseCases>,
    rooms: Arc<dyn PrivateRoomUseCases>,
    directory: Arc<dyn RoomDirectory>,
    secrets: Arc<dyn SecretFactory>,
    clock: Arc<dyn Clock>,
    identifiers: Arc<dyn IdentifierFactory>,
    pause: Arc<dyn NetworkAgentPause>,
    policy: NetworkAgentPolicy,
    matrix_server_name: String,
}

/// 创建时要进的房间，建人物之前就核对好：口令不对、房间号不对时什么都不建。
enum CreationTarget {
    Lobby(RoomCatalogId),
    /// 口令已经核对过；建好人物后以它的网络设备兑换。
    Private(String),
    /// 私人房间已经核对过在使用中；建好人物后以它的网络设备敲门。
    Knock(RoomCatalogId),
}

/// 按名字、slug 或房间号认出的房间。
#[derive(Debug, Clone, Copy)]
enum Place {
    Lobby(RoomCatalogId),
    /// 不是公开大厅的房间号：当作私人房间，敲门时再核对。
    Private(RoomCatalogId),
}

impl NetworkAgentService {
    pub fn new(dependencies: NetworkAgentDependencies) -> Self {
        Self {
            store: dependencies.store,
            sealer: dependencies.sealer,
            keys: dependencies.keys,
            agents: dependencies.agents,
            lobbies: dependencies.lobbies,
            access: dependencies.access,
            rooms: dependencies.rooms,
            directory: dependencies.directory,
            secrets: dependencies.secrets,
            clock: dependencies.clock,
            identifiers: dependencies.identifiers,
            pause: dependencies.pause,
            policy: dependencies.policy,
            matrix_server_name: dependencies.matrix_server_name,
        }
    }

    async fn create_internal(
        &self,
        request: CreateNetworkAgent,
    ) -> NetworkAgentResult<CreatedNetworkAgent> {
        if !self.policy.enabled {
            return Err(NetworkAgentFailure::new(NetworkAgentFailureKind::Disabled));
        }
        let name = NetworkAgentName::parse(&request.name)
            .map_err(|_| NetworkAgentFailure::new(NetworkAgentFailureKind::InvalidName))?;
        let (target, mut naming) = self
            .creation_target(request.room, &request.source_digest)
            .await?;
        if matches!(target, CreationTarget::Lobby(_)) {
            naming = Some(self.admit_to_lobbies(&request.source_digest).await?);
        }
        // 没建成就把这次占的起名名额退回去，不然连着失败几次，这个来源一小时就进不来了。
        let created = self
            .create_admitted(name, request.source_digest, target)
            .await;
        if created.is_err() {
            self.give_back(naming).await;
        }
        created
    }

    /// 名额已经占好、要进的房间也核对过：生成身份、占住名字、生效并进房间。
    async fn create_admitted(
        &self,
        name: NetworkAgentName,
        source_digest: [u8; 32],
        target: CreationTarget,
    ) -> NetworkAgentResult<CreatedNetworkAgent> {
        let now = self.clock.now();
        let id = NetworkAgentId::from_uuid(uuid::Uuid::now_v7());
        let token = self.secrets.generate().map_err(|_| internal())?;
        let device_key = self.keys.generate().map_err(|_| internal())?;
        let instance_key = self.keys.generate().map_err(|_| internal())?;
        let principal_id = self.identifiers.principal_id();
        let device = Device::register(
            self.identifiers.device_id(),
            principal_id,
            DEVICE_LABEL.to_owned(),
            DevicePlatform::Network,
            DevicePublicSigningKey::new(device_key.public_key.to_vec()).map_err(|_| internal())?,
            now,
        )
        .and_then(|mut device| device.verify().map(|()| device))
        .map_err(|_| internal())?;
        let secrets = vec![
            (
                NetworkAgentSecretKind::DeviceSigningSeed,
                self.seal(id, NetworkAgentSecretKind::DeviceSigningSeed, &device_key)?,
            ),
            (
                NetworkAgentSecretKind::InstanceSigningSeed,
                self.seal(
                    id,
                    NetworkAgentSecretKind::InstanceSigningSeed,
                    &instance_key,
                )?,
            ),
        ];

        // 名字先占住：没停用的网络 Agent 不重名（不分大小写），撞上了就加序号再试。
        let mut provisioning = NetworkAgentProvisioning {
            id,
            principal: crate::principal_projection::network_agent_registration(
                principal_id,
                &id.to_string(),
                name.as_str(),
                now,
                &self.matrix_server_name,
            ),
            device,
            device_label: DEVICE_LABEL.to_owned(),
            token_digest: self.secrets.digest(token.expose()),
            display_name: name.as_str().to_owned(),
            source_digest,
            secrets,
            created_at: now,
        };
        self.claim_name(&name, &mut provisioning).await?;

        // 从这里起失败时令牌不会交给 Agent，这条记录也就没用了：停用它，放开名字与全站名额。
        // 停用本身失败时记录留在 provisioning，仍占着名字，但不影响这次的结果。
        match self
            .activate_and_enter(&provisioning, instance_key.public_key, target, now)
            .await
        {
            Ok((agent_id, placement)) => Ok(CreatedNetworkAgent {
                network_agent_id: id,
                agent_id,
                display_name: provisioning.display_name,
                token,
                placement,
            }),
            Err(failure) => {
                let _released = self.store.disable(id, self.clock.now()).await;
                Err(failure)
            }
        }
    }

    /// 先一次查出同名的都用了哪些序号，取第一个空着的写入；写的时候又被别人抢先占了，
    /// 记下它再挑下一个。都占了就请 Agent 换个名字。
    async fn claim_name(
        &self,
        name: &NetworkAgentName,
        provisioning: &mut NetworkAgentProvisioning,
    ) -> NetworkAgentResult<()> {
        let mut taken: HashSet<String> = self
            .store
            .names_starting_with(&name.numbering_prefix(MAX_NAME_NUMBER))
            .await
            .map_err(repository)?
            .into_iter()
            .collect();
        for _ in 0..NAME_ATTEMPTS {
            let Some(number) = name.first_free_number(&taken, MAX_NAME_NUMBER) else {
                break;
            };
            let numbered = name.numbered(number);
            numbered.as_str().clone_into(&mut provisioning.display_name);
            provisioning
                .principal
                .display_name
                .clone_from(&provisioning.display_name);
            match self.store.begin(provisioning).await.map_err(repository)? {
                NetworkAgentBeginOutcome::Created => return Ok(()),
                NetworkAgentBeginOutcome::NameTaken => {
                    taken.insert(numbered.as_str().to_lowercase());
                }
            }
        }
        Err(NetworkAgentFailure::new(
            NetworkAgentFailureKind::NameUnavailable,
        ))
    }

    /// 以这台网络设备的身份建 Agent、登记实例、保存 Matrix 会话，生效后进大厅；
    /// 凭口令的只放行（记为私人房间的 Agent 成员），拿房间号的敲门。
    async fn activate_and_enter(
        &self,
        provisioning: &NetworkAgentProvisioning,
        instance_public_key: [u8; 32],
        target: CreationTarget,
        now: UtcMillis,
    ) -> NetworkAgentResult<(AgentId, NetworkAgentPlacement)> {
        let id = provisioning.id;
        let actor = network_actor(&provisioning.principal, provisioning.device.id(), now)?;
        // 两步都按网络 Agent 的 ID 幂等：重试回到同一个 Agent 与实例。
        let agent = self
            .agents
            .create_host_agent_for_device(CreateHostAgentForDevice {
                request_id: AgentCreationRequestId::from_uuid(id.as_uuid()),
                actor: actor.clone(),
                display_name: provisioning.display_name.clone(),
            })
            .await
            .map_err(|_| dependency())?;
        let instance = self
            .agents
            .register_instance(RegisterAgentInstance {
                request_id: AgentInstanceRegistrationRequestId::from_uuid(id.as_uuid()),
                actor: actor.clone(),
                agent_id: agent.agent.id(),
                adapter_type: ADAPTER_TYPE.to_owned(),
                external_subject_hash: None,
                capability_version: CAPABILITY_VERSION.to_owned(),
                configuration: Map::new(),
                public_signing_key: AgentInstancePublicSigningKey::new(
                    instance_public_key.to_vec(),
                )
                .map_err(|_| internal())?,
            })
            .await
            .map_err(|_| dependency())?;
        let agent_instance_id = instance.registration.instance.id();
        let matrix_access_token = self
            .sealer
            .seal(
                id,
                NetworkAgentSecretKind::MatrixAccessToken,
                instance.matrix_session.access_token().expose().as_bytes(),
            )
            .map_err(|_| dependency())?;
        self.store
            .activate(&NetworkAgentActivation {
                id,
                agent_id: agent.agent.id(),
                agent_instance_id,
                matrix_access_token,
                activated_at: self.clock.now(),
            })
            .await
            .map_err(repository)?;

        let agent_id = agent.agent.id();
        let placement = match target {
            CreationTarget::Lobby(catalog) => {
                let room = self
                    .enter_room(&actor, agent_id, agent_instance_id, catalog, None)
                    .await?;
                self.record_room(id, &room).await?;
                NetworkAgentPlacement::Entered(room)
            }
            CreationTarget::Private(code) => {
                NetworkAgentPlacement::Admitted(self.redeem(actor, agent_id, code).await?)
            }
            CreationTarget::Knock(catalog_id) => {
                match self.knock(actor, agent_id, catalog_id, now).await? {
                    NetworkAgentAdmission::Admitted(NetworkAgentTarget::Private(room)) => {
                        NetworkAgentPlacement::Admitted(room)
                    }
                    NetworkAgentAdmission::Knocked(knock) => NetworkAgentPlacement::Knocked(knock),
                    NetworkAgentAdmission::AlreadyIn(_)
                    | NetworkAgentAdmission::Admitted(NetworkAgentTarget::Lobby(_)) => {
                        return Err(internal());
                    }
                }
            }
        };
        Ok((agent_id, placement))
    }

    /// 要进的房间先核对好：大厅要存在，口令要对（猜错计在来源上），敲门的私人房间要在使用中
    /// （敲门计在来源上）。口令是网络 Agent 建的房间的，对了还要占起名的名额。
    /// 占了起名名额的（网络 Agent 建的房间的口令）一起交回去，没建成时退回。
    async fn creation_target(
        &self,
        room: NetworkAgentRoomRequest,
        source_digest: &[u8; 32],
    ) -> NetworkAgentResult<(CreationTarget, Option<NamingTicket>)> {
        match room {
            NetworkAgentRoomRequest::Lobby(room) => match self.place(room.as_deref()).await? {
                Place::Lobby(catalog) => Ok((CreationTarget::Lobby(catalog), None)),
                Place::Private(catalog) => {
                    self.access
                        .knockable(catalog)
                        .await
                        .map_err(knock_failure)?;
                    self.take_knock(source_digest).await?;
                    Ok((CreationTarget::Knock(catalog), None))
                }
            },
            NetworkAgentRoomRequest::Code(code) => {
                let room = self.resolve_code(&code, source_digest).await?;
                let naming = if self
                    .store
                    .owned_by_network_agent(room.catalog_id)
                    .await
                    .map_err(repository)?
                {
                    Some(self.take_naming(source_digest).await?)
                } else {
                    None
                };
                Ok((CreationTarget::Private(code), naming))
            }
        }
    }

    async fn admit_internal(
        &self,
        token: &str,
        room: NetworkAgentRoomRequest,
        source_digest: [u8; 32],
    ) -> NetworkAgentResult<NetworkAgentAdmission> {
        let record = self.authenticated(token).await?;
        let agent_id = record.agent_id.ok_or_else(internal)?;
        let now = self.clock.now();
        self.store
            .record_activity(record.id, now)
            .await
            .map_err(repository)?;
        let away = self.return_to_lobbies(&record).await?;
        match room {
            NetworkAgentRoomRequest::Lobby(room) => {
                let place = self.place(room.as_deref()).await?;
                let (Place::Lobby(catalog) | Place::Private(catalog)) = place;
                let rooms = self.store.rooms(record.id).await.map_err(repository)?;
                if let Some(room) = rooms.iter().find(|room| room.catalog_id == catalog) {
                    return Ok(NetworkAgentAdmission::AlreadyIn(
                        self.named(room.clone()).await?,
                    ));
                }
                match place {
                    Place::Lobby(catalog) => {
                        // 已经在别的公开大厅里的换个大厅不再算；从私人房间头一回来大厅的要算。
                        // 闲置时请出、这次大厅满了没回去的，只看大厅满没满，不占名额。
                        if !self.in_public_lobby(&rooms).await? {
                            if away.is_empty() {
                                self.admit_to_lobbies(&source_digest).await?;
                            } else if self.lobbies_full(&self.public_lobby_ids().await?).await? {
                                return Err(NetworkAgentFailure::new(
                                    NetworkAgentFailureKind::CapacityReached,
                                ));
                            }
                        }
                        Ok(NetworkAgentAdmission::Admitted(NetworkAgentTarget::Lobby(
                            catalog,
                        )))
                    }
                    Place::Private(catalog) => {
                        self.take_knock(&source_digest).await?;
                        self.knock(self.actor_of(&record, now)?, agent_id, catalog, now)
                            .await
                    }
                }
            }
            NetworkAgentRoomRequest::Code(code) => {
                // 先按来源核对，猜错计在来源上；对了再以它自己的网络设备兑换。已经在里面时
                // 兑换与之后的进入都是幂等的。
                self.resolve_code(&code, &source_digest).await?;
                let room = self
                    .redeem(self.actor_of(&record, now)?, agent_id, code)
                    .await?;
                Ok(NetworkAgentAdmission::Admitted(
                    NetworkAgentTarget::Private(room),
                ))
            }
        }
    }

    async fn enter_internal(
        &self,
        token: &str,
        target: NetworkAgentTarget,
    ) -> NetworkAgentResult<NetworkAgentRoom> {
        let record = self.authenticated(token).await?;
        self.enter_as(&record, target).await
    }

    async fn enter_admitted_internal(
        &self,
        agent_id: AgentId,
        room: NetworkAgentRoom,
    ) -> NetworkAgentResult<NetworkAgentRoom> {
        let record = self.active_by_agent(agent_id).await?;
        self.enter_as(&record, NetworkAgentTarget::Private(room))
            .await
    }

    /// 以这个网络 Agent 的网络设备进房间，并记下来。
    async fn enter_as(
        &self,
        record: &NetworkAgentRecord,
        target: NetworkAgentTarget,
    ) -> NetworkAgentResult<NetworkAgentRoom> {
        let (Some(agent_id), Some(agent_instance_id)) = (record.agent_id, record.agent_instance_id)
        else {
            return Err(internal());
        };
        let actor = self.actor_of(record, self.clock.now())?;
        let room = match target {
            NetworkAgentTarget::Lobby(catalog) => {
                self.enter_room(&actor, agent_id, agent_instance_id, catalog, None)
                    .await?
            }
            NetworkAgentTarget::Private(room) => {
                let entered = self
                    .enter_room(
                        &actor,
                        agent_id,
                        agent_instance_id,
                        room.catalog_id,
                        Some(room.matrix_room_id.clone()),
                    )
                    .await?;
                NetworkAgentRoom {
                    name: room.name,
                    ..entered
                }
            }
        };
        self.record_room(record.id, &room).await?;
        Ok(room)
    }

    async fn record_room(
        &self,
        id: NetworkAgentId,
        room: &NetworkAgentRoom,
    ) -> NetworkAgentResult<()> {
        self.store
            .record_room(
                id,
                &NetworkAgentRoomRecord {
                    catalog_id: room.catalog_id,
                    matrix_room_id: room.matrix_room_id.clone(),
                    joined_at: self.clock.now(),
                    name: None,
                },
            )
            .await
            .map_err(repository)
    }

    /// 以这个网络 Agent 的网络设备敲门：已经是 Agent 成员的直接放行。
    async fn knock(
        &self,
        actor: AuthenticatedDevice,
        agent_id: AgentId,
        catalog_id: RoomCatalogId,
        now: UtcMillis,
    ) -> NetworkAgentResult<NetworkAgentAdmission> {
        let result = self
            .access
            .knock(KnockOnRoom {
                actor,
                agent_id,
                catalog_id,
            })
            .await
            .map_err(knock_failure)?;
        Ok(match result {
            KnockResult::Member(room) => {
                NetworkAgentAdmission::Admitted(NetworkAgentTarget::Private(private_room(room)))
            }
            KnockResult::Waiting(record) | KnockResult::Declined(record) => {
                NetworkAgentAdmission::Knocked(knock_view(&record, now).ok_or_else(internal)?)
            }
        })
    }

    /// 敲门按来源计数：每小时有上限，超了告诉 Agent 什么时候能再敲。
    async fn take_knock(&self, source: &[u8; 32]) -> NetworkAgentResult<()> {
        self.take(
            &format!("knock:{}", hex(source)),
            3_600_000,
            self.policy.knocks_per_source_per_hour,
        )
        .await
    }

    /// 核对口令：猜错按来源计数，与本机设备分开计。回答口令是哪个房间的。
    async fn resolve_code(
        &self,
        code: &str,
        source_digest: &[u8; 32],
    ) -> NetworkAgentResult<RedeemedRoom> {
        self.access
            .resolve(ResolveJoinCode {
                caller: JoinCodeCaller::NetworkSource(*source_digest),
                code: code.to_owned(),
            })
            .await
            .map_err(code_failure)
    }

    /// 凭口令把这个 Agent 记为私人房间的 Agent 成员；还没进房间。
    async fn redeem(
        &self,
        actor: AuthenticatedDevice,
        agent_id: AgentId,
        code: String,
    ) -> NetworkAgentResult<NetworkAgentRoom> {
        self.access
            .redeem(RedeemJoinCode {
                actor,
                agent_id,
                code,
            })
            .await
            .map(private_room)
            .map_err(code_failure)
    }

    /// 这个网络 Agent 的网络设备：代它调用本机 Bridge 走的那些用例，只活在这一次请求里。
    fn actor_of(
        &self,
        record: &NetworkAgentRecord,
        now: UtcMillis,
    ) -> NetworkAgentResult<AuthenticatedDevice> {
        let registration = crate::principal_projection::network_agent_registration(
            record.principal_id,
            &record.id.to_string(),
            &record.display_name,
            record.created_at,
            &self.matrix_server_name,
        );
        network_actor(&registration, record.device_id, now)
    }

    /// 带上目录里的房间名。
    async fn named(&self, room: NetworkAgentRoomRecord) -> NetworkAgentResult<NetworkAgentRoom> {
        let name = self
            .directory
            .find_catalog(room.catalog_id)
            .await
            .map_err(repository)?
            .map_or_else(String::new, |catalog| catalog.name().to_owned());
        Ok(NetworkAgentRoom {
            catalog_id: room.catalog_id,
            matrix_room_id: room.matrix_room_id,
            name,
        })
    }

    async fn me_internal(&self, token: &str) -> NetworkAgentResult<NetworkAgentView> {
        let record = self.authenticated(token).await?;
        let agent_id = record.agent_id.ok_or_else(internal)?;
        self.store
            .record_activity(record.id, self.clock.now())
            .await
            .map_err(repository)?;
        self.return_to_lobbies(&record).await?;
        let mut rooms = Vec::new();
        for room in self.store.rooms(record.id).await.map_err(repository)? {
            rooms.push(self.named(room).await?);
        }
        let now = self.clock.now();
        let knocks = self
            .access
            .knocks_of(agent_id)
            .await
            .map_err(|_| dependency())?
            .iter()
            .filter(|knock| !rooms.iter().any(|room| room.catalog_id == knock.catalog_id))
            .filter_map(|knock| knock_view(knock, now))
            .collect();
        let incoming_knocks = self.incoming_knocks_of(&record).await?.knocks;
        Ok(NetworkAgentView {
            network_agent_id: record.id,
            agent_id,
            display_name: record.display_name,
            created_at: record.created_at,
            rooms,
            knocks,
            incoming_knocks,
        })
    }

    async fn session_internal(&self, token: &str) -> NetworkAgentResult<NetworkAgentSession> {
        let record = self.authenticated(token).await?;
        self.store
            .record_activity(record.id, self.clock.now())
            .await
            .map_err(repository)?;
        let away = self.return_to_lobbies(&record).await?;
        self.open_session(record, away).await
    }

    async fn exit_session_internal(&self, token: &str) -> NetworkAgentResult<NetworkAgentSession> {
        let record = self.authenticated(token).await?;
        self.open_session(record, Vec::new()).await
    }

    async fn session_of_agent_internal(
        &self,
        agent_id: AgentId,
    ) -> NetworkAgentResult<NetworkAgentSession> {
        let record = self.active_by_agent(agent_id).await?;
        let away = self.lobbies_away(record.id).await?;
        self.open_session(record, away).await
    }

    /// 闲置时请出的公开大厅，这次调接口时先替它回去（`specs/launch-capacity/design.md` 第 2 步）：
    /// 原来的分片还在、放得下就回原来的，不然由大厅分配；不占来源的名额。大厅里的网络 Agent 满了、
    /// 或者一时进不去时，这次调用照常做，返回还没回去的，下次调接口再试。
    async fn return_to_lobbies(
        &self,
        record: &NetworkAgentRecord,
    ) -> NetworkAgentResult<Vec<NetworkAgentLobbyAway>> {
        let away = self.lobbies_away(record.id).await?;
        if away.is_empty() {
            return Ok(away);
        }
        let (Some(agent_id), Some(agent_instance_id)) = (record.agent_id, record.agent_instance_id)
        else {
            return Err(internal());
        };
        let actor = self.actor_of(record, self.clock.now())?;
        let lobbies = self.public_lobby_ids().await?;
        let mut remaining = Vec::new();
        for lobby in away {
            if self.lobbies_full(&lobbies).await? {
                remaining.push(lobby);
                continue;
            }
            let entered = match self
                .enter_room(
                    &actor,
                    agent_id,
                    agent_instance_id,
                    lobby.catalog_id,
                    Some(lobby.matrix_room_id.clone()),
                )
                .await
            {
                Ok(room) => Ok(room),
                // 原来的分片满了或者不在了：由大厅分配。
                Err(_) => {
                    self.enter_room(&actor, agent_id, agent_instance_id, lobby.catalog_id, None)
                        .await
                }
            };
            match entered {
                Ok(room) => self.record_room(record.id, &room).await?,
                Err(_) => remaining.push(lobby),
            }
        }
        Ok(remaining)
    }

    /// 闲置时请出、还没回去的公开大厅；大厅已经不在目录里的不算。
    async fn lobbies_away(
        &self,
        id: NetworkAgentId,
    ) -> NetworkAgentResult<Vec<NetworkAgentLobbyAway>> {
        let away = self.store.lobbies_away(id).await.map_err(repository)?;
        if away.is_empty() {
            return Ok(away);
        }
        let lobbies = self.public_lobby_ids().await?;
        Ok(away
            .into_iter()
            .filter(|lobby| lobbies.contains(&lobby.catalog_id))
            .collect())
    }

    /// 生效中的网络 Agent：按 Agent 找，不是网络 Agent 或已停用时回答未认证。
    async fn active_by_agent(&self, agent_id: AgentId) -> NetworkAgentResult<NetworkAgentRecord> {
        if !self.policy.enabled {
            return Err(NetworkAgentFailure::new(NetworkAgentFailureKind::Disabled));
        }
        self.store
            .find_by_agent(agent_id)
            .await
            .map_err(repository)?
            .filter(|record| record.status == NetworkAgentStatus::Active)
            .ok_or_else(|| NetworkAgentFailure::new(NetworkAgentFailureKind::Unauthorized))
    }

    /// 解开封存的 Matrix 会话与签名种子，连同所在的房间交给网关。
    async fn open_session(
        &self,
        record: NetworkAgentRecord,
        lobbies_away: Vec<NetworkAgentLobbyAway>,
    ) -> NetworkAgentResult<NetworkAgentSession> {
        let (Some(agent_id), Some(agent_instance_id)) = (record.agent_id, record.agent_instance_id)
        else {
            return Err(internal());
        };
        let matrix_access_token = self
            .open_secret(record.id, NetworkAgentSecretKind::MatrixAccessToken)
            .await?;
        let instance_signing_seed = self
            .open_secret(record.id, NetworkAgentSecretKind::InstanceSigningSeed)
            .await?;
        let rooms = self.store.rooms(record.id).await.map_err(repository)?;
        Ok(self.session_of(
            record,
            (agent_id, agent_instance_id),
            matrix_access_token,
            instance_signing_seed,
            (rooms, lobbies_away),
        ))
    }

    fn session_of(
        &self,
        record: NetworkAgentRecord,
        (agent_id, agent_instance_id): (AgentId, AgentInstanceId),
        matrix_access_token: SecretValue,
        instance_signing_seed: SecretValue,
        (rooms, lobbies_away): (Vec<NetworkAgentRoomRecord>, Vec<NetworkAgentLobbyAway>),
    ) -> NetworkAgentSession {
        NetworkAgentSession {
            network_agent_id: record.id,
            principal_id: record.principal_id,
            agent_id,
            agent_instance_id,
            display_name: record.display_name,
            agent_matrix_user_id: format!(
                "@{}:{}",
                MatrixAgentLocalpart::from_agent_id(agent_id).as_str(),
                self.matrix_server_name
            ),
            matrix_access_token,
            instance_signing_seed,
            rooms,
            matrix_device_id: record
                .matrix_device_id
                .unwrap_or_else(|| crate::agents::instance_matrix_device_id(agent_instance_id)),
            encrypted_since: record.encrypted_since,
            lobbies_away,
        }
    }

    async fn encryption_secrets_internal(
        &self,
        id: NetworkAgentId,
    ) -> NetworkAgentResult<NetworkAgentEncryptionSecrets> {
        let store_passphrase = self
            .generated_secret(id, NetworkAgentSecretKind::MatrixStorePassphrase)
            .await?;
        let content_root_key = self
            .generated_secret(id, NetworkAgentSecretKind::MessageContentRootKey)
            .await?;
        let recovery_credential = match self
            .read_secret(id, NetworkAgentSecretKind::MatrixRecoveryKey)
            .await?
        {
            OpenedSecret::Secret(secret) => Some(secret),
            OpenedSecret::Missing => None,
            OpenedSecret::Unsealable => return Err(dependency()),
        };
        Ok(NetworkAgentEncryptionSecrets {
            store_passphrase,
            content_root_key,
            recovery_credential,
        })
    }

    async fn replace_matrix_device_internal(
        &self,
        id: NetworkAgentId,
    ) -> NetworkAgentResult<NetworkAgentMatrixDevice> {
        let record = self
            .store
            .find(id)
            .await
            .map_err(repository)?
            .filter(|record| record.status == NetworkAgentStatus::Active)
            .ok_or_else(|| NetworkAgentFailure::new(NetworkAgentFailureKind::Unauthorized))?;
        let instance_id = record.agent_instance_id.ok_or_else(internal)?;
        let rotated = self
            .agents
            .replace_instance_matrix_device(RotateAgentInstanceMatrixSession {
                actor: self.actor_of(&record, self.clock.now())?,
                instance_id,
            })
            .await
            .map_err(|_| dependency())?;
        let access_token = rotated.matrix_session.access_token().clone();
        self.put_text_secret(id, NetworkAgentSecretKind::MatrixAccessToken, &access_token)
            .await?;
        Ok(NetworkAgentMatrixDevice {
            device_id: rotated
                .matrix_session
                .metadata()
                .device_id()
                .as_str()
                .to_owned(),
            access_token,
        })
    }

    /// 读一个第一次要用时才生成的秘密：缺了就生成并封存；在库里却解不开时报依赖不可用，
    /// 绝不重新生成去覆盖。
    async fn generated_secret(
        &self,
        id: NetworkAgentId,
        kind: NetworkAgentSecretKind,
    ) -> NetworkAgentResult<SecretValue> {
        match self.read_secret(id, kind).await? {
            OpenedSecret::Secret(secret) => Ok(secret),
            OpenedSecret::Missing => {
                let secret = self.secrets.generate().map_err(|_| dependency())?;
                self.put_text_secret(id, kind, &secret).await?;
                Ok(secret)
            }
            OpenedSecret::Unsealable => Err(dependency()),
        }
    }

    async fn put_text_secret(
        &self,
        id: NetworkAgentId,
        kind: NetworkAgentSecretKind,
        secret: &SecretValue,
    ) -> NetworkAgentResult<()> {
        let sealed = self
            .sealer
            .seal(id, kind, secret.expose().as_bytes())
            .map_err(|_| dependency())?;
        self.store
            .put_secret(id, kind, &sealed, self.clock.now())
            .await
            .map_err(repository)
    }

    /// 取出并解封一个秘密；缺失说明记录不完整，解不开说明密钥或密文出了问题，都不是令牌的错。
    async fn open_secret(
        &self,
        id: NetworkAgentId,
        kind: NetworkAgentSecretKind,
    ) -> NetworkAgentResult<SecretValue> {
        match self.read_secret(id, kind).await? {
            OpenedSecret::Secret(secret) => Ok(secret),
            OpenedSecret::Missing => Err(internal()),
            OpenedSecret::Unsealable => Err(dependency()),
        }
    }

    /// 库不可用是错误；秘密缺失或解不开如实说明，由调用方决定怎么办。
    async fn read_secret(
        &self,
        id: NetworkAgentId,
        kind: NetworkAgentSecretKind,
    ) -> NetworkAgentResult<OpenedSecret> {
        let Some(sealed) = self.store.find_secret(id, kind).await.map_err(repository)? else {
            return Ok(OpenedSecret::Missing);
        };
        Ok(self
            .sealer
            .open(id, kind, &sealed)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|text| SecretValue::new(text).ok())
            .map_or(OpenedSecret::Unsealable, OpenedSecret::Secret))
    }

    async fn disable_stale_internal(&self) -> NetworkAgentResult<usize> {
        let now = self.clock.now();
        let before = |millis: i64| UtcMillis::new(now.value() - millis).map_err(|_| internal());
        let cutoff = NetworkAgentStaleCutoff {
            idle_before: before(IDLE_LIFETIME_MILLIS)?,
            provisioning_before: before(PROVISIONING_TIMEOUT_MILLIS)?,
        };
        let disabled = self
            .store
            .disable_stale(cutoff, now, STALE_BATCH)
            .await
            .map_err(repository)?;
        Ok(disabled.len())
    }

    async fn pending_exits_internal(
        &self,
        limit: u32,
    ) -> NetworkAgentResult<Vec<NetworkAgentPendingExit>> {
        let records = self.store.pending_exits(limit).await.map_err(repository)?;
        let mut exits = Vec::with_capacity(records.len());
        for record in records {
            exits.push(self.pending_exit(record).await?);
        }
        Ok(exits)
    }

    async fn pending_exit(
        &self,
        record: NetworkAgentRecord,
    ) -> NetworkAgentResult<NetworkAgentPendingExit> {
        let unopenable = NetworkAgentPendingExit::Unopenable {
            id: record.id,
            principal_id: record.principal_id,
        };
        let (Some(agent_id), Some(agent_instance_id)) = (record.agent_id, record.agent_instance_id)
        else {
            return Ok(unopenable);
        };
        let id = record.id;
        let OpenedSecret::Secret(matrix_access_token) = self
            .read_secret(id, NetworkAgentSecretKind::MatrixAccessToken)
            .await?
        else {
            return Ok(unopenable);
        };
        let OpenedSecret::Secret(instance_signing_seed) = self
            .read_secret(id, NetworkAgentSecretKind::InstanceSigningSeed)
            .await?
        else {
            return Ok(unopenable);
        };
        let rooms = self.store.rooms(id).await.map_err(repository)?;
        // 停用了，不再回大厅。
        Ok(NetworkAgentPendingExit::Session(Box::new(self.session_of(
            record,
            (agent_id, agent_instance_id),
            matrix_access_token,
            instance_signing_seed,
            (rooms, Vec::new()),
        ))))
    }

    async fn idle_lobby_exits_internal(
        &self,
        limit: u32,
    ) -> NetworkAgentResult<Vec<NetworkAgentLobbyExit>> {
        let idle_before =
            UtcMillis::new(self.clock.now().value() - LOBBY_IDLE_MILLIS).map_err(|_| internal())?;
        let idle = self
            .store
            .idle_in_lobbies(idle_before, limit)
            .await
            .map_err(repository)?;
        let mut exits = Vec::with_capacity(idle.len());
        for NetworkAgentIdleLobby { id, room } in idle {
            let matrix_access_token = match self
                .read_secret(id, NetworkAgentSecretKind::MatrixAccessToken)
                .await?
            {
                OpenedSecret::Secret(token) => Some(token),
                OpenedSecret::Missing | OpenedSecret::Unsealable => None,
            };
            exits.push(NetworkAgentLobbyExit {
                id,
                room,
                matrix_access_token,
            });
        }
        Ok(exits)
    }

    async fn take_message_quota_internal(
        &self,
        id: NetworkAgentId,
        catalog_id: RoomCatalogId,
    ) -> NetworkAgentResult<()> {
        if !self.policy.enabled {
            return Err(NetworkAgentFailure::new(NetworkAgentFailureKind::Disabled));
        }
        if !self.is_public_lobby(catalog_id).await? {
            return Ok(());
        }
        let agent = id.as_uuid().simple().to_string();
        for (window, limit, label) in [
            (60_000_u64, self.policy.lobby_messages_per_minute, "minute"),
            (86_400_000_u64, self.policy.lobby_messages_per_day, "day"),
        ] {
            self.take(&format!("send:{label}:{agent}"), window, limit)
                .await?;
        }
        // 整个大厅合起来的放在最后：自己已经超了的，别再去占大厅的份额。
        self.take(
            &format!("lobby-send:minute:{}", catalog_id.as_uuid().simple()),
            60_000,
            self.policy.lobby_total_messages_per_minute,
        )
        .await
    }

    /// 在一个固定窗口里记一次；到上限时告诉调用方窗口什么时候结束。
    async fn take(&self, bucket: &str, window: u64, limit: u32) -> NetworkAgentResult<()> {
        let policy = RateWindowPolicy {
            window: DurationMillis::new(window).map_err(|_| internal())?,
            limit,
        };
        match self
            .store
            .take(bucket, self.clock.now(), policy)
            .await
            .map_err(repository)?
        {
            RateWindowDecision::Allowed => Ok(()),
            RateWindowDecision::Limited { retry_at } => {
                Err(NetworkAgentFailure::rate_limited(retry_at))
            }
        }
    }

    async fn disable_internal(&self, token: &str) -> NetworkAgentResult<()> {
        let record = self.authenticated(token).await?;
        self.store
            .disable(record.id, self.clock.now())
            .await
            .map_err(repository)
    }

    /// 只认生效中的网络 Agent；总开关关着时一律拒绝，哪怕没带令牌。
    async fn authenticated(&self, token: &str) -> NetworkAgentResult<NetworkAgentRecord> {
        if !self.policy.enabled {
            return Err(NetworkAgentFailure::new(NetworkAgentFailureKind::Disabled));
        }
        if token.is_empty() {
            return Err(NetworkAgentFailure::new(
                NetworkAgentFailureKind::Unauthorized,
            ));
        }
        let digest = self.secrets.digest(token);
        self.store
            .find_by_token(&digest)
            .await
            .map_err(repository)?
            .filter(|record| record.status == NetworkAgentStatus::Active)
            .ok_or_else(|| NetworkAgentFailure::new(NetworkAgentFailureKind::Unauthorized))
    }

    async fn public_lobbies_internal(&self) -> NetworkAgentResult<Vec<NetworkAgentLobby>> {
        if !self.policy.enabled {
            return Err(NetworkAgentFailure::new(NetworkAgentFailureKind::Disabled));
        }
        let lobbies = self
            .directory
            .list_public(&RoomDirectoryQuery::default())
            .await
            .map_err(repository)?;
        let default = lobbies
            .iter()
            .find(|entry| entry.catalog.slug().map(RoomSlug::as_str) == Some(DEFAULT_LOBBY_SLUG))
            .or_else(|| lobbies.first())
            .map(|entry| entry.catalog.id());
        Ok(lobbies
            .iter()
            .map(|entry| NetworkAgentLobby {
                name: entry.catalog.name().to_owned(),
                slug: entry.catalog.slug().map(|slug| slug.as_str().to_owned()),
                online_agent_count: entry.online_agent_count,
                default: Some(entry.catalog.id()) == default,
            })
            .collect())
    }

    /// 认出要进的房间：房间号是公开大厅的就进那间，不是的当作私人房间（敲门时再核对）；别的按公开
    /// 大厅的名字或 slug 找。
    async fn place(&self, wanted: Option<&str>) -> NetworkAgentResult<Place> {
        let Some(catalog) = wanted.and_then(parse_room_number) else {
            return self.lobby(wanted).await.map(Place::Lobby);
        };
        let lobbies = self
            .directory
            .list_public(&RoomDirectoryQuery::default())
            .await
            .map_err(repository)?;
        Ok(
            if lobbies.iter().any(|entry| entry.catalog.id() == catalog) {
                Place::Lobby(catalog)
            } else {
                Place::Private(catalog)
            },
        )
    }

    /// 按名字或 slug 找公开大厅：先精确，再忽略大小写；不给名字就是默认公开大厅。
    async fn lobby(&self, wanted: Option<&str>) -> NetworkAgentResult<RoomCatalogId> {
        let lobbies = self
            .directory
            .list_public(&RoomDirectoryQuery::default())
            .await
            .map_err(repository)?;
        let wanted = wanted.map(str::trim).filter(|name| !name.is_empty());
        let found = match wanted {
            None => lobbies
                .iter()
                .find(|entry| {
                    entry.catalog.slug().map(RoomSlug::as_str) == Some(DEFAULT_LOBBY_SLUG)
                })
                .or_else(|| lobbies.first()),
            Some(name) => lobbies
                .iter()
                .find(|entry| {
                    entry.catalog.name() == name
                        || entry.catalog.slug().map(RoomSlug::as_str) == Some(name)
                })
                .or_else(|| {
                    let lowered = name.to_lowercase();
                    let mut matches = lobbies.iter().filter(|entry| {
                        entry.catalog.name().to_lowercase() == lowered
                            || entry
                                .catalog
                                .slug()
                                .is_some_and(|slug| slug.as_str().to_lowercase() == lowered)
                    });
                    let first = matches.next();
                    // 忽略大小写后仍有多间同名时不猜，当作找不到并列出候选。
                    first.filter(|_| matches.next().is_none())
                }),
        };
        found.map(|entry| entry.catalog.id()).ok_or_else(|| {
            NetworkAgentFailure::room_not_found(
                lobbies
                    .iter()
                    .map(|entry| entry.catalog.name().to_owned())
                    .collect(),
            )
        })
    }

    /// 让一个网络 Agent 进公开大厅：大厅里的网络 Agent 满了就请它过一会儿再来，没满才占起名的
    /// 名额。满了不扣名额：一直重试“大厅满了”的来源不该一小时后连空出来的位置都进不来。
    async fn admit_to_lobbies(&self, source: &[u8; 32]) -> NetworkAgentResult<NamingTicket> {
        if self.lobbies_full(&self.public_lobby_ids().await?).await? {
            return Err(NetworkAgentFailure::new(
                NetworkAgentFailureKind::CapacityReached,
            ));
        }
        self.take_naming(source).await
    }

    /// 公开大厅里的网络 Agent 到没到上限。闲置时请出的不算。
    async fn lobbies_full(&self, lobbies: &[RoomCatalogId]) -> NetworkAgentResult<bool> {
        let in_lobbies = self
            .store
            .count_live_in(lobbies)
            .await
            .map_err(repository)?;
        Ok(in_lobbies >= self.policy.max_lobby_agents)
    }

    /// 起名的名额：同一来源每小时、每天各有上限，超过时告诉 Agent 什么时候能再试。进公开大厅和
    /// 凭网络 Agent 建的房间的口令新起网络 Agent 共用。计数的桶沿用 `create:` 这个名字（以前
    /// 一起名就算一次）。两档一起记：有一档不够就都不记。
    async fn take_naming(&self, source: &[u8; 32]) -> NetworkAgentResult<NamingTicket> {
        let source = hex(source);
        let mut buckets = Vec::with_capacity(2);
        for (window, limit, label) in [
            (
                3_600_000_u64,
                self.policy.lobby_entries_per_source_per_hour,
                "hour",
            ),
            (
                86_400_000_u64,
                self.policy.lobby_entries_per_source_per_day,
                "day",
            ),
        ] {
            buckets.push(RateWindowBucket {
                bucket: format!("create:{label}:{source}"),
                policy: RateWindowPolicy {
                    window: DurationMillis::new(window).map_err(|_| internal())?,
                    limit,
                },
            });
        }
        let taken_at = self.clock.now();
        match self
            .store
            .take_all(&buckets, taken_at)
            .await
            .map_err(repository)?
        {
            RateWindowDecision::Allowed => Ok(NamingTicket {
                buckets: buckets.into_iter().map(|bucket| bucket.bucket).collect(),
                taken_at,
            }),
            RateWindowDecision::Limited { retry_at } => {
                Err(NetworkAgentFailure::rate_limited(retry_at))
            }
        }
    }

    /// 退回占过的起名名额。退不成只是这个来源少一次，不影响这次的结果。
    async fn give_back(&self, naming: Option<NamingTicket>) {
        if let Some(naming) = naming {
            let _ignored = self.store.give_back(&naming.buckets, naming.taken_at).await;
        }
    }

    async fn public_lobby_ids(&self) -> NetworkAgentResult<Vec<RoomCatalogId>> {
        Ok(self
            .directory
            .list_public(&RoomDirectoryQuery::default())
            .await
            .map_err(repository)?
            .into_iter()
            .map(|entry| entry.catalog.id())
            .collect())
    }

    /// 这个房间是不是公开大厅：大厅的各个分片都记在大厅自己的目录条目下。
    async fn is_public_lobby(&self, catalog_id: RoomCatalogId) -> NetworkAgentResult<bool> {
        Ok(self
            .directory
            .find_catalog(catalog_id)
            .await
            .map_err(repository)?
            .is_some_and(|catalog| catalog.kind() == RoomCatalogKind::PublicLobby))
    }

    async fn in_public_lobby(&self, rooms: &[NetworkAgentRoomRecord]) -> NetworkAgentResult<bool> {
        for room in rooms {
            if self.is_public_lobby(room.catalog_id).await? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// 进公开大厅（由大厅分配），或指名进私人房间。大厅正在准备新房间时按服务器给的时间等一会儿再试。
    async fn enter_room(
        &self,
        actor: &AuthenticatedDevice,
        agent_id: AgentId,
        agent_instance_id: AgentInstanceId,
        catalog_id: RoomCatalogId,
        target_room: Option<MatrixRoomReference>,
    ) -> NetworkAgentResult<NetworkAgentRoom> {
        let mut catalog_id = catalog_id;
        for _ in 0..MAX_LOBBY_ATTEMPTS {
            let outcome = self
                .lobbies
                .enter(EnterAgentLobby {
                    actor: actor.clone(),
                    agent_id,
                    agent_instance_id,
                    catalog_id,
                    preferred_language: None,
                    preferred_region: None,
                    target_room: target_room.clone(),
                })
                .await
                .map_err(|_| dependency())?;
            match outcome {
                EnterLobbyOutcome::Joined { room, .. } => {
                    let name = self
                        .directory
                        .find_catalog(room.catalog_id())
                        .await
                        .map_err(repository)?
                        .map_or_else(String::new, |catalog| catalog.name().to_owned());
                    return Ok(NetworkAgentRoom {
                        catalog_id: room.catalog_id(),
                        matrix_room_id: room.matrix_room_id().clone(),
                        name,
                    });
                }
                EnterLobbyOutcome::ProvisioningBusy { retry_at } => {
                    self.pause.until(retry_at).await;
                }
                EnterLobbyOutcome::CapacityChanged {
                    catalog_id: changed,
                } => {
                    catalog_id = changed;
                }
            }
        }
        Err(dependency())
    }

    fn seal(
        &self,
        id: NetworkAgentId,
        kind: NetworkAgentSecretKind,
        key: &crate::ports::GeneratedSigningKey,
    ) -> NetworkAgentResult<crate::ports::SealedSecret> {
        self.sealer
            .seal(id, kind, key.encoded_seed.expose().as_bytes())
            .map_err(|_| dependency())
    }
}

impl NetworkAgentUseCases for NetworkAgentService {
    fn create(
        &self,
        request: CreateNetworkAgent,
    ) -> PortFuture<'_, NetworkAgentResult<CreatedNetworkAgent>> {
        Box::pin(self.create_internal(request))
    }

    fn me<'a>(&'a self, token: &'a str) -> PortFuture<'a, NetworkAgentResult<NetworkAgentView>> {
        Box::pin(self.me_internal(token))
    }

    fn disable<'a>(&'a self, token: &'a str) -> PortFuture<'a, NetworkAgentResult<()>> {
        Box::pin(self.disable_internal(token))
    }

    fn session<'a>(
        &'a self,
        token: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentSession>> {
        Box::pin(self.session_internal(token))
    }

    fn take_message_quota(
        &self,
        id: NetworkAgentId,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, NetworkAgentResult<()>> {
        Box::pin(self.take_message_quota_internal(id, catalog_id))
    }

    fn disable_stale(&self) -> PortFuture<'_, NetworkAgentResult<usize>> {
        Box::pin(self.disable_stale_internal())
    }

    fn pending_exits(
        &self,
        limit: u32,
    ) -> PortFuture<'_, NetworkAgentResult<Vec<NetworkAgentPendingExit>>> {
        Box::pin(self.pending_exits_internal(limit))
    }

    fn mark_rooms_left(&self, id: NetworkAgentId) -> PortFuture<'_, NetworkAgentResult<()>> {
        Box::pin(async move {
            self.store
                .mark_rooms_left(id, self.clock.now())
                .await
                .map_err(repository)
        })
    }

    fn pending_key_deletions(
        &self,
        limit: u32,
    ) -> PortFuture<'_, NetworkAgentResult<Vec<NetworkAgentId>>> {
        Box::pin(async move {
            self.store
                .pending_key_deletions(limit)
                .await
                .map_err(repository)
        })
    }

    fn idle_lobby_exits(
        &self,
        limit: u32,
    ) -> PortFuture<'_, NetworkAgentResult<Vec<NetworkAgentLobbyExit>>> {
        Box::pin(self.idle_lobby_exits_internal(limit))
    }

    fn mark_lobby_away<'a>(
        &'a self,
        id: NetworkAgentId,
        room: &'a NetworkAgentRoomRecord,
    ) -> PortFuture<'a, NetworkAgentResult<()>> {
        Box::pin(async move {
            self.store
                .mark_lobby_away(id, room, self.clock.now())
                .await
                .map_err(repository)
        })
    }

    fn exit_session<'a>(
        &'a self,
        token: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentSession>> {
        Box::pin(self.exit_session_internal(token))
    }

    fn delete_keys(&self, id: NetworkAgentId) -> PortFuture<'_, NetworkAgentResult<()>> {
        Box::pin(async move {
            self.store
                .delete_keys(id, self.clock.now())
                .await
                .map_err(repository)
        })
    }

    fn public_lobbies(&self) -> PortFuture<'_, NetworkAgentResult<Vec<NetworkAgentLobby>>> {
        Box::pin(self.public_lobbies_internal())
    }

    fn admit<'a>(
        &'a self,
        token: &'a str,
        room: NetworkAgentRoomRequest,
        source_digest: [u8; 32],
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentAdmission>> {
        Box::pin(self.admit_internal(token, room, source_digest))
    }

    fn enter<'a>(
        &'a self,
        token: &'a str,
        target: NetworkAgentTarget,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentRoom>> {
        Box::pin(self.enter_internal(token, target))
    }

    fn session_of_agent(
        &self,
        agent_id: AgentId,
    ) -> PortFuture<'_, NetworkAgentResult<NetworkAgentSession>> {
        Box::pin(self.session_of_agent_internal(agent_id))
    }

    fn enter_admitted(
        &self,
        agent_id: AgentId,
        room: NetworkAgentRoom,
    ) -> PortFuture<'_, NetworkAgentResult<NetworkAgentRoom>> {
        Box::pin(self.enter_admitted_internal(agent_id, room))
    }

    fn mark_encrypted(&self, id: NetworkAgentId) -> PortFuture<'_, NetworkAgentResult<UtcMillis>> {
        Box::pin(async move {
            self.store
                .mark_encrypted(id, self.clock.now())
                .await
                .map_err(repository)
        })
    }

    fn encryption_secrets(
        &self,
        id: NetworkAgentId,
    ) -> PortFuture<'_, NetworkAgentResult<NetworkAgentEncryptionSecrets>> {
        Box::pin(self.encryption_secrets_internal(id))
    }

    fn store_recovery_credential<'a>(
        &'a self,
        id: NetworkAgentId,
        credential: &'a SecretValue,
    ) -> PortFuture<'a, NetworkAgentResult<()>> {
        Box::pin(self.put_text_secret(id, NetworkAgentSecretKind::MatrixRecoveryKey, credential))
    }

    fn replace_matrix_device(
        &self,
        id: NetworkAgentId,
    ) -> PortFuture<'_, NetworkAgentResult<NetworkAgentMatrixDevice>> {
        Box::pin(self.replace_matrix_device_internal(id))
    }
}

/// 占过的起名名额：在哪几个窗口里、什么时候记的。没建成时按它退回。
struct NamingTicket {
    buckets: Vec<String>,
    taken_at: UtcMillis,
}

/// 读一个封存秘密的结果。
enum OpenedSecret {
    Secret(SecretValue),
    Missing,
    Unsealable,
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

/// 网络设备代表合成主体调用用例时的身份；只活在这一次请求里。
fn network_actor(
    principal: &PrincipalRegistration,
    device_id: DeviceId,
    now: UtcMillis,
) -> NetworkAgentResult<AuthenticatedDevice> {
    Ok(AuthenticatedDevice {
        account: PrincipalAccount {
            principal: Principal::new(principal.principal.id()),
            matrix_user_id: principal.matrix_user_id.clone(),
            display_name: principal.display_name.clone(),
            avatar_content_id: None,
            locale: principal.locale.clone(),
        },
        device_id,
        access_token_expires_at: now
            .checked_add(DurationMillis::new(ACTOR_LIFETIME_MILLIS).map_err(|_| internal())?)
            .map_err(|_| internal())?,
    })
}

/// 给 Agent 看的敲门：放进来了的不列（它已经在房间里），在等的过了期算作废。
fn knock_view(record: &PrivateRoomAgentKnockRecord, now: UtcMillis) -> Option<NetworkAgentKnock> {
    let status = match record.status {
        PrivateRoomAgentKnockStatus::Admitted => return None,
        PrivateRoomAgentKnockStatus::Declined => NetworkAgentKnockStatus::Declined,
        PrivateRoomAgentKnockStatus::Waiting if record.expires_at <= now => {
            NetworkAgentKnockStatus::Expired
        }
        PrivateRoomAgentKnockStatus::Waiting => NetworkAgentKnockStatus::Waiting,
    };
    Some(NetworkAgentKnock {
        catalog_id: record.catalog_id,
        status,
        knocked_at: record.knocked_at,
        expires_at: record.expires_at,
    })
}

/// 敲门的房间不对（不存在、不是私人房间、已归档）时，和找不到公开大厅一样回答。
fn knock_failure(failure: AgentAccessFailure) -> NetworkAgentFailure {
    match failure.kind() {
        AgentAccessFailureKind::NotFound
        | AgentAccessFailureKind::Conflict
        | AgentAccessFailureKind::InvalidRequest => NetworkAgentFailure::room_not_found(Vec::new()),
        AgentAccessFailureKind::RateLimited => failure
            .retry_at()
            .map_or_else(dependency, NetworkAgentFailure::rate_limited),
        AgentAccessFailureKind::DependencyUnavailable | AgentAccessFailureKind::UnknownCommit => {
            dependency()
        }
        AgentAccessFailureKind::Forbidden
        | AgentAccessFailureKind::Internal
        | AgentAccessFailureKind::NotRevealable => internal(),
    }
}

fn private_room(room: RedeemedRoom) -> NetworkAgentRoom {
    NetworkAgentRoom {
        catalog_id: room.catalog_id,
        matrix_room_id: room.matrix_room_id,
        name: room.name,
    }
}

/// 口令不对、已更换或已停用，房间不收 Agent，或被移出后拿旧口令再来，都只说口令无效。
fn code_failure(failure: AgentAccessFailure) -> NetworkAgentFailure {
    match failure.kind() {
        AgentAccessFailureKind::InvalidRequest
        | AgentAccessFailureKind::NotFound
        | AgentAccessFailureKind::Conflict
        | AgentAccessFailureKind::Forbidden => {
            NetworkAgentFailure::new(NetworkAgentFailureKind::CodeInvalid)
        }
        AgentAccessFailureKind::RateLimited => failure
            .retry_at()
            .map_or_else(dependency, NetworkAgentFailure::rate_limited),
        AgentAccessFailureKind::DependencyUnavailable | AgentAccessFailureKind::UnknownCommit => {
            dependency()
        }
        AgentAccessFailureKind::Internal | AgentAccessFailureKind::NotRevealable => internal(),
    }
}

fn repository(_error: RepositoryError) -> NetworkAgentFailure {
    dependency()
}

const fn dependency() -> NetworkAgentFailure {
    NetworkAgentFailure::new(NetworkAgentFailureKind::DependencyUnavailable)
}

const fn internal() -> NetworkAgentFailure {
    NetworkAgentFailure::new(NetworkAgentFailureKind::Internal)
}
