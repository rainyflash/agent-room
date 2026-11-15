//! 子房间第 2f-2b 步（`specs/redesign/sub-rooms.md` 的“对账”）：父房间里移出凭口令或敲门进来的 Agent
//! 成员时，它在子房间里也跟着移出。存储在同一个事务里改记录、排上“踢出去”，事务外面照现在的记录做：
//! 不在里面了的踢出去，归档了的子房间里也踢，还在里面的不动。

use agent_room_application::ports::{
    ModerationMemberGateway, PrivateRoomAgentEviction, PrivateRoomAgentMemberRecord,
};
use agent_room_domain::{
    join_codes::PrivateRoomAgentMemberStatus, moderation::ModerationActionKind,
};

use super::{
    sync::{room, two_sub_rooms},
    *,
};

/// 和真实存储一样：父房间里移出时，它下面的子房间里已加入的那几条跟着移出，排上踢出去。
pub(super) struct FamilyAgentMembers {
    members: Mutex<Vec<PrivateRoomAgentMemberRecord>>,
    rooms: Arc<MemoryRooms>,
}

impl FamilyAgentMembers {
    pub(super) fn new(rooms: Arc<MemoryRooms>) -> Self {
        Self {
            members: Mutex::new(Vec::new()),
            rooms,
        }
    }

    /// 记一个已加入的 Agent 成员，能看能说。
    fn join(&self, catalog: u128, agent_id: AgentId, user: &MatrixUserId) {
        self.members
            .lock()
            .expect("Agent 成员锁正常")
            .push(PrivateRoomAgentMemberRecord {
                catalog_id: catalog_id(catalog),
                agent_id,
                display_name: "门口的 Agent".to_owned(),
                matrix_user_id: user.clone(),
                owner_display_name: None,
                status: PrivateRoomAgentMemberStatus::Joined,
                permissions: permissions(&[
                    PrivateRoomCapability::View,
                    PrivateRoomCapability::Speak,
                ]),
                joined_at: at(0),
                status_changed_at: at(0),
            });
    }

    fn status(&self, catalog: u128, agent_id: AgentId) -> Option<PrivateRoomAgentMemberStatus> {
        self.members
            .lock()
            .expect("Agent 成员锁正常")
            .iter()
            .find(|member| member.catalog_id == catalog_id(catalog) && member.agent_id == agent_id)
            .map(|member| member.status)
    }

    fn set_status(&self, catalog: u128, agent_id: AgentId, status: PrivateRoomAgentMemberStatus) {
        for member in self.members.lock().expect("Agent 成员锁正常").iter_mut() {
            if member.catalog_id == catalog_id(catalog) && member.agent_id == agent_id {
                member.status = status;
            }
        }
    }

    fn remove_in(&self, catalog_id: RoomCatalogId, agent_id: AgentId, now: UtcMillis) -> bool {
        self.members
            .lock()
            .expect("Agent 成员锁正常")
            .iter_mut()
            .find(|member| {
                member.catalog_id == catalog_id
                    && member.agent_id == agent_id
                    && member.status == PrivateRoomAgentMemberStatus::Joined
            })
            .map(|member| {
                member.status = PrivateRoomAgentMemberStatus::Removed;
                member.permissions = PrivateRoomPermissions::NONE;
                member.status_changed_at = now;
            })
            .is_some()
    }
}

