//! 网络 Agent 看到的“正在输入”：同步带回来时记在这个进程里，等消息时据此判断叫醒它的人
//! 是不是还在打字（`specs/agent-reading/waiting.md`）。重启以后等下一次变化再知道。

use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
};

use agent_room_application::ports::MatrixSyncBatch;
use agent_room_bridge_ipc::{typing::TypingRooms, wake::IpcTyping};
use agent_room_domain::ids::NetworkAgentId;

#[derive(Default)]
pub(super) struct AgentTyping {
    agents: Mutex<HashMap<NetworkAgentId, TypingRooms>>,
}

impl AgentTyping {
    /// 记下一次同步带回的“正在输入”；没人打字的 Agent 不留。
    pub(super) fn record(&self, agent: NetworkAgentId, batch: &MatrixSyncBatch, now_ms: i64) {
        if batch.rooms().iter().all(|room| room.typing().is_none()) {
            return;
        }
        let mut agents = self.agents.lock().unwrap_or_else(PoisonError::into_inner);
        let rooms = agents.entry(agent).or_default();
        rooms.record(batch, now_ms);
        if rooms.is_empty() {
            agents.remove(&agent);
        }
    }

    /// 此刻还算在打字的。
    pub(super) fn now(&self, agent: NetworkAgentId, now_ms: i64) -> Vec<IpcTyping> {
        self.agents
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&agent)
            .map(|rooms| rooms.now(None, now_ms))
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
    fn 每个网络_agent_各记各的_停用以后不再记着() {
        let typing = AgentTyping::default();
        let agent = NetworkAgentId::from_uuid(Uuid::now_v7());
        let other = NetworkAgentId::from_uuid(Uuid::now_v7());
        typing.record(agent, &ada_typing(&["@ada:matrix.test"]), 1_000);
        assert_eq!(typing.now(agent, 2_000).len(), 1);
        assert!(typing.now(other, 2_000).is_empty());
        typing.record(agent, &ada_typing(&[]), 3_000);
        assert!(typing.agents.lock().unwrap().is_empty(), "都停了就不留");

        typing.record(agent, &ada_typing(&["@bob:matrix.test"]), 4_000);
        typing.forget(agent);
        assert!(typing.now(agent, 4_000).is_empty());
    }
}
