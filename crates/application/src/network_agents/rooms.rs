//! 网络 Agent 自己建房间、管房间（`specs/agent-rooms/design.md` 第 4 步）。
//!
//! 房主是网络 Agent 的合成主体，它的 Agent 按“房主的 Agent”进去（`admits_agent_of`）。建房、关房、
//! 口令、敲门、邀请人和移出 Agent 都走私人房间的用例，操作者是网络 Agent
//! （`PrivateRoomActor::NetworkAgent`）；建房时顺手生成口令，好让它转给别的 Agent。管房间只认它
//! 当房主、还没关的房间，别的一律说不是它的。
//!
//! 网络 Agent 没有账号、没人为它负责，建房有上限（设计“限额”一节）：每个网络 Agent 同时几个没关的
//! 房间、每天建几个，全站网络 Agent 建的没关的房间也有上限。它没关的房间里已经有同名的就回到那间，
//! 不新建：调用超时以后再调一次，不会多出一间。
//!
//! 能放行的只有它自己，所以有人敲它建的房间的门时，网关在它等消息时叫醒它，等消息的回答和 `/me`
//! 都列出在等的敲门（`incoming_knocks`）。不然敲门放一小时没人回答就作废了。

use std::fmt;

use agent_room_domain::{
    ids::{AgentId, NetworkAgentId, PrincipalId, RoomCatalogId},
    join_codes::parse_room_number,
    private_rooms::PrivateRoomPermissions,
};

use super::{
    NetworkAgentFailure, NetworkAgentFailureKind, NetworkAgentResult, NetworkAgentRoom,
    NetworkAgentService, dependency, internal, repository,
};
use crate::{
    ports::{NetworkAgentRecord, PortFuture, PrivateRoomAgentKnockRecord, PrivateRoomSnapshot},
    private_rooms::{
        AdmittedKnock, AgentAccessFailure, AgentAccessFailureKind, AnswerKnock, ArchivePrivateRoom,
        CreatePrivateRoom, InspectAgentAccess, InvitePrivateRoomMember, ListPrivateRoomsForAccount,
        ManageJoinCode, PrivateRoomActor, PrivateRoomFailure, PrivateRoomFailureKind,
        PrivateRoomFailureStage, RemoveAgentMember,
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

    /// 看它建的房间现在的口令。没有口令时回答 `Missing`，不替它生成：看口令不该有副作用。
    /// 下面管房间的几样，`room` 都是它当房主、还没关的房间的房间号或 Matrix 房间 ID。
    fn room_code<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentRoomCodeView>>;

    /// 生成口令；已经有的换成新的，旧的马上失效，已经进来的 Agent 不受影响。
    fn replace_code<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentRoomCodeView>>;

    /// 停用口令：拿旧口令进不来了，敲门照旧。原本就没有口令也算成功。
    fn disable_code<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<()>>;

    /// 在等、没作废的敲门，先敲的在前。
    fn waiting_knocks<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<Vec<PrivateRoomAgentKnockRecord>>>;

    /// 放行：记为这个房间的 Agent 成员，敲门还在等。由网关替它进房间，进去了再 `complete_knock`。
    fn admit_knock<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
        agent_id: AgentId,
    ) -> PortFuture<'a, NetworkAgentResult<AdmittedKnock>>;

    /// 放行的 Agent 进了房间，把敲门记为放进来了。
    fn complete_knock<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
        agent_id: AgentId,
    ) -> PortFuture<'a, NetworkAgentResult<()>>;

    /// 不让进：之后它再敲还是不让进。原本就不在等也算成功。
    fn decline_knock<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
        agent_id: AgentId,
    ) -> PortFuture<'a, NetworkAgentResult<()>>;

    /// 按账户 ID 邀请一个人，只给查看和发言；对方在应用里接受后就能进来。已经受邀、已经在房间里
    /// 都算成功。每个网络 Agent 每天发的邀请有上限。
    fn invite_person<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
        account_id: PrincipalId,
    ) -> PortFuture<'a, NetworkAgentResult<()>>;

    /// 移出凭口令或敲门进来的 Agent；它要再进来得有新口令或再敲门。已经移出的也算成功。
    fn remove_agent<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
        agent_id: AgentId,
    ) -> PortFuture<'a, NetworkAgentResult<()>>;

    /// 敲它建的房间的门、在等它回答的：它当房主、还没关的房间里在等的敲门，先敲的在前。网关等消息
    /// 时隔一会儿看一次，有新的就叫醒它。不记活动，只是看一眼。
    fn incoming_knocks<'a>(
        &'a self,
        token: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentIncomingKnocks>>;
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
    /// 没有口令：停用了，或者还没生成。只能敲门进来。
    Missing,
}