impl PrivateRoomAgentEviction for FamilyAgentMembers {
    fn joined_agent_members(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomAgentMemberRecord>>> {
        let joined = self
            .members
            .lock()
            .expect("Agent 成员锁正常")
            .iter()
            .filter(|member| {
                member.catalog_id == catalog_id
                    && member.status == PrivateRoomAgentMemberStatus::Joined
            })
            .cloned()
            .collect();
        Box::pin(async move { Ok(joined) })
    }

    fn mark_agent_removed(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        let removed = self.remove_in(catalog_id, agent_id, now);
        for sub_room in self.rooms.sub_rooms_of(catalog_id) {
            if self.remove_in(sub_room, agent_id, now) {
                self.rooms.queue_agent_sync(sub_room, agent_id);
            }
        }
        Box::pin(async move { Ok(removed) })
    }

    fn agent_member_in(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomAgentMemberRecord>>> {
        let found = self
            .members
            .lock()
            .expect("Agent 成员锁正常")
            .iter()
            .find(|member| member.catalog_id == catalog_id && member.agent_id == agent_id)
            .cloned();
        Box::pin(async move { Ok(found) })
    }
}

fn at(millis: i64) -> UtcMillis {
    UtcMillis::new(millis).expect("时间有效")
}

fn agent(value: u128) -> AgentId {
    AgentId::from_uuid(uuid(70_000 + value))
}

fn agent_user(name: &str) -> MatrixUserId {
    MatrixUserId::new(format!("@{name}:matrix.test")).expect("用户有效")
}

/// 两间子房间；不在房间里的人名下有一个 Agent，凭口令进了父房间，又进了第一间子房间，聊天服务器上
/// 两间都在。
async fn agent_member_inside() -> (Rooms, AgentId, MatrixUserId) {
    let rooms = two_sub_rooms().await;
    let agent_id = agent(1);
    let user = agent_user("door-agent");
    rooms.agents.own(principal_id(OUTSIDER), agent_id);
    for catalog in [PARENT, 20] {
        rooms.agent_members.join(catalog, agent_id, &user);
        rooms
            .matrix
            .set_membership_in(&room(catalog), &user, PrivateMatrixMembership::Joined);
    }
    (rooms, agent_id, user)
}

#[tokio::test]
async fn 治理在父房间里踢出他时_他名下在子房间里的_agent_成员当场跟着出去() {
    let (rooms, agent_id, user) = agent_member_inside().await;

    ModerationMemberGateway::apply(
        &rooms.service,
        ModerationActionKind::Kick,
        catalog_id(PARENT),
        principal_id(OUTSIDER),
    )
    .await
    .expect("踢出");

    let kicked = format!("kick {}", user.as_str());
    assert!(rooms.matrix.changes_in(&room(PARENT)).contains(&kicked));
    assert_eq!(
        rooms.matrix.changes_in(&room(20)),
        [kicked],
        "这个请求结束前子房间里也做完了"
    );
    assert!(rooms.matrix.changes_in(&room(21)).is_empty());
    assert_eq!(
        rooms.agent_members.status(20, agent_id),
        Some(PrivateRoomAgentMemberStatus::Removed)
    );
    assert!(rooms.store.queued_sync().is_empty(), "不用等后台");
}

#[tokio::test]
async fn 后台照现在的记录做_不在里面了才踢_聊天服务器上已经不在的不踢_还在里面的不动() {
    let (rooms, agent_id, user) = agent_member_inside().await;
    rooms
        .agent_members
        .set_status(20, agent_id, PrivateRoomAgentMemberStatus::Removed);
    rooms.store.queue_agent_sync(catalog_id(20), agent_id);
    // 还在里面的那一间：排着也不动。
    rooms.agent_members.join(21, agent_id, &user);
    rooms.store.queue_agent_sync(catalog_id(21), agent_id);

    let outcome = rooms.sync().await;
    assert_eq!(outcome.done, 2);
    assert_eq!(
        rooms.matrix.changes_in(&room(20)),
        [format!("kick {}", user.as_str())]
    );
    assert!(rooms.matrix.changes_in(&room(21)).is_empty());

    // 再排一次：聊天服务器上已经出去了，不再踢。
    rooms.matrix.clear_changes();
    rooms.store.queue_agent_sync(catalog_id(20), agent_id);
    assert_eq!(rooms.sync().await.done, 1);
    assert!(rooms.matrix.changes_in(&room(20)).is_empty());
    assert!(rooms.store.queued_sync().is_empty());
}

#[tokio::test]
async fn 归档了的子房间里也踢_不设发言级别() {
    let (rooms, agent_id, user) = agent_member_inside().await;
    rooms.store.seed(&Seed {
        archived: true,
        ..Seed::open(22, "old")
    });
    rooms.agent_members.join(22, agent_id, &user);
    rooms
        .matrix
        .set_membership_in(&room(22), &user, PrivateMatrixMembership::Joined);
    rooms
        .agent_members
        .set_status(22, agent_id, PrivateRoomAgentMemberStatus::Removed);
    rooms.store.queue_agent_sync(catalog_id(22), agent_id);

    assert_eq!(rooms.sync().await.done, 1);
    assert_eq!(
        rooms.matrix.changes_in(&room(22)),
        [format!("kick {}", user.as_str())],
        "设一次发言级别就把归档的房间解开了"
    );
}
