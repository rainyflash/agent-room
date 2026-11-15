//! 子房间第 2d-3 步（`specs/redesign/sub-rooms.md` 的“请进来、移出和封禁”）：管得了这一间的人
//! （这一间的管理员、父房间的房主和管理员，进没进来都一样）把父房间里的人直接请进来、移出、封禁、
//! 解封。
//!
//! 管这一间的人不一定在这一间里，房间的规则（`PrivateRoom::remove_member` 这些）要求动手的人是
//! 这间房里有“管理”的成员。所以谁动得了谁先照子房间的规则判断好
//! （[`authorize_sub_room_governance`]），改成员记录时以这一间的房主（就是父房间的房主，他在每一间
//! 里、权限全有）的名义改；记身份变动时写的还是真正动手的人。

use agent_room_domain::{
    private_rooms::{PrivateRoomLifecycleStatus, PrivateRoomMember, PrivateRoomMembershipStatus},
    room_join_requests::JoinRequestStatus,
    sub_rooms::{
        SubRoomEntryRefusal, SubRoomGovernanceRefusal, authorize_sub_room_admission,
        authorize_sub_room_governance, manages_sub_room,
    },
};

use super::{
    super::{
        GovernSubRoomMember, PrivateRoomFailure, PrivateRoomFailureKind, PrivateRoomFailureStage,
        PrivateRoomResult, SubRoomRefusal,
        failure::{domain, failure, matrix, repository},
    },
    MemberAgentChange, PrivateRoomService, ensure_active_actor, matrix_room_id,
    sub_room_membership::{LoadedSubRoom, SubRoomEntrant, entry_refused},
    sub_rooms::refused,
};

/// 要动的是哪一样。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Governance {
    Remove,
    Ban,
    Unban,
}

impl Governance {
    const fn operation(self) -> &'static str {
        match self {
            Self::Remove => "sub_room.member.remove",
            Self::Ban => "sub_room.member.ban",
            Self::Unban => "sub_room.member.unban",
        }
    }
}

impl PrivateRoomService {
    /// 把父房间里的人直接请进来，哪种门都行。和他自己点“加入”同一段
    /// （[`PrivateRoomService::admit_into_sub_room`]）：被禁言着的不给发言级别，最后才邀请。他在等的
    /// 申请记成批准。
    pub(super) async fn admit_sub_room_member_internal(
        &self,
        request: GovernSubRoomMember,
    ) -> PrivateRoomResult<()> {
        const OPERATION: &str = "sub_room.member.admit";
        let now = self.clock.now();
        ensure_active_actor(&request.actor, now, OPERATION)?;
        let actor = request.actor.principal_id;
        let LoadedSubRoom {
            parent, sub_room, ..
        } = self.load_managed_sub_room(&request, OPERATION).await?;
        if !manages_sub_room(parent.room(), sub_room.room(), actor) {
            return Err(refused(OPERATION, SubRoomRefusal::ManagersOnly));
        }
        let permissions =
            authorize_sub_room_admission(parent.room(), sub_room.room(), request.target)
                .map_err(|refusal| target_refused(OPERATION, refusal))?;
        let matrix_user = self
            .principal_matrix_user(request.target, OPERATION)
            .await?;
        self.admit_into_sub_room(
            request.parent_catalog_id,
            sub_room,
            &SubRoomEntrant {
                principal: request.target,
                matrix_user: &matrix_user,
                permissions,
            },
            now,
            OPERATION,
        )
        .await?;
        // 请进来就等于批准了。没有在等的、刚被别人处理了的都不算错：他已经在里面了。
        self.join_requests
            .decide_join_request(
                request.catalog_id,
                request.target,
                JoinRequestStatus::Approved,
                Some(actor),
                now,
            )
            .await
            .map_err(|error| repository(OPERATION, PrivateRoomFailureStage::Persistence, &error))?;
        Ok(())
    }

    pub(super) async fn remove_sub_room_member_internal(
        &self,
        request: GovernSubRoomMember,
    ) -> PrivateRoomResult<()> {
        self.govern_sub_room_member(request, Governance::Remove)
            .await
    }

    pub(super) async fn ban_sub_room_member_internal(
        &self,
        request: GovernSubRoomMember,
    ) -> PrivateRoomResult<()> {
        self.govern_sub_room_member(request, Governance::Ban).await
    }

    pub(super) async fn unban_sub_room_member_internal(
        &self,
        request: GovernSubRoomMember,
    ) -> PrivateRoomResult<()> {
        self.govern_sub_room_member(request, Governance::Unban)
            .await
    }