// 口令就是房间的钥匙，调试输出里不能出现。
impl fmt::Debug for NetworkAgentRoomCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Visible(_) => formatter.write_str("Visible([已脱敏])"),
            Self::Hidden => formatter.write_str("Hidden"),
            Self::Missing => formatter.write_str("Missing"),
        }
    }
}

/// 它建的一个房间和房间现在的口令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentRoomCodeView {
    pub room: NetworkAgentRoom,
    pub code: NetworkAgentRoomCode,
}

/// 敲它建的房间的门、在等它回答的。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkAgentIncomingKnocks {
    /// 它有没关的房间：一间都没有时谁也敲不了它的门，别处有人敲门时网关不必替它来看。
    pub owns_rooms: bool,
    /// 先敲的在前。
    pub knocks: Vec<NetworkAgentIncomingKnock>,
}

/// 谁敲了它的哪个房间的门。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentIncomingKnock {
    pub room: NetworkAgentRoom,
    pub knock: PrivateRoomAgentKnockRecord,
}

/// 管房间时认出的房间，和以谁的身份管。
struct ManagedRoom {
    /// 网络 Agent 自己：每天发的邀请按它算。
    agent: NetworkAgentId,
    actor: PrivateRoomActor,
    room: NetworkAgentRoom,
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

    async fn room_code_internal(
        &self,
        token: &str,
        room: &str,
    ) -> NetworkAgentResult<NetworkAgentRoomCodeView> {
        let ManagedRoom { actor, room, .. } = self.managed(token, room).await?;
        let revealed = self
            .access
            .reveal_code(ManageJoinCode {
                actor,
                catalog_id: room.catalog_id,
            })
            .await;
        let code = match revealed {
            Ok(code) => NetworkAgentRoomCode::Visible(code.code.display()),
            Err(failure) if failure.kind() == AgentAccessFailureKind::NotFound => {
                NetworkAgentRoomCode::Missing
            }
            Err(failure) if failure.kind() == AgentAccessFailureKind::NotRevealable => {
                NetworkAgentRoomCode::Hidden
            }
            Err(failure) => return Err(access_failure(failure)),
        };
        Ok(NetworkAgentRoomCodeView { room, code })
    }

    async fn replace_code_internal(
        &self,
        token: &str,
        room: &str,
    ) -> NetworkAgentResult<NetworkAgentRoomCodeView> {
        let ManagedRoom { actor, room, .. } = self.managed(token, room).await?;
        let code = self.new_code(actor, room.catalog_id).await?;
        Ok(NetworkAgentRoomCodeView { room, code })
    }

    async fn disable_code_internal(&self, token: &str, room: &str) -> NetworkAgentResult<()> {
        let ManagedRoom { actor, room, .. } = self.managed(token, room).await?;
        self.access
            .disable_code(ManageJoinCode {
                actor,
                catalog_id: room.catalog_id,
            })
            .await
            .map_err(access_failure)
    }

    async fn waiting_knocks_internal(
        &self,
        token: &str,
        room: &str,
    ) -> NetworkAgentResult<Vec<PrivateRoomAgentKnockRecord>> {
        let ManagedRoom { actor, room, .. } = self.managed(token, room).await?;
        self.access
            .waiting_knocks(InspectAgentAccess {
                actor,
                catalog_id: room.catalog_id,
            })
            .await
            .map_err(knock_failure)
    }

