use std::{collections::BTreeSet, sync::Arc};

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::{
        PrivateRoom, PrivateRoomLifecycleStatus, PrivateRoomMember, PrivateRoomMembershipStatus,
        PrivateRoomPermissions,
    },
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState,
    },
    version::AggregateVersion,
};

use crate::{
    authentication::AuthenticatedPrincipal,
    persistence::RepositoryErrorKind,
    ports::{
        Clock, IdentifierFactory, MatrixCreateRoom, MatrixRoomAliasLocalpart, MatrixRoomId,
        MatrixRoomPowerProfile, MatrixRoomPreset, MatrixRoomVisibility, MatrixUserId, PortFuture,
        PrivateMatrixMembership, PrivateMatrixRoomCreation, PrivateMatrixSpeakingAssignment,
        PrivateRoomAgentDirectory, PrivateRoomMatrixGateway, PrivateRoomMatrixProvisioner,
        PrivateRoomPrincipalDirectory, PrivateRoomSnapshot, PrivateRoomStore,
    },
};

use super::{
    ArchivePrivateRoom, ChangePrivateRoomPermissions, CreatePrivateRoom, GovernPrivateRoomMember,
    InspectPrivateRoom, InviteByLink, InvitePrivateRoomMember, ListPrivateRooms,
    ListPrivateRoomsForAccount, PrivateRoomActor, PrivateRoomCreatorAgent, PrivateRoomFailureKind,
    PrivateRoomFailureStage, PrivateRoomMembershipAction, PrivateRoomOrigin, PrivateRoomResult,
    RenamePrivateRoom, TransferPrivateRoomOwnership,
    failure::{domain, failure, matrix, repository},
};

