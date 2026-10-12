//! 网络 Agent 自己建房间、关房间（`specs/agent-rooms/design.md` 第 4 步）。
//!
//! 房主是网络 Agent 的合成主体，它的 Agent 按“房主的 Agent”进去（`admits_agent_of`）。建房和关房
//! 走私人房间的用例，操作者是网络 Agent（`PrivateRoomActor::NetworkAgent`）；建房时顺手生成口令，
//! 好让它转给别的 Agent。
//!
//! 网络 Agent 没有账号、没人为它负责，建房有上限（设计“限额”一节）：每个网络 Agent 同时几个没关的
//! 房间、每天建几个，全站网络 Agent 建的没关的房间也有上限。它没关的房间里已经有同名的就回到那间，
//! 不新建：调用超时以后再调一次，不会多出一间。

use std::fmt;

use agent_room_domain::{
    ids::{NetworkAgentId, PrincipalId, RoomCatalogId},
    join_codes::parse_room_number,
};

use super::{
    NetworkAgentFailure, NetworkAgentFailureKind, NetworkAgentResult, NetworkAgentRoom,
    NetworkAgentService, dependency, internal, repository,
};
use crate::{
    ports::{PortFuture, PrivateRoomSnapshot},
    private_rooms::{
        AgentAccessFailure, AgentAccessFailureKind, ArchivePrivateRoom, CreatePrivateRoom,
        ListPrivateRoomsForAccount, ManageJoinCode, PrivateRoomActor, PrivateRoomFailure,
        PrivateRoomFailureKind,
    },
};

/// 房间名的上限，和人在网页上建房一样。
const ROOM_NAME_CHARACTERS: usize = 128;
const DAY_MILLIS: u64 = 24 * 60 * 60 * 1_000;

pub trait NetworkAgentRoomUseCases: Send + Sync {
    /// 建一个私人房间，房主是这个网络 Agent，并生成口令。它没关的房间里已经有同名的，就回到那间，
    /// 照旧给口令：没有就生成，看不到的不替它换。还没进去：由网关让它的加密客户端就绪后再进。
    fn create_room<'a>(
        &'a self,
        token: &'a str,
        name: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentOwnedRoom>>;

    /// 关掉（归档）自己当房主的房间：谁都不能再说话，之前的消息照样看得到。已经关了的也算成功。
    /// `room` 是房间号（私人房间网址的最后一段，也收整段网址），或它进过的房间的 Matrix 房间 ID。
    fn close_room<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<()>>;
}

/// 网络 Agent 建好的房间，或回到的同名房间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentOwnedRoom {
    pub room: NetworkAgentRoom,
    /// 新建的；为 false 时回到了它没关的同名房间。
    pub created: bool,
    /// 它已经在房间里；为 false 时由网关让它进去。
    pub joined: bool,
    pub code: NetworkAgentRoomCode,
}

/// 房间现在的口令。
#[derive(Clone, PartialEq, Eq)]
pub enum NetworkAgentRoomCode {
    /// 给人看的形式 `XXXX-XXXX-XXXX`。
    Visible(String),
    /// 有口令，但看不到了（这台服务器没法保管口令）：要转给别的 Agent 就得换一个。
    Hidden,
}

// 口令就是房间的钥匙，调试输出里不能出现。
impl fmt::Debug for NetworkAgentRoomCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Visible(_) => formatter.write_str("Visible([已脱敏])"),
            Self::Hidden => formatter.write_str("Hidden"),
        }
    }
}