    /// 回答敲门的请求：认出房间，带上它这个操作者。
    async fn answer(
        &self,
        token: &str,
        room: &str,
        agent_id: AgentId,
    ) -> NetworkAgentResult<AnswerKnock> {
        let ManagedRoom { actor, room, .. } = self.managed(token, room).await?;
        Ok(AnswerKnock {
            actor,
            catalog_id: room.catalog_id,
            agent_id,
        })
    }

    async fn invite_person_internal(
        &self,
        token: &str,
        room: &str,
        account_id: PrincipalId,
    ) -> NetworkAgentResult<()> {
        let ManagedRoom { agent, actor, room } = self.managed(token, room).await?;
        // 邀请会弹到对方的应用里：每天发的有上限。没有这个账户的也算一次，免得拿它一个个试账户 ID。
        self.take(
            &format!("invite:day:{}", agent.as_uuid().simple()),
            DAY_MILLIS,
            self.policy.invitations_per_agent_per_day,
        )
        .await?;
        self.rooms
            .invite(InvitePrivateRoomMember {
                actor,
                catalog_id: room.catalog_id,
                target_principal_id: account_id,
                permissions: PrivateRoomPermissions::INVITED_BY_AGENT,
            })
            .await
            .map(|_| ())
            .map_err(invitation_failure)
    }

    async fn remove_agent_internal(
        &self,
        token: &str,
        room: &str,
        agent_id: AgentId,
    ) -> NetworkAgentResult<()> {
        let ManagedRoom { actor, room, .. } = self.managed(token, room).await?;
        self.access
            .remove_agent(RemoveAgentMember {
                actor,
                catalog_id: room.catalog_id,
                agent_id,
            })
            .await
            .map_err(removal_failure)
    }

    /// 管房间的请求先认令牌、记一次活动，再认出房间：只认它当房主、还没关的房间。
    async fn managed(&self, token: &str, room: &str) -> NetworkAgentResult<ManagedRoom> {
        let record = self.authenticated(token).await?;
        let now = self.clock.now();
        self.store
            .record_activity(record.id, now)
            .await
            .map_err(repository)?;
        let actor = PrivateRoomActor::NetworkAgent(self.actor_of(&record, now)?);
        let wanted = parse_room_number(room);
        let matrix_room_id = room.trim();
        let room = self
            .open_rooms(record.principal_id)
            .await?
            .iter()
            .find(|snapshot| {
                wanted.map_or_else(
                    || snapshot.instance().matrix_room_id().as_str() == matrix_room_id,
                    |catalog_id| snapshot.catalog().id() == catalog_id,
                )
            })
            .map(owned_room)
            .ok_or_else(not_owned)?;
        Ok(ManagedRoom {
            agent: record.id,
            actor,
            room,
        })
    }

    /// 它当房主、还没关的私人房间。合成主体不会被邀请进别人的房间，列出来的本来就都是它的。
    /// 它当房主、还没关的房间里在等的敲门，先敲的在前。看的这一会儿刚关掉的房间跳过：没有要它
    /// 回答的敲门了。
    pub(super) async fn incoming_knocks_of(
        &self,
        record: &NetworkAgentRecord,
    ) -> NetworkAgentResult<NetworkAgentIncomingKnocks> {
        let rooms = self.open_rooms(record.principal_id).await?;
        if rooms.is_empty() {
            return Ok(NetworkAgentIncomingKnocks::default());
        }
        let actor = PrivateRoomActor::NetworkAgent(self.actor_of(record, self.clock.now())?);
        let mut knocks = Vec::new();
        for snapshot in &rooms {
            let room = owned_room(snapshot);
            let waiting = match self
                .access
                .waiting_knocks(InspectAgentAccess {
                    actor: actor.clone(),
                    catalog_id: room.catalog_id,
                })
                .await
            {
                Ok(waiting) => waiting,
                Err(failure)
                    if matches!(
                        failure.kind(),
                        AgentAccessFailureKind::NotFound
                            | AgentAccessFailureKind::Forbidden
                            | AgentAccessFailureKind::Conflict
                    ) =>
                {
                    continue;
                }
                Err(failure) => return Err(access_failure(failure)),
            };
            knocks.extend(waiting.into_iter().map(|knock| NetworkAgentIncomingKnock {
                room: room.clone(),
                knock,
            }));
        }
        knocks.sort_by_key(|incoming| incoming.knock.knocked_at);
        Ok(NetworkAgentIncomingKnocks {
            owns_rooms: true,
            knocks,
        })
    }

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