pub trait PrivateRoomUseCases: Send + Sync {
    fn create(
        &self,
        request: CreatePrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn inspect(
        &self,
        request: InspectPrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    /// 房间是谁建的。和 [`Self::inspect`] 一样受邀和已加入的成员都看得到：接受邀请之前就知道
    /// 房主是不是网络 Agent、服务器读不读得到。
    fn origin(
        &self,
        request: InspectPrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomOrigin>>;

    fn list(
        &self,
        request: ListPrivateRooms,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<PrivateRoomSnapshot>>>;

    /// 列出账号的 Agent 此刻进得去的私人房间（账号已加入、能发言、房间没归档），供已认证的
    /// 设备替该账号的 Agent 查询。只是受邀或只能看的房间不列：Agent 照着进会被拒。
    fn list_for_account(
        &self,
        request: ListPrivateRoomsForAccount,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<PrivateRoomSnapshot>>>;

    fn invite(
        &self,
        request: InvitePrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn accept(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn decline(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn leave(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn remove(
        &self,
        request: GovernPrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn ban(
        &self,
        request: GovernPrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    /// 撤销封禁，这个人回到“已移出”，之后能再邀请。
    fn unban(
        &self,
        request: GovernPrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn update_permissions(
        &self,
        request: ChangePrivateRoomPermissions,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn transfer_ownership(
        &self,
        request: TransferPrivateRoomOwnership,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn archive(
        &self,
        request: ArchivePrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;

    fn rename(
        &self,
        request: RenamePrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;
}

/// 没有操作者的归档：停用的网络 Agent 当房主的房间由系统关掉（`specs/agent-rooms/design.md`）。
/// 和 [`PrivateRoomUseCases`] 分开，只交给网络 Agent 的网关。
pub trait PrivateRoomRetirement: Send + Sync {
    /// 关掉 `owner` 当房主、还在用的私人房间，返回这次关了几个。副作用和房主自己归档一样：
    /// 锁死 Matrix 上的发言、改目录状态。一个关不成就报错，已经关上的下次跳过。
    fn archive_owned_by(&self, owner: PrincipalId) -> PortFuture<'_, PrivateRoomResult<usize>>;
}

/// 凭邀请链接进来：以链接创建人的名义做一次普通的邀请（`specs/room-people/design.md`）。创建人
/// 不在场，不看他的登录，只看他此刻还能管邀请链接、给的权限还在他的权限以内。和
/// [`PrivateRoomUseCases`] 分开，只交给邀请链接的服务。
pub trait PrivateRoomLinkInvitations: Send + Sync {
    fn invite_by_link(
        &self,
        request: InviteByLink,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>>;
}

pub struct PrivateRoomDependencies {
    pub store: Arc<dyn PrivateRoomStore>,
    pub matrix_provisioner: Arc<dyn PrivateRoomMatrixProvisioner>,
    pub matrix: Arc<dyn PrivateRoomMatrixGateway>,
    pub principals: Arc<dyn PrivateRoomPrincipalDirectory>,
    /// 成员能力变化时找到其名下 Agent，让它们在 Matrix 上的能力随之收紧或恢复。
    pub agents: Arc<dyn PrivateRoomAgentDirectory>,
    /// 必须能读取当前 Matrix 房间状态的受信服务身份，例如内容授权服务。
    pub trusted_matrix_readers: Vec<MatrixUserId>,
    pub identifiers: Arc<dyn IdentifierFactory>,
    pub clock: Arc<dyn Clock>,
}

pub struct PrivateRoomService {
    store: Arc<dyn PrivateRoomStore>,
    matrix_provisioner: Arc<dyn PrivateRoomMatrixProvisioner>,
    matrix: Arc<dyn PrivateRoomMatrixGateway>,
    principals: Arc<dyn PrivateRoomPrincipalDirectory>,
    agents: Arc<dyn PrivateRoomAgentDirectory>,
    trusted_matrix_readers: Vec<MatrixUserId>,
    identifiers: Arc<dyn IdentifierFactory>,
    clock: Arc<dyn Clock>,
}

impl PrivateRoomService {
    pub fn new(dependencies: PrivateRoomDependencies) -> Self {
        Self {
            store: dependencies.store,
            matrix_provisioner: dependencies.matrix_provisioner,
            matrix: dependencies.matrix,
            principals: dependencies.principals,
            agents: dependencies.agents,
            trusted_matrix_readers: dependencies.trusted_matrix_readers,
            identifiers: dependencies.identifiers,
            clock: dependencies.clock,
        }
    }

    async fn create_internal(
        &self,
        request: CreatePrivateRoom,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.create";
        let owner_matrix_user =
            active_room_actor_matrix_user(&request.actor, self.clock.now(), OPERATION)?;
        self.ensure_creator(&request, OPERATION).await?;
        if let Some(existing) = self.find(request.catalog_id, OPERATION).await? {
            return ensure_same_creation(&request, existing, OPERATION);
        }

        let prepared = self
            .prepare_creation(&request, owner_matrix_user, OPERATION)
            .await?;
        let matrix_room_id = self
            .matrix_provisioner
            .create(&prepared.matrix)
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        self.matrix
            .set_speaking_batch(&matrix_room_id, &prepared.speaking_assignments)
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        let snapshot = self.build_snapshot(&request, prepared.room, &matrix_room_id, OPERATION)?;
        let now = self.clock.now();
        match self.store.create(&snapshot, now).await {
            Ok(()) => Ok(snapshot),
            Err(error) if error.kind() == RepositoryErrorKind::Conflict => {
                let existing =
                    self.find(request.catalog_id, OPERATION)
                        .await?
                        .ok_or_else(|| {
                            failure(
                                OPERATION,
                                PrivateRoomFailureStage::Persistence,
                                PrivateRoomFailureKind::Conflict,
                            )
                        })?;
                ensure_same_creation(&request, existing, OPERATION)
            }
            Err(error) => Err(repository(
                OPERATION,
                PrivateRoomFailureStage::Persistence,
                &error,
            )),
        }
    }

    async fn inspect_internal(
        &self,
        request: InspectPrivateRoom,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.inspect";
        ensure_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let visible = snapshot
            .room()
            .member(request.actor.principal_id)
            .is_some_and(|member| {
                matches!(
                    member.status(),
                    PrivateRoomMembershipStatus::Invited | PrivateRoomMembershipStatus::Joined
                )
            });
        if !visible {
            return Err(failure(
                OPERATION,
                PrivateRoomFailureStage::Validation,
                PrivateRoomFailureKind::NotFound,
            ));
        }
        Ok(snapshot)
    }

    async fn origin_internal(
        &self,
        request: InspectPrivateRoom,
    ) -> PrivateRoomResult<PrivateRoomOrigin> {
        const OPERATION: &str = "private_room.origin";
        let snapshot = self.inspect_internal(request).await?;
        let room = snapshot.room();
        let Some(agent_id) = room.created_by_agent() else {
            return Ok(PrivateRoomOrigin {
                created_by_agent: None,
                owner_is_network_agent: false,
            });
        };
        let description = self
            .agents
            .describe_agent(agent_id)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        // 网络 Agent 只能建自己当房主的房间，也转让不了：建房的是网络 Agent、房主又是它，
        // 服务器就读得到这里的消息。
        let owner_is_network_agent = description
            .as_ref()
            .and_then(|agent| agent.network_principal_id)
            .is_some_and(|principal| principal == room.owner_principal_id());
        Ok(PrivateRoomOrigin {
            created_by_agent: Some(PrivateRoomCreatorAgent {
                agent_id,
                display_name: description.map(|agent| agent.display_name),
            }),
            owner_is_network_agent,
        })
    }

    async fn list_internal(
        &self,
        request: ListPrivateRooms,
    ) -> PrivateRoomResult<Vec<PrivateRoomSnapshot>> {
        const OPERATION: &str = "private_room.list";
        ensure_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        self.store
            .list_for_principal(request.actor.principal_id)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))
    }

    async fn list_for_account_internal(
        &self,
        request: ListPrivateRoomsForAccount,
    ) -> PrivateRoomResult<Vec<PrivateRoomSnapshot>> {
        const OPERATION: &str = "private_room.list_for_account";
        let rooms = self
            .store
            .list_for_principal(request.principal_id)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        // 和 Agent 进房间时同一条规则（`admits_agent_of`），列出来的就进得去。
        Ok(rooms
            .into_iter()
            .filter(|snapshot| snapshot.room().admits_agent_of(request.principal_id))
            .collect())
    }

    async fn invite_internal(
        &self,
        request: InvitePrivateRoomMember,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.invite";
        ensure_active_room_actor(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let actor = request.actor.principal_id();
        let changed = if request.actor.is_agent() {
            if request.permissions != PrivateRoomPermissions::INVITED_BY_AGENT {
                return Err(failure(
                    OPERATION,
                    PrivateRoomFailureStage::Validation,
                    PrivateRoomFailureKind::InvalidRequest,
                ));
            }
            room.invite_by_agent(actor, request.target_principal_id)
        } else {
            room.invite(actor, request.target_principal_id, request.permissions)
        }
        .map_err(|error| domain(OPERATION, &error))?;
        self.deliver_invitation(
            snapshot,
            room,
            expected,
            changed,
            request.target_principal_id,
            OPERATION,
        )
        .await
    }

    async fn invite_by_link_internal(
        &self,
        request: InviteByLink,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.invite_by_link";
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        room.authorize_invite_link(request.link_creator)
            .map_err(|error| domain(OPERATION, &error))?;
        let changed = room
            .invite(
                request.link_creator,
                request.target_principal_id,
                request.permissions,
            )
            .map_err(|error| domain(OPERATION, &error))?;
        self.deliver_invitation(
            snapshot,
            room,
            expected,
            changed,
            request.target_principal_id,
            OPERATION,
        )
        .await
    }

    /// 领域上已经记成受邀的人，在 Matrix 上邀请他、按邀请的权限设好发言级别，再保存。
    async fn deliver_invitation(
        &self,
        snapshot: PrivateRoomSnapshot,
        room: PrivateRoom,
        expected: AggregateVersion,
        changed: bool,
        target_principal_id: PrincipalId,
        operation: &'static str,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        // Agent 邀请已经在房间里的人：什么都不用做。别再发 Matrix 邀请，Synapse 不让邀请已经加入的人。
        let Some(member) = room
            .member(target_principal_id)
            .filter(|member| !member.has_joined() || changed)
        else {
            return Ok(snapshot);
        };
        let speak = member.permissions().speak();
        let target = self
            .principal_matrix_user(target_principal_id, operation)
            .await?;
        let matrix_room = matrix_room_id(&snapshot, operation)?;
        self.matrix
            .invite(&matrix_room, &target)
            .await
            .map_err(|error| matrix(operation, error))?;
        // 私人房间的默认成员级别低于发言级别，建房时一起邀请的成员才批量拿到发言级别；之后邀请的
        // 成员也要按邀请的权限设好，否则进了房间也发不了言。被移出后再邀请只给旁观时同样收回。
        self.set_speaking(&matrix_room, &target, speak, operation)
            .await?;
        self.save_if_changed(&room, expected, changed, operation)
            .await?;
        replace_room(snapshot, room, operation)
    }

    async fn accept_internal(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.accept";
        let actor_matrix = active_actor_matrix_user(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let changed = room
            .accept_invitation(request.actor.principal_id)
            .map_err(|error| domain(OPERATION, &error))?;
        self.require_matrix_membership(
            &snapshot,
            &actor_matrix,
            MatrixMembershipRequirement::Joined,
            OPERATION,
        )
        .await?;
        self.save_if_changed(&room, expected, changed, OPERATION)
            .await?;
        replace_room(snapshot, room, OPERATION)
    }

    async fn decline_internal(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.decline";
        let actor_matrix = active_actor_matrix_user(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let changed = room
            .decline_invitation(request.actor.principal_id)
            .map_err(|error| domain(OPERATION, &error))?;
        self.require_matrix_membership(
            &snapshot,
            &actor_matrix,
            MatrixMembershipRequirement::AbsentOrLeft,
            OPERATION,
        )
        .await?;
        self.save_if_changed(&room, expected, changed, OPERATION)
            .await?;
        replace_room(snapshot, room, OPERATION)
    }

    async fn leave_internal(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.leave";
        let actor_matrix = active_actor_matrix_user(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let changed = room
            .leave(request.actor.principal_id)
            .map_err(|error| domain(OPERATION, &error))?;
        self.require_matrix_membership(
            &snapshot,
            &actor_matrix,
            MatrixMembershipRequirement::AbsentOrLeft,
            OPERATION,
        )
        .await?;
        let matrix_room = matrix_room_id(&snapshot, OPERATION)?;
        self.govern_member_agents(
            &matrix_room,
            request.actor.principal_id,
            MemberAgentChange::Evict,
            OPERATION,
        )
        .await?;
        self.save_if_changed(&room, expected, changed, OPERATION)
            .await?;
        replace_room(snapshot, room, OPERATION)
    }

    async fn remove_internal(
        &self,
        request: GovernPrivateRoomMember,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.remove";
        ensure_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let changed = room
            .remove_member(request.actor.principal_id, request.target_principal_id)
            .map_err(|error| domain(OPERATION, &error))?;
        let target = self
            .principal_matrix_user(request.target_principal_id, OPERATION)
            .await?;
        let matrix_room = matrix_room_id(&snapshot, OPERATION)?;
        self.matrix
            .kick(&matrix_room, &target)
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        self.govern_member_agents(
            &matrix_room,
            request.target_principal_id,
            MemberAgentChange::Evict,
            OPERATION,
        )
        .await?;
        self.save_if_changed(&room, expected, changed, OPERATION)
            .await?;
        replace_room(snapshot, room, OPERATION)
    }

    async fn ban_internal(
        &self,
        request: GovernPrivateRoomMember,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.ban";
        ensure_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let changed = room
            .ban_member(request.actor.principal_id, request.target_principal_id)
            .map_err(|error| domain(OPERATION, &error))?;
        let target = self
            .principal_matrix_user(request.target_principal_id, OPERATION)
            .await?;
        let matrix_room = matrix_room_id(&snapshot, OPERATION)?;
        self.matrix
            .ban(&matrix_room, &target)
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        // Agent 只移出不封禁：封禁针对成员本人，Agent 仍可能被其他能发言的共同所有者带进来。
        self.govern_member_agents(
            &matrix_room,
            request.target_principal_id,
            MemberAgentChange::Evict,
            OPERATION,
        )
        .await?;
        self.save_if_changed(&room, expected, changed, OPERATION)
            .await?;
        replace_room(snapshot, room, OPERATION)
    }

    async fn unban_internal(
        &self,
        request: GovernPrivateRoomMember,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.unban";
        ensure_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let changed = room
            .unban_member(request.actor.principal_id, request.target_principal_id)
            .map_err(|error| domain(OPERATION, &error))?;
        if !changed {
            return replace_room(snapshot, room, OPERATION);
        }
        // 先在 Matrix 上解开再保存，和封禁同一个顺序：保存失败时产品上还封着，重试会再解一次
        // （已经解开的跳过）再保存。他的 Agent 封禁时只移出没封，这里不用管。
        let target = self
            .principal_matrix_user(request.target_principal_id, OPERATION)
            .await?;
        let matrix_room = matrix_room_id(&snapshot, OPERATION)?;
        self.matrix
            .unban(&matrix_room, &target)
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        self.save_if_changed(&room, expected, changed, OPERATION)
            .await?;
        replace_room(snapshot, room, OPERATION)
    }

    async fn update_permissions_internal(
        &self,
        request: ChangePrivateRoomPermissions,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.update_permissions";
        ensure_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let previous = snapshot
            .room()
            .member(request.target_principal_id)
            .map(PrivateRoomMember::permissions)
            .ok_or_else(|| {
                failure(
                    OPERATION,
                    PrivateRoomFailureStage::Validation,
                    PrivateRoomFailureKind::Conflict,
                )
            })?;
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let changed = room
            .update_permissions(
                request.actor.principal_id,
                request.target_principal_id,
                request.permissions,
            )
            .map_err(|error| domain(OPERATION, &error))?;
        let target = self
            .principal_matrix_user(request.target_principal_id, OPERATION)
            .await?;
        let matrix_room = matrix_room_id(&snapshot, OPERATION)?;

        let target_principal = request.target_principal_id;
        match (previous.speak(), request.permissions.speak()) {
            (true, false) => {
                self.set_speaking(&matrix_room, &target, false, OPERATION)
                    .await?;
                self.govern_member_agents(
                    &matrix_room,
                    target_principal,
                    MemberAgentChange::Speak(false),
                    OPERATION,
                )
                .await?;
                self.save_if_changed(&room, expected, changed, OPERATION)
                    .await?;
            }
            (false, true) => {
                self.save_if_changed(&room, expected, changed, OPERATION)
                    .await?;
                self.set_speaking(&matrix_room, &target, true, OPERATION)
                    .await?;
                self.govern_member_agents(
                    &matrix_room,
                    target_principal,
                    MemberAgentChange::Speak(true),
                    OPERATION,
                )
                .await?;
            }
            (_, desired) => {
                self.save_if_changed(&room, expected, changed, OPERATION)
                    .await?;
                if !changed {
                    self.set_speaking(&matrix_room, &target, desired, OPERATION)
                        .await?;
                    self.govern_member_agents(
                        &matrix_room,
                        target_principal,
                        MemberAgentChange::Speak(desired),
                        OPERATION,
                    )
                    .await?;
                }
            }
        }
        replace_room(snapshot, room, OPERATION)
    }

    async fn transfer_ownership_internal(
        &self,
        request: TransferPrivateRoomOwnership,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.transfer_ownership";
        let actor_matrix = active_actor_matrix_user(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        let target_matrix = self
            .principal_matrix_user(request.target_principal_id, OPERATION)
            .await?;
        let matrix_room = matrix_room_id(&snapshot, OPERATION)?;

        if snapshot.room().owner_principal_id() == request.target_principal_id {
            return self
                .reconcile_completed_transfer(
                    snapshot,
                    &request,
                    &matrix_room,
                    &actor_matrix,
                    &target_matrix,
                    OPERATION,
                )
                .await;
        }

        let target_previous_speaks = snapshot
            .room()
            .member(request.target_principal_id)
            .is_some_and(|member| member.permissions().speak());
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let changed = room
            .transfer_ownership(
                request.actor.principal_id,
                request.target_principal_id,
                request.former_owner_permissions,
            )
            .map_err(|error| domain(OPERATION, &error))?;
        if !request.former_owner_permissions.speak() {
            self.set_speaking(&matrix_room, &actor_matrix, false, OPERATION)
                .await?;
            self.govern_member_agents(
                &matrix_room,
                request.actor.principal_id,
                MemberAgentChange::Speak(false),
                OPERATION,
            )
            .await?;
        }
        self.save_if_changed(&room, expected, changed, OPERATION)
            .await?;
        if !target_previous_speaks {
            self.set_speaking(&matrix_room, &target_matrix, true, OPERATION)
                .await?;
        }
        if request.former_owner_permissions.speak() {
            self.set_speaking(&matrix_room, &actor_matrix, true, OPERATION)
                .await?;
        }
        replace_room(snapshot, room, OPERATION)
    }

    async fn archive_internal(
        &self,
        request: ArchivePrivateRoom,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.archive";
        ensure_active_room_actor(&request.actor, self.clock.now(), OPERATION)?;
        // 本机 Agent 替人建的房间由人在网页上关（要最近验证过身份）；只有网络 Agent 能关自己的房间。
        if matches!(request.actor, PrivateRoomActor::Device(_)) {
            return Err(failure(
                OPERATION,
                PrivateRoomFailureStage::Validation,
                PrivateRoomFailureKind::Forbidden,
            ));
        }
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        self.archive_as_owner(snapshot, request.actor.principal_id(), OPERATION)
            .await
    }

    async fn archive_owned_by_internal(&self, owner: PrincipalId) -> PrivateRoomResult<usize> {
        const OPERATION: &str = "private_room.archive_owned";
        let rooms = self
            .store
            .list_for_principal(owner)
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        let mut archived = 0;
        for snapshot in rooms {
            let room = snapshot.room();
            if room.owner_principal_id() != owner
                || room.status() == PrivateRoomLifecycleStatus::Archived
            {
                continue;
            }
            self.archive_as_owner(snapshot, owner, OPERATION).await?;
            archived += 1;
        }
        Ok(archived)
    }

    /// 以房主的身份归档：先锁死 Matrix 上的发言（重复设也不出错），再存目录状态。存失败时
    /// 房间还算在用，再来一次会重走一遍。
    async fn archive_as_owner(
        &self,
        snapshot: PrivateRoomSnapshot,
        owner: PrincipalId,
        operation: &'static str,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        let expected = snapshot.room().version();
        let mut room = snapshot.room().clone();
        let changed = room
            .archive(owner)
            .map_err(|error| domain(operation, &error))?;
        let matrix_room = matrix_room_id(&snapshot, operation)?;
        self.matrix
            .archive(&matrix_room)
            .await
            .map_err(|error| matrix(operation, error))?;
        self.save_if_changed(&room, expected, changed, operation)
            .await?;
        replace_room(snapshot, room, operation)
    }

    async fn rename_internal(
        &self,
        request: RenamePrivateRoom,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        const OPERATION: &str = "private_room.rename";
        ensure_active_actor(&request.actor, self.clock.now(), OPERATION)?;
        let snapshot = self.load(request.catalog_id, OPERATION).await?;
        snapshot
            .room()
            .authorize_rename(request.actor.principal_id)
            .map_err(|error| domain(OPERATION, &error))?;
        let name = request.name.trim().to_owned();
        if name == snapshot.catalog().name() {
            return Ok(snapshot);
        }
        let matrix_room = matrix_room_id(&snapshot, OPERATION)?;
        let renamed = snapshot
            .renaming(name)
            .map_err(|error| domain(OPERATION, &error))?;
        // 先改 Matrix 房间名：它可以重复设置；目录写失败时重试会再走一遍。
        self.matrix
            .set_name(&matrix_room, renamed.catalog().name())
            .await
            .map_err(|error| matrix(OPERATION, error))?;
        self.store
            .rename(
                request.catalog_id,
                renamed.catalog().name(),
                self.clock.now(),
            )
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        Ok(renamed)
    }

    /// 人建房不带建房的 Agent；Agent 建房必须带，且是操作者名下还在用的 Agent，也不顺带邀请人
    /// （之后一个个邀请）。
    async fn ensure_creator(
        &self,
        request: &CreatePrivateRoom,
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        let agent_id = match (request.actor.is_agent(), request.created_by_agent) {
            (false, None) => return Ok(()),
            (true, Some(agent_id)) if request.invitations.is_empty() => agent_id,
            _ => {
                return Err(failure(
                    operation,
                    PrivateRoomFailureStage::Validation,
                    PrivateRoomFailureKind::InvalidRequest,
                ));
            }
        };
        let owned = self
            .agents
            .owns_active_agent(request.actor.principal_id(), agent_id)
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Directory, &error))?;
        if owned {
            Ok(())
        } else {
            Err(failure(
                operation,
                PrivateRoomFailureStage::Validation,
                PrivateRoomFailureKind::Forbidden,
            ))
        }
    }

    async fn prepare_creation(
        &self,
        request: &CreatePrivateRoom,
        owner_matrix_user: Option<MatrixUserId>,
        operation: &'static str,
    ) -> PrivateRoomResult<PreparedPrivateRoomCreation> {
        let owner = request.actor.principal_id();
        let mut room = match request.created_by_agent {
            Some(agent_id) => PrivateRoom::create_by_agent(request.catalog_id, owner, agent_id),
            None => PrivateRoom::create(request.catalog_id, owner),
        };
        let mut invited_principals = BTreeSet::new();
        let mut speaking_assignments = Vec::new();
        let mut matrix_invites = Vec::new();
        let mut unique_matrix_users = BTreeSet::new();
        // 网络 Agent 的合成主体没有自己的 Matrix 账号：不邀请房主本人，房主由它的 Agent 代表进房间。
        if let Some(owner_matrix_user) = owner_matrix_user {
            unique_matrix_users.insert(owner_matrix_user.as_str().to_owned());
            speaking_assignments.push(PrivateMatrixSpeakingAssignment::new(
                owner_matrix_user.clone(),
                true,
            ));
            matrix_invites.push(owner_matrix_user);
        }

        for invitation in &request.invitations {
            if invitation.principal_id == owner
                || !invited_principals.insert(invitation.principal_id)
            {
                return Err(failure(
                    operation,
                    PrivateRoomFailureStage::Validation,
                    PrivateRoomFailureKind::InvalidRequest,
                ));
            }
            room.invite(owner, invitation.principal_id, invitation.permissions)
                .map_err(|error| domain(operation, &error))?;
            let user_id = self
                .principal_matrix_user(invitation.principal_id, operation)
                .await?;
            if !unique_matrix_users.insert(user_id.as_str().to_owned()) {
                return Err(failure(
                    operation,
                    PrivateRoomFailureStage::Directory,
                    PrivateRoomFailureKind::Conflict,
                ));
            }
            speaking_assignments.push(PrivateMatrixSpeakingAssignment::new(
                user_id.clone(),
                invitation.permissions.speak(),
            ));
            matrix_invites.push(user_id);
        }

        for reader in &self.trusted_matrix_readers {
            if unique_matrix_users.insert(reader.as_str().to_owned()) {
                speaking_assignments
                    .push(PrivateMatrixSpeakingAssignment::new(reader.clone(), false));
                matrix_invites.push(reader.clone());
            }
        }

        let alias = private_room_alias(request.catalog_id, operation)?;
        let mut matrix_request = MatrixCreateRoom::new(
            Some(request.name.clone()),
            (!request.description.is_empty()).then(|| request.description.clone()),
            MatrixRoomVisibility::Private,
            MatrixRoomPreset::PrivateChat,
            false,
            matrix_invites,
        )
        .map_err(|error| domain(operation, &error))?
        .with_alias_localpart(alias.clone())
        .with_end_to_end_encryption()
        .with_power_profile(MatrixRoomPowerProfile::ManagedPrivate);
        if let Some(retention_days) = request.retention_days {
            matrix_request = matrix_request
                .with_retention_days(retention_days)
                .map_err(|error| domain(operation, &error))?;
        }
        let matrix = PrivateMatrixRoomCreation::new(matrix_request, alias)
            .map_err(|error| domain(operation, &error))?;
        Ok(PreparedPrivateRoomCreation {
            room,
            matrix,
            speaking_assignments,
        })
    }

    fn build_snapshot(
        &self,
        request: &CreatePrivateRoom,
        room: PrivateRoom,
        matrix_room_id: &MatrixRoomId,
        operation: &'static str,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        let catalog = RoomCatalog::new(
            request.catalog_id,
            RoomCatalogFields {
                kind: RoomCatalogKind::PrivateRoom,
                slug: None,
                name: request.name.clone(),
                description: request.description.clone(),
                language: None,
                matrix_space_id: None,
                owner_principal_id: Some(request.actor.principal_id()),
                visibility: RoomCatalogVisibility::Private,
                retention_days: request.retention_days,
                status: RoomCatalogStatus::Active,
            },
        )
        .map_err(|error| domain(operation, &error))?;
        let matrix_reference = MatrixRoomReference::new(matrix_room_id.as_str().to_owned())
            .map_err(|error| domain(operation, &error))?;
        let instance = RoomInstance::restore(
            self.identifiers.room_instance_id(),
            RoomInstanceFields {
                catalog_id: request.catalog_id,
                matrix_room_id: matrix_reference,
                region: None,
                capacity: RoomCapacity::standard(),
                projected_member_count: 0,
                allocated_slots: 0,
                activity_score_millis: 0,
                state: RoomInstanceState::Active,
            },
        )
        .map_err(|error| domain(operation, &error))?;
        PrivateRoomSnapshot::new(catalog, instance, room).map_err(|error| domain(operation, &error))
    }

    async fn find(
        &self,
        catalog_id: RoomCatalogId,
        operation: &'static str,
    ) -> PrivateRoomResult<Option<PrivateRoomSnapshot>> {
        self.store
            .find_by_catalog(catalog_id)
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Persistence, &error))
    }

    async fn load(
        &self,
        catalog_id: RoomCatalogId,
        operation: &'static str,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        self.find(catalog_id, operation).await?.ok_or_else(|| {
            failure(
                operation,
                PrivateRoomFailureStage::Persistence,
                PrivateRoomFailureKind::NotFound,
            )
        })
    }

    async fn principal_matrix_user(
        &self,
        principal_id: PrincipalId,
        operation: &'static str,
    ) -> PrivateRoomResult<MatrixUserId> {
        self.principals
            .matrix_user_id(principal_id)
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Directory, &error))?
            .ok_or_else(|| {
                failure(
                    operation,
                    PrivateRoomFailureStage::Directory,
                    PrivateRoomFailureKind::NotFound,
                )
            })
    }

    async fn save_if_changed(
        &self,
        room: &PrivateRoom,
        expected: AggregateVersion,
        changed: bool,
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        if !changed {
            return Ok(());
        }
        self.store
            .save(room, expected, self.clock.now())
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Persistence, &error))
    }

    async fn require_matrix_membership(
        &self,
        snapshot: &PrivateRoomSnapshot,
        user_id: &MatrixUserId,
        requirement: MatrixMembershipRequirement,
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        let matrix_room = matrix_room_id(snapshot, operation)?;
        let membership = self
            .matrix
            .membership(&matrix_room, user_id)
            .await
            .map_err(|error| matrix(operation, error))?;
        if requirement.matches(membership) {
            Ok(())
        } else {
            Err(failure(
                operation,
                PrivateRoomFailureStage::Matrix,
                PrivateRoomFailureKind::Conflict,
            ))
        }
    }

    async fn set_speaking(
        &self,
        room_id: &MatrixRoomId,
        user_id: &MatrixUserId,
        allowed: bool,
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        self.matrix
            .set_speaking(room_id, user_id, allowed)
            .await
            .map_err(|error| matrix(operation, error))
    }

    /// 让某个成员名下已在房间里的 Agent 跟随该成员：能发言就能发言，不能就只能旁观，成员离开
    /// 房间时一并移出。Agent 不是成员，这是它们在 Matrix 上唯一的能力来源；入场时授予的发言级别
    /// 必须在这里收回，否则成员被收回发言或移除后，它的 Agent 仍能在房间里说话、读消息。
    async fn govern_member_agents(
        &self,
        matrix_room: &MatrixRoomId,
        principal_id: PrincipalId,
        change: MemberAgentChange,
        operation: &'static str,
    ) -> PrivateRoomResult<()> {
        let agents = self
            .agents
            .agent_matrix_users(principal_id)
            .await
            .map_err(|error| repository(operation, PrivateRoomFailureStage::Directory, &error))?;
        for agent in &agents {
            let membership = self
                .matrix
                .membership(matrix_room, agent)
                .await
                .map_err(|error| matrix(operation, error))?;
            // 只处理确实在房间里（已加入或受邀）的 Agent，不给无关用户写进权限表。
            if !matches!(
                membership,
                Some(PrivateMatrixMembership::Joined | PrivateMatrixMembership::Invited)
            ) {
                continue;
            }
            match change {
                MemberAgentChange::Speak(allowed) => {
                    self.matrix.set_speaking(matrix_room, agent, allowed).await
                }
                MemberAgentChange::Evict => self.matrix.kick(matrix_room, agent).await,
            }
            .map_err(|error| matrix(operation, error))?;
        }
        Ok(())
    }

    async fn reconcile_completed_transfer(
        &self,
        snapshot: PrivateRoomSnapshot,
        request: &TransferPrivateRoomOwnership,
        matrix_room: &MatrixRoomId,
        actor_matrix: &MatrixUserId,
        target_matrix: &MatrixUserId,
        operation: &'static str,
    ) -> PrivateRoomResult<PrivateRoomSnapshot> {
        if request.actor.principal_id == request.target_principal_id {
            self.set_speaking(matrix_room, target_matrix, true, operation)
                .await?;
            return Ok(snapshot);
        }
        let former_matches = snapshot
            .room()
            .member(request.actor.principal_id)
            .is_some_and(|member| member.permissions() == request.former_owner_permissions);
        if !former_matches {
            return Err(failure(
                operation,
                PrivateRoomFailureStage::Validation,
                PrivateRoomFailureKind::Conflict,
            ));
        }
        self.set_speaking(matrix_room, target_matrix, true, operation)
            .await?;
        self.set_speaking(
            matrix_room,
            actor_matrix,
            request.former_owner_permissions.speak(),
            operation,
        )
        .await?;
        if !request.former_owner_permissions.speak() {
            self.govern_member_agents(
                matrix_room,
                request.actor.principal_id,
                MemberAgentChange::Speak(false),
                operation,
            )
            .await?;
        }
        Ok(snapshot)
    }
}

impl PrivateRoomUseCases for PrivateRoomService {
    fn create(
        &self,
        request: CreatePrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.create_internal(request))
    }

    fn inspect(
        &self,
        request: InspectPrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.inspect_internal(request))
    }

    fn origin(
        &self,
        request: InspectPrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomOrigin>> {
        Box::pin(self.origin_internal(request))
    }

    fn list(
        &self,
        request: ListPrivateRooms,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<PrivateRoomSnapshot>>> {
        Box::pin(self.list_internal(request))
    }

    fn list_for_account(
        &self,
        request: ListPrivateRoomsForAccount,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<PrivateRoomSnapshot>>> {
        Box::pin(self.list_for_account_internal(request))
    }

    fn invite(
        &self,
        request: InvitePrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.invite_internal(request))
    }

    fn accept(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.accept_internal(request))
    }

    fn decline(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.decline_internal(request))
    }

    fn leave(
        &self,
        request: PrivateRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.leave_internal(request))
    }

    fn remove(
        &self,
        request: GovernPrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.remove_internal(request))
    }

    fn ban(
        &self,
        request: GovernPrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.ban_internal(request))
    }

    fn unban(
        &self,
        request: GovernPrivateRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.unban_internal(request))
    }

    fn update_permissions(
        &self,
        request: ChangePrivateRoomPermissions,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.update_permissions_internal(request))
    }

    fn transfer_ownership(
        &self,
        request: TransferPrivateRoomOwnership,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.transfer_ownership_internal(request))
    }

    fn archive(
        &self,
        request: ArchivePrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.archive_internal(request))
    }

