//! 网络 Agent 看到的“正在输入”：同步带回来时记在这个进程里，等消息时据此判断叫醒它的人
//! 是不是还在打字（`specs/agent-reading/waiting.md`）。重启以后等下一次变化再知道。

use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
};

use agent_room_application::ports::MatrixSyncBatch;
use agent_room_bridge_ipc::{typing::TypingRooms, wake::Typist};
use agent_room_domain::ids::NetworkAgentId;

#[derive(Default)]
pub(super) struct AgentTyping {
    agents: Mutex<HashMap<NetworkAgentId, TypingRooms>>,
}

impl AgentTyping {
    /// 记下一次同步带回的“正在输入”；什么也没记着的 Agent 不留。
    pub(super) fn record(&self, agent: NetworkAgentId, batch: &MatrixSyncBatch, now_ms: i64) {
        let mut agents = self.agents.lock().unwrap_or_else(PoisonError::into_inner);
        if batch.rooms().iter().all(|room| room.typing().is_none()) && !agents.contains_key(&agent)
        {
            return;
        }
        let rooms = agents.entry(agent).or_default();
        rooms.record(batch, now_ms);
        if rooms.is_empty() {
            agents.remove(&agent);
        }
    }

    /// 还在打字的和刚停下的。
    pub(super) fn typists(&self, agent: NetworkAgentId, now_ms: i64) -> Vec<Typist> {
        self.agents
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&agent)
            .map(|rooms| rooms.typists(now_ms))
            .unwrap_or_default()
    }

    /// 停用以后不再记着。
    pub(super) fn forget(&self, agent: NetworkAgentId) {
        self.agents
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&agent);
    }
}

#[cfg(test)]
mod tests {
    use agent_room_application::ports::{
        MatrixRoomId, MatrixRoomSync, MatrixRoomSyncKind, MatrixSyncBatch, MatrixSyncToken,
        MatrixUserId,
    };
    use agent_room_domain::ids::NetworkAgentId;
    use uuid::Uuid;

    use super::AgentTyping;

    fn ada_typing(typing: &[&str]) -> MatrixSyncBatch {
        let room = MatrixRoomSync::new(
            MatrixRoomId::new("!a:matrix.test").unwrap(),
            MatrixRoomSyncKind::Joined,
            false,
            None,
            Vec::new(),
            Vec::new(),
        )
        .with_typing(
            typing
                .iter()
                .map(|user| MatrixUserId::new(*user).unwrap())
                .collect(),
        );
        MatrixSyncBatch::new(MatrixSyncToken::new("s1").unwrap(), vec![room])
    }

    #[test]
    fn 每个网络_agent_各记各的_停下一会儿以后和停用以后不再记着() {
        let typing = AgentTyping::default();
        let agent = NetworkAgentId::from_uuid(Uuid::now_v7());
        let other = NetworkAgentId::from_uuid(Uuid::now_v7());
        typing.record(agent, &ada_typing(&["@ada:matrix.test"]), 1_000);
        assert!(typing.typists(agent, 2_000)[0].typing);
        assert!(typing.typists(other, 2_000).is_empty());
        typing.record(agent, &ada_typing(&[]), 3_000);
        let stopped = typing.typists(agent, 3_000);
        assert!(
            !stopped[0].typing && stopped[0].at_ms == 3_000,
            "记下停下的时刻"
        );
        // 之后的同步没再带回正在输入，停下一分钟以后照样清掉。
        let quiet = MatrixSyncBatch::new(MatrixSyncToken::new("s2").unwrap(), Vec::new());
        typing.record(agent, &quiet, 64_000);
        assert!(typing.agents.lock().unwrap().is_empty());

        typing.record(agent, &ada_typing(&["@bob:matrix.test"]), 70_000);
        typing.forget(agent);
        assert!(typing.typists(agent, 70_000).is_empty());
    }
}