    fn room_code<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentRoomCodeView>> {
        Box::pin(self.room_code_internal(token, room))
    }

    fn replace_code<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentRoomCodeView>> {
        Box::pin(self.replace_code_internal(token, room))
    }

    fn disable_code<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<()>> {
        Box::pin(self.disable_code_internal(token, room))
    }

    fn waiting_knocks<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<Vec<PrivateRoomAgentKnockRecord>>> {
        Box::pin(self.waiting_knocks_internal(token, room))
    }

    fn admit_knock<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
        agent_id: AgentId,
    ) -> PortFuture<'a, NetworkAgentResult<AdmittedKnock>> {
        Box::pin(async move {
            let answer = self.answer(token, room, agent_id).await?;
            self.access.admit_knock(answer).await.map_err(knock_failure)
        })
    }

    fn complete_knock<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
        agent_id: AgentId,
    ) -> PortFuture<'a, NetworkAgentResult<()>> {
        Box::pin(async move {
            let answer = self.answer(token, room, agent_id).await?;
            self.access
                .complete_knock(answer)
                .await
                .map_err(knock_failure)
        })
    }

    fn decline_knock<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
        agent_id: AgentId,
    ) -> PortFuture<'a, NetworkAgentResult<()>> {
        Box::pin(async move {
            let answer = self.answer(token, room, agent_id).await?;
            self.access
                .decline_knock(answer)
                .await
                .map_err(knock_failure)
        })
    }

    fn invite_person<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
        account_id: PrincipalId,
    ) -> PortFuture<'a, NetworkAgentResult<()>> {
        Box::pin(self.invite_person_internal(token, room, account_id))
    }

    fn remove_agent<'a>(
        &'a self,
        token: &'a str,
        room: &'a str,
        agent_id: AgentId,
    ) -> PortFuture<'a, NetworkAgentResult<()>> {
        Box::pin(self.remove_agent_internal(token, room, agent_id))
    }

    fn incoming_knocks<'a>(
        &'a self,
        token: &'a str,
    ) -> PortFuture<'a, NetworkAgentResult<NetworkAgentIncomingKnocks>> {
        Box::pin(async move {
            let record = self.authenticated(token).await?;
            self.incoming_knocks_of(&record).await
        })
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

/// 邀请时找不到的是账户：房间已经认过是它的。房间这时恰好关了，领域上是冲突，也只说不是它能管的。
fn invitation_failure(failure: PrivateRoomFailure) -> NetworkAgentFailure {
    match (failure.kind(), failure.stage()) {
        (PrivateRoomFailureKind::NotFound, PrivateRoomFailureStage::Directory) => {
            NetworkAgentFailure::new(NetworkAgentFailureKind::AccountNotFound)
        }
        (PrivateRoomFailureKind::Conflict, _) => not_owned(),
        _ => room_failure(failure),
    }
}

/// 移出时找不到的是 Agent 成员：房间已经认过是它的。
fn removal_failure(failure: AgentAccessFailure) -> NetworkAgentFailure {
    if failure.kind() == AgentAccessFailureKind::NotFound {
        return NetworkAgentFailure::new(NetworkAgentFailureKind::AgentNotInRoom);
    }
    access_failure(failure)
}

/// 回答敲门时找不到的只会是敲门：房间已经认过是它的。
fn knock_failure(failure: AgentAccessFailure) -> NetworkAgentFailure {
    if failure.kind() == AgentAccessFailureKind::NotFound {
        return NetworkAgentFailure::new(NetworkAgentFailureKind::KnockNotFound);
    }
    access_failure(failure)
}

const fn not_owned() -> NetworkAgentFailure {
    NetworkAgentFailure::new(NetworkAgentFailureKind::RoomNotOwned)
}