    fn rename(
        &self,
        request: RenamePrivateRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.rename_internal(request))
    }
}

impl PrivateRoomLinkInvitations for PrivateRoomService {
    fn invite_by_link(
        &self,
        request: InviteByLink,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        Box::pin(self.invite_by_link_internal(request))
    }
}

impl PrivateRoomRetirement for PrivateRoomService {
    fn archive_owned_by(&self, owner: PrincipalId) -> PortFuture<'_, PrivateRoomResult<usize>> {
        Box::pin(self.archive_owned_by_internal(owner))
    }
}

#[derive(Debug, Clone, Copy)]
enum MatrixMembershipRequirement {
    Joined,
    AbsentOrLeft,
}

struct PreparedPrivateRoomCreation {
    room: PrivateRoom,
    matrix: PrivateMatrixRoomCreation,
    speaking_assignments: Vec<PrivateMatrixSpeakingAssignment>,
}

impl MatrixMembershipRequirement {
    const fn matches(self, membership: Option<PrivateMatrixMembership>) -> bool {
        match self {
            Self::Joined => matches!(membership, Some(PrivateMatrixMembership::Joined)),
            Self::AbsentOrLeft => {
                matches!(membership, None | Some(PrivateMatrixMembership::Left))
            }
        }
    }
}

