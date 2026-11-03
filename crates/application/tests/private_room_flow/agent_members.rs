//! 凭口令或敲门进来的 Agent 成员：治理踢出、封禁一个人时连他名下的一起移出，网络 Agent 只移出它自己。

use agent_room_application::ports::{PrivateRoomAgentEviction, PrivateRoomAgentMemberRecord};
use agent_room_domain::join_codes::PrivateRoomAgentMemberStatus;

use super::*;

#[derive(Default)]
pub(super) struct TestAgentMembers {
    pub(super) members: Mutex<Vec<PrivateRoomAgentMemberRecord>>,
}

impl TestAgentMembers {
    /// 记一个已加入的 Agent 成员，能看能说。
    pub(super) fn join(&self, catalog_id: RoomCatalogId, agent_id: AgentId, user: &MatrixUserId) {
        self.members
            .lock()
            .expect("Agent 成员锁正常")
            .push(PrivateRoomAgentMemberRecord {
                catalog_id,
                agent_id,
                display_name: "门口的 Agent".to_owned(),
                matrix_user_id: user.clone(),
                owner_display_name: None,
                status: PrivateRoomAgentMemberStatus::Joined,
                permissions: speaker_permissions(),
                joined_at: UtcMillis::new(0).expect("时间有效"),
                status_changed_at: UtcMillis::new(0).expect("时间有效"),
            });
    }

    pub(super) fn status(&self, agent_id: AgentId) -> PrivateRoomAgentMemberStatus {
        self.members
            .lock()
            .expect("Agent 成员锁正常")
            .iter()
            .find(|member| member.agent_id == agent_id)
            .expect("记过这个 Agent")
            .status
    }
}

impl PrivateRoomAgentEviction for TestAgentMembers {
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
        let mut members = self.members.lock().expect("Agent 成员锁正常");
        let removed = members
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
            .is_some();
        Box::pin(async move { Ok(removed) })
    }
}