impl NetworkAgentService {
    async fn create_room_internal(
        &self,
        token: &str,
        name: &str,
    ) -> NetworkAgentResult<NetworkAgentOwnedRoom> {
        let record = self.authenticated(token).await?;
        let agent_id = record.agent_id.ok_or_else(internal)?;
        let now = self.clock.now();
        self.store
            .record_activity(record.id, now)
            .await
            .map_err(repository)?;
        let name = room_name(name)?;
        let actor = PrivateRoomActor::NetworkAgent(self.actor_of(&record, now)?);
        let open = self.open_rooms(record.principal_id).await?;
        if let Some(existing) = open
            .iter()
            .find(|snapshot| snapshot.catalog().name() == name)
        {
            let room = owned_room(existing);
            let joined = self.has_joined(record.id, room.catalog_id).await?;
            let code = self.current_code(actor, room.catalog_id).await?;
            return Ok(NetworkAgentOwnedRoom {
                room,
                created: false,
                joined,
                code,
            });
        }
        self.admit_room(record.id, open.len()).await?;
        let snapshot = self
            .rooms
            .create(CreatePrivateRoom {
                actor: actor.clone(),
                catalog_id: self.identifiers.room_catalog_id(),
                name,
                description: String::new(),
                retention_days: None,
                invitations: Vec::new(),
                created_by_agent: Some(agent_id),
            })
            .await
            .map_err(creation_failure)?;
        let room = owned_room(&snapshot);
        let code = self.new_code(actor, room.catalog_id).await?;
        Ok(NetworkAgentOwnedRoom {
            room,
            created: true,
            joined: false,
            code,
        })
    }

    async fn close_room_internal(&self, token: &str, room: &str) -> NetworkAgentResult<()> {
        let record = self.authenticated(token).await?;
        let now = self.clock.now();
        self.store
            .record_activity(record.id, now)
            .await
            .map_err(repository)?;
        let catalog_id = self.room_of(record.id, room).await?;
        // 只有房主关得了，已经关了的再关一次照样成功，都由领域规则把关。
        self.rooms
            .archive(ArchivePrivateRoom {
                actor: PrivateRoomActor::NetworkAgent(self.actor_of(&record, now)?),
                catalog_id,
            })
            .await
            .map(|_| ())
            .map_err(room_failure)
    }

    /// 它当房主、还没关的私人房间。合成主体不会被邀请进别人的房间，列出来的本来就都是它的。
    async fn open_rooms(
        &self,
        principal_id: PrincipalId,
    ) -> NetworkAgentResult<Vec<PrivateRoomSnapshot>> {
        let rooms = self
            .rooms
            .list_for_account(ListPrivateRoomsForAccount { principal_id })
            .await
            .map_err(room_failure)?;
        Ok(rooms
            .into_iter()
            .filter(|snapshot| snapshot.room().owner_principal_id() == principal_id)
            .collect())
    }

    /// 建房前查上限：自己没关的房间满了、全站网络 Agent 建的没关的房间满了都不建；都没满再记一次
    /// 每天建的个数，超了告诉它什么时候能再建。
    async fn admit_room(&self, id: NetworkAgentId, open: usize) -> NetworkAgentResult<()> {
        if u64::try_from(open).unwrap_or(u64::MAX) >= u64::from(self.policy.open_rooms_per_agent) {
            return Err(NetworkAgentFailure::new(
                NetworkAgentFailureKind::RoomLimitReached,
            ));
        }
        let everyone = self
            .store
            .count_owned_private_rooms()
            .await
            .map_err(repository)?;
        if everyone >= self.policy.max_open_agent_rooms {
            return Err(NetworkAgentFailure::new(
                NetworkAgentFailureKind::RoomCapacityReached,
            ));
        }
        self.take(
            &format!("room:day:{}", id.as_uuid().simple()),
            DAY_MILLIS,
            self.policy.rooms_per_agent_per_day,
        )
        .await
    }

    async fn has_joined(
        &self,
        id: NetworkAgentId,
        catalog_id: RoomCatalogId,
    ) -> NetworkAgentResult<bool> {
        Ok(self
            .store
            .rooms(id)
            .await
            .map_err(repository)?
            .iter()
            .any(|room| room.catalog_id == catalog_id))
    }

    /// 认出要管的房间：房间号，或它进过的房间的 Matrix 房间 ID。
    async fn room_of(&self, id: NetworkAgentId, room: &str) -> NetworkAgentResult<RoomCatalogId> {
        if let Some(catalog_id) = parse_room_number(room) {
            return Ok(catalog_id);
        }
        let wanted = room.trim();
        self.store
            .rooms(id)
            .await
            .map_err(repository)?
            .into_iter()
            .find(|joined| joined.matrix_room_id.as_str() == wanted)
            .map(|joined| joined.catalog_id)
            .ok_or_else(not_owned)
    }