    /// 移出、封禁、解封。先在聊天服务器上做（都可以重复做），再存成员记录：存不上的，再点一次接得上。
    ///
    /// - 移出：他不在里面就什么都不做。踢出去，他的 Agent 一起移出。
    /// - 封禁：他得是父房间里已加入的人，或者在这一间里有过记录（被父房间移出过的人也封得了，免得
    ///   以后再进父房间时能直接进来）。封禁的时候在等的申请由存储在同一个事务里作废。
    /// - 解封：回到“已移出”，之后能再申请、再被请进来。没封着的什么都不做。
    async fn govern_sub_room_member(
        &self,
        request: GovernSubRoomMember,
        governance: Governance,
    ) -> PrivateRoomResult<()> {
        let operation = governance.operation();
        ensure_active_actor(&request.actor, self.clock.now(), operation)?;
        let actor = request.actor.principal_id;
        let target = request.target;
        let LoadedSubRoom {
            parent, sub_room, ..
        } = self.load_managed_sub_room(&request, operation).await?;
        authorize_sub_room_governance(parent.room(), sub_room.room(), actor, target)
            .map_err(|refusal| governance_refused(operation, refusal))?;
        let record = sub_room.room().member(target);
        let skip = match governance {
            Governance::Remove => {
                !record.is_some_and(|member| member.status().carries_permissions())
            }
            Governance::Ban => {
                let in_parent = parent
                    .room()
                    .member(target)
                    .is_some_and(PrivateRoomMember::has_joined);
                if record.is_none() && !in_parent {
                    return Err(refused(operation, SubRoomRefusal::NotInParent));
                }
                false
            }
            Governance::Unban => {
                !record.is_some_and(|member| member.status() == PrivateRoomMembershipStatus::Banned)
            }
        };
        if skip {
            return Ok(());
        }
        // 以这一间的房主的名义改：谁动得了谁上面已经照子房间的规则判断过了。
        let owner = sub_room.room().owner_principal_id();
        let mut room = sub_room.room().clone();
        let changed = match governance {
            Governance::Remove => room.remove_member(owner, target),
            Governance::Ban => room.ban_member(owner, target),
            Governance::Unban => room.unban_member(owner, target),
        }
        .map_err(|error| domain(operation, &error))?;
        let target_matrix = self.principal_matrix_user(target, operation).await?;
        let matrix_room = matrix_room_id(&sub_room, operation)?;
        match governance {
            Governance::Remove => self.matrix.kick(&matrix_room, &target_matrix).await,
            Governance::Ban => self.matrix.ban(&matrix_room, &target_matrix).await,
            Governance::Unban => self.matrix.unban(&matrix_room, &target_matrix).await,
        }
        .map_err(|error| matrix(operation, error))?;
        if governance != Governance::Unban {
            // Agent 只移出不封禁，和整间房的封禁一样：封禁针对的是人。
            self.govern_member_agents(&matrix_room, target, MemberAgentChange::Evict, operation)
                .await?;
        }
        self.save_if_changed(sub_room.room(), &room, actor, changed, operation)
            .await
    }

    /// 读这一间，查房间都还在用。管不管得了由调的人判断：请进来只看管不管得了，移出、封禁还要看
    /// 对象是谁。
    async fn load_managed_sub_room(
        &self,
        request: &GovernSubRoomMember,
        operation: &'static str,
    ) -> PrivateRoomResult<LoadedSubRoom> {
        let loaded = self
            .load_sub_room(
                request.parent_catalog_id,
                request.catalog_id,
                request.actor.principal_id,
                operation,
            )
            .await?;
        let active = PrivateRoomLifecycleStatus::Active;
        let both_active =
            loaded.parent.room().status() == active && loaded.sub_room.room().status() == active;
        if !both_active {
            // 管不了的人也只看到“归档了”：归档的事父房间里的人本来就知道。
            return Err(refused(operation, SubRoomRefusal::Archived));
        }
        Ok(loaded)
    }
}

/// 请进来的对象进不来：说的是“他”，不是“你”。门不看（管这一间的人请进来哪种门都行），别的照
/// 他自己加入时的说法。
fn target_refused(operation: &'static str, refusal: SubRoomEntryRefusal) -> PrivateRoomFailure {
    match refusal {
        SubRoomEntryRefusal::NotInParent => refused(operation, SubRoomRefusal::NotInParent),
        SubRoomEntryRefusal::Banned => refused(operation, SubRoomRefusal::TargetBanned),
        other => entry_refused(operation, other),
    }
}

fn governance_refused(
    operation: &'static str,
    refusal: SubRoomGovernanceRefusal,
) -> PrivateRoomFailure {
    match refusal {
        SubRoomGovernanceRefusal::ManagersOnly => refused(operation, SubRoomRefusal::ManagersOnly),
        SubRoomGovernanceRefusal::Owner => refused(operation, SubRoomRefusal::OwnerStays),
        SubRoomGovernanceRefusal::PeerManager => refused(operation, SubRoomRefusal::PeerManager),
        SubRoomGovernanceRefusal::ParentAdmin => failure(
            operation,
            PrivateRoomFailureStage::Validation,
            PrivateRoomFailureKind::OwnerOnly,
        ),
        // 自己退出用“退出”。
        SubRoomGovernanceRefusal::Oneself => failure(
            operation,
            PrivateRoomFailureStage::Validation,
            PrivateRoomFailureKind::InvalidRequest,
        ),
    }
}
