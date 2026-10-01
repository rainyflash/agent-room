//! 房间里此刻谁在打字：同步带回的“正在输入”按房间记下来，等消息时交给规则（见 `wake`）。
//! 网络网关和本机 Bridge 共用。只是数据，不碰时钟：调用方给出现在几点（毫秒，同一个时钟就行）。

use std::collections::HashMap;

use agent_room_application::ports::MatrixSyncBatch;

use crate::wake::{IpcTyping, TYPING_TTL};

/// 一个 Agent 看到的“正在输入”。
#[derive(Debug, Default)]
pub struct TypingRooms {
    rooms: HashMap<String, Seen>,
}

/// 一个房间里此刻在打字的人，和什么时候听说的。
#[derive(Debug)]
struct Seen {
    users: Vec<String>,
    at_ms: i64,
}

impl TypingRooms {
    /// 记下一次同步带回的：变了的房间整个换掉，没人打字的不留。打字的人有变化时返回 true。
    pub fn record(&mut self, batch: &MatrixSyncBatch, now_ms: i64) -> bool {
        let mut changed = false;
        for room in batch.rooms() {
            let Some(typing) = room.typing() else {
                continue;
            };
            let room_id = room.room_id().as_str();
            let users: Vec<String> = typing.iter().map(|user| user.as_str().to_owned()).collect();
            changed |= self
                .rooms
                .get(room_id)
                .map_or(!users.is_empty(), |seen| seen.users != users);
            if users.is_empty() {
                self.rooms.remove(room_id);
            } else {
                self.rooms.insert(
                    room_id.to_owned(),
                    Seen {
                        users,
                        at_ms: now_ms,
                    },
                );
            }
        }
        changed
    }

    /// 此刻还算在打字的：太久（`TYPING_TTL`）没再听说的不算，免得漏掉一次“停了”就一直等。
    /// 给了房间就只看这个房间。
    pub fn now(&self, room_id: Option<&str>, now_ms: i64) -> Vec<IpcTyping> {
        let ttl = i64::try_from(TYPING_TTL.as_millis()).unwrap_or(i64::MAX);
        let mut typing: Vec<IpcTyping> = self
            .rooms
            .iter()
            .filter(|(id, seen)| {
                room_id.is_none_or(|room_id| room_id == id.as_str())
                    && now_ms.saturating_sub(seen.at_ms) < ttl
            })
            .map(|(id, seen)| IpcTyping {
                room_id: id.clone(),
                user_ids: seen.users.clone(),
            })
            .collect();
        typing.sort_by(|left, right| left.room_id.cmp(&right.room_id));
        typing
    }

    /// 没有哪个房间有人在打字。
    pub fn is_empty(&self) -> bool {
        self.rooms.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use agent_room_application::ports::{
        MatrixRoomId, MatrixRoomSync, MatrixRoomSyncKind, MatrixSyncBatch, MatrixSyncToken,
        MatrixUserId,
    };

    use super::TypingRooms;
    use crate::wake::IpcTyping;

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

    fn typing(room_id: &str, users: &[&str]) -> IpcTyping {
        IpcTyping {
            room_id: room_id.to_owned(),
            user_ids: users.iter().map(|&user| user.to_owned()).collect(),
        }
    }

    #[test]
    fn 记下每个房间谁在打字_停了或太久没消息就不算() {
        let mut rooms = TypingRooms::default();
        assert!(rooms.record(
            &batch(&[
                ("!a:matrix.test", Some(&["@ada:matrix.test"])),
                ("!b:matrix.test", None)
            ]),
            1_000,
        ));
        assert_eq!(
            rooms.now(None, 2_000),
            [typing("!a:matrix.test", &["@ada:matrix.test"])]
        );
        assert!(rooms.now(Some("!b:matrix.test"), 2_000).is_empty());
        assert!(rooms.now(None, 31_000).is_empty(), "30 秒没再听说就不算");

        // 这一段没带回正在输入的房间照旧；名单没变不算变化，但重新算时间。
        assert!(!rooms.record(&batch(&[("!a:matrix.test", None)]), 3_000));
        assert!(!rooms.record(
            &batch(&[("!a:matrix.test", Some(&["@ada:matrix.test"]))]),
            20_000
        ));
        assert_eq!(rooms.now(Some("!a:matrix.test"), 31_000).len(), 1);
        // 带回空名单就是都停了。
        assert!(rooms.record(&batch(&[("!a:matrix.test", Some(&[]))]), 32_000));
        assert!(rooms.is_empty());
        assert!(!rooms.record(&batch(&[("!a:matrix.test", Some(&[]))]), 33_000));
    }
}
