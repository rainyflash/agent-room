//! 网络 Agent 看到的“正在输入”：同步带回来时记在这个进程里，等消息时据此判断叫醒它的人
//! 是不是还在打字（`specs/agent-reading/waiting.md`）。重启以后等下一次变化再知道。

use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
};

use agent_room_application::ports::MatrixSyncBatch;
use agent_room_bridge_ipc::wake::{IpcTyping, TYPING_TTL};
use agent_room_domain::ids::NetworkAgentId;

#[derive(Default)]
pub(super) struct TypingRooms {
    agents: Mutex<HashMap<NetworkAgentId, HashMap<String, Seen>>>,
}

/// 一个房间里此刻在打字的人，和什么时候听说的（Unix 毫秒）。
struct Seen {
    users: Vec<String>,
    at_ms: i64,
}

impl TypingRooms {
    /// 记下一次同步带回的“正在输入”：变了的房间整个换掉，没人打字的房间不留。
    pub(super) fn record(&self, agent: NetworkAgentId, batch: &MatrixSyncBatch, now_ms: i64) {
        let mut changed = batch
            .rooms()
            .iter()
            .filter_map(|room| Some((room.room_id().as_str(), room.typing()?)))
            .peekable();
        if changed.peek().is_none() {
            return;
        }
        let mut agents = self.agents.lock().unwrap_or_else(PoisonError::into_inner);
        let rooms = agents.entry(agent).or_default();
        for (room_id, users) in changed {
            if users.is_empty() {
                rooms.remove(room_id);
            } else {
                let users = users.iter().map(|user| user.as_str().to_owned()).collect();
                rooms.insert(
                    room_id.to_owned(),
                    Seen {
                        users,
                        at_ms: now_ms,
                    },
                );
            }
        }
        if rooms.is_empty() {
            agents.remove(&agent);
        }
    }

    /// 此刻还算在打字的：太久（`TYPING_TTL`）没再听说的不算，免得漏掉一次“停了”就一直等。
    pub(super) fn now(&self, agent: NetworkAgentId, now_ms: i64) -> Vec<IpcTyping> {
        let ttl = i64::try_from(TYPING_TTL.as_millis()).unwrap_or(i64::MAX);
        self.agents
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&agent)
            .map(|rooms| {
                rooms
                    .iter()
                    .filter(|(_, seen)| now_ms.saturating_sub(seen.at_ms) < ttl)
                    .map(|(room_id, seen)| IpcTyping {
                        room_id: room_id.clone(),
                        user_ids: seen.users.clone(),
                    })
                    .collect()
            })
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
    use agent_room_bridge_ipc::wake::IpcTyping;
    use agent_room_domain::ids::NetworkAgentId;
    use uuid::Uuid;

    use super::TypingRooms;

    fn batch(rooms: &[(&str, Option<&[&str]>)]) -> MatrixSyncBatch {
        let rooms = rooms
            .iter()
            .map(|(room_id, typing)| {
                let room = MatrixRoomSync::new(
                    MatrixRoomId::new(*room_id).unwrap(),
                    MatrixRoomSyncKind::Joined,
                    false,
                    None,
                    Vec::new(),
                    Vec::new(),
                );
                match typing {
                    Some(users) => room.with_typing(
                        users
                            .iter()
                            .map(|user| MatrixUserId::new(*user).unwrap())
                            .collect(),
                    ),
                    None => room,
                }
            })
            .collect();
        MatrixSyncBatch::new(MatrixSyncToken::new("s1").unwrap(), rooms)
    }

    #[test]
    fn 记下每个房间谁在打字_停了或太久没消息就不算() {
        let typing = TypingRooms::default();
        let agent = NetworkAgentId::from_uuid(Uuid::now_v7());
        let other = NetworkAgentId::from_uuid(Uuid::now_v7());
        typing.record(
            agent,
            &batch(&[
                ("!a:matrix.test", Some(&["@ada:matrix.test"])),
                ("!b:matrix.test", None),
            ]),
            1_000,
        );
        assert_eq!(
            typing.now(agent, 2_000),
            [IpcTyping {
                room_id: "!a:matrix.test".to_owned(),
                user_ids: vec!["@ada:matrix.test".to_owned()],
            }]
        );
        assert!(typing.now(other, 2_000).is_empty(), "各记各的");
        assert!(typing.now(agent, 31_000).is_empty(), "30 秒没再听说就不算");

        // 这一段没带回正在输入的房间照旧；带回空名单就是都停了。
        typing.record(agent, &batch(&[("!a:matrix.test", None)]), 3_000);
        assert_eq!(typing.now(agent, 3_000).len(), 1);
        typing.record(agent, &batch(&[("!a:matrix.test", Some(&[]))]), 4_000);
        assert!(typing.now(agent, 4_000).is_empty());

        typing.record(
            agent,
            &batch(&[("!a:matrix.test", Some(&["@bob:matrix.test"]))]),
            5_000,
        );
        typing.forget(agent);
        assert!(typing.now(agent, 5_000).is_empty());
    }
}