    /// 房间现在的口令：能看就给，没有就生成一个。有却看不到时不替它换：换了以后，已经拿到旧口令、
    /// 还没进来的 Agent 就进不来了。
    async fn current_code(
        &self,
        actor: PrivateRoomActor,
        catalog_id: RoomCatalogId,
    ) -> NetworkAgentResult<NetworkAgentRoomCode> {
        let revealed = self
            .access
            .reveal_code(ManageJoinCode {
                actor: actor.clone(),
                catalog_id,
            })
            .await;
        match revealed {
            Ok(code) => Ok(NetworkAgentRoomCode::Visible(code.code.display())),
            Err(failure) if failure.kind() == AgentAccessFailureKind::NotFound => {
                self.new_code(actor, catalog_id).await
            }
            Err(failure) if failure.kind() == AgentAccessFailureKind::NotRevealable => {
                Ok(NetworkAgentRoomCode::Hidden)
            }
            Err(failure) => Err(access_failure(failure)),
        }
    }

    async fn new_code(
        &self,
        actor: PrivateRoomActor,
        catalog_id: RoomCatalogId,
    ) -> NetworkAgentResult<NetworkAgentRoomCode> {
        self.access
            .generate_code(ManageJoinCode { actor, catalog_id })
            .await
            .map(|generated| NetworkAgentRoomCode::Visible(generated.code.display()))
            .map_err(access_failure)
    }
}

impl NetworkAgentRoomUseCases for NetworkAgentService {
    fn create_room<'a>(
        &'a self,
        token: &'a str,
        name: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentOwnedRoom>> {
        Box::pin(self.create_room_internal(token, name))
    }

    fn close_room<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<()>> {
        Box::pin(self.close_room_internal(token, room))
    }
}

/// 房间名去掉首尾空白后 1 到 128 个字符，不能有控制字符，和人在网页上建房一样。
fn room_name(name: &str) -> NetworkAgentResult<String> {
    let name = name.trim();
    let characters = name.chars().count();
    if characters == 0 || characters > ROOM_NAME_CHARACTERS || name.chars().any(char::is_control) {
        return Err(NetworkAgentFailure::new(
            NetworkAgentFailureKind::RoomNameInvalid,
        ));
    }
    Ok(name.to_owned())
}

fn owned_room(snapshot: &PrivateRoomSnapshot) -> NetworkAgentRoom {
    NetworkAgentRoom {
        catalog_id: snapshot.catalog().id(),
        matrix_room_id: snapshot.instance().matrix_room_id().clone(),
        name: snapshot.catalog().name().to_owned(),
    }
}

/// 建房时过不了领域校验的只会是名字，其余和别的管房间一样。
fn creation_failure(failure: PrivateRoomFailure) -> NetworkAgentFailure {
    if failure.kind() == PrivateRoomFailureKind::InvalidRequest {
        return NetworkAgentFailure::new(NetworkAgentFailureKind::RoomNameInvalid);
    }
    room_failure(failure)
}

/// 房间不存在和不是它的都只说不是它的房间，不透露别人的房间在不在。
fn room_failure(failure: PrivateRoomFailure) -> NetworkAgentFailure {
    match failure.kind() {
        PrivateRoomFailureKind::NotFound | PrivateRoomFailureKind::Forbidden => not_owned(),
        PrivateRoomFailureKind::Conflict
        | PrivateRoomFailureKind::DependencyUnavailable
        | PrivateRoomFailureKind::UnknownCommit => dependency(),
        PrivateRoomFailureKind::InvalidRequest | PrivateRoomFailureKind::Internal => internal(),
    }
}

fn access_failure(failure: AgentAccessFailure) -> NetworkAgentFailure {
    match failure.kind() {
        AgentAccessFailureKind::NotFound | AgentAccessFailureKind::Forbidden => not_owned(),
        AgentAccessFailureKind::RateLimited => failure
            .retry_at()
            .map_or_else(dependency, NetworkAgentFailure::rate_limited),
        AgentAccessFailureKind::Conflict
        | AgentAccessFailureKind::DependencyUnavailable
        | AgentAccessFailureKind::UnknownCommit => dependency(),
        AgentAccessFailureKind::InvalidRequest
        | AgentAccessFailureKind::Internal
        | AgentAccessFailureKind::NotRevealable => internal(),
    }
}

const fn not_owned() -> NetworkAgentFailure {
    NetworkAgentFailure::new(NetworkAgentFailureKind::RoomNotOwned)
}