fn ensure_same_creation(
    request: &CreatePrivateRoom,
    existing: PrivateRoomSnapshot,
    operation: &'static str,
) -> PrivateRoomResult<PrivateRoomSnapshot> {
    let same = existing.room().owner_principal_id() == request.actor.principal_id()
        && existing.room().created_by_agent() == request.created_by_agent
        && existing.catalog().name() == request.name
        && existing.catalog().description() == request.description
        && existing.catalog().retention_days() == request.retention_days;
    if same {
        Ok(existing)
    } else {
        Err(failure(
            operation,
            PrivateRoomFailureStage::Validation,
            PrivateRoomFailureKind::Conflict,
        ))
    }
}

fn private_room_alias(
    catalog_id: RoomCatalogId,
    operation: &'static str,
) -> PrivateRoomResult<MatrixRoomAliasLocalpart> {
    MatrixRoomAliasLocalpart::new(format!(
        "agent-room-private-{}",
        catalog_id.as_uuid().simple()
    ))
    .map_err(|_| {
        failure(
            operation,
            PrivateRoomFailureStage::Validation,
            PrivateRoomFailureKind::Internal,
        )
    })
}

fn ensure_active_actor(
    actor: &AuthenticatedPrincipal,
    now: agent_room_domain::time::UtcMillis,
    operation: &'static str,
) -> PrivateRoomResult<()> {
    if now < actor.expires_at {
        Ok(())
    } else {
        Err(failure(
            operation,
            PrivateRoomFailureStage::Validation,
            PrivateRoomFailureKind::Forbidden,
        ))
    }
}

fn active_actor_matrix_user(
    actor: &AuthenticatedPrincipal,
    now: agent_room_domain::time::UtcMillis,
    operation: &'static str,
) -> PrivateRoomResult<MatrixUserId> {
    ensure_active_actor(actor, now, operation)?;
    MatrixUserId::new(actor.matrix_user_id.clone()).map_err(|_| {
        failure(
            operation,
            PrivateRoomFailureStage::Validation,
            PrivateRoomFailureKind::Internal,
        )
    })
}

fn ensure_active_room_actor(
    actor: &PrivateRoomActor,
    now: agent_room_domain::time::UtcMillis,
    operation: &'static str,
) -> PrivateRoomResult<()> {
    if now < actor.expires_at() {
        Ok(())
    } else {
        Err(failure(
            operation,
            PrivateRoomFailureStage::Validation,
            PrivateRoomFailureKind::Forbidden,
        ))
    }
}

/// 私人房间操作者还在有效期内时，给出它自己的 Matrix 账号；网络 Agent 没有，给空。
fn active_room_actor_matrix_user(
    actor: &PrivateRoomActor,
    now: agent_room_domain::time::UtcMillis,
    operation: &'static str,
) -> PrivateRoomResult<Option<MatrixUserId>> {
    ensure_active_room_actor(actor, now, operation)?;
    actor
        .own_matrix_user_id()
        .map(|user| {
            MatrixUserId::new(user.to_owned()).map_err(|_| {
                failure(
                    operation,
                    PrivateRoomFailureStage::Validation,
                    PrivateRoomFailureKind::Internal,
                )
            })
        })
        .transpose()
}

fn matrix_room_id(
    snapshot: &PrivateRoomSnapshot,
    operation: &'static str,
) -> PrivateRoomResult<MatrixRoomId> {
    MatrixRoomId::new(snapshot.instance().matrix_room_id().as_str().to_owned()).map_err(|_| {
        failure(
            operation,
            PrivateRoomFailureStage::Persistence,
            PrivateRoomFailureKind::Internal,
        )
    })
}

fn replace_room(
    snapshot: PrivateRoomSnapshot,
    room: PrivateRoom,
    operation: &'static str,
) -> PrivateRoomResult<PrivateRoomSnapshot> {
    snapshot
        .replacing_room(room)
        .map_err(|error| domain(operation, &error))
}

/// 成员能力变化时，其名下 Agent 在 Matrix 上要跟着做的调整。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemberAgentChange {
    Speak(bool),
    Evict,
}
