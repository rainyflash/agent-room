//! 房间里谁在打字、谁刚停下：同步带回的“正在输入”或 Bridge 交来的名单按房间记下来，
//! 等消息时交给规则（见 `wake`）。网络网关、本机 Bridge 和等消息的客户端共用。
//! 只是数据，不碰时钟：调用方给出现在几点（毫秒，和消息到的时间同一个时钟）。

use std::collections::HashMap;

use agent_room_application::ports::{MatrixSyncBatch, MatrixUserId};

use crate::wake::{IpcTyping, MAX_SETTLE, TYPING_TTL, Typist};

/// 一个 Agent 看到的“正在输入”。
#[derive(Debug, Default)]
pub struct TypingRooms {
    /// 房间 → 用户 → 情况。
    rooms: HashMap<String, HashMap<String, Seen>>,
}

#[derive(Debug, Clone, Copy)]
struct Seen {
    typing: bool,
    /// 还在打字时是最后一次听说的时刻，停了是发现他停下的时刻。
    at_ms: i64,
}

impl TypingRooms {
    /// 记下一次同步带回的：带回了“正在输入”的房间，名单上的在打字，不在名单上的停了。
    /// 有人开始或停下打字时返回 true。
    pub fn record(&mut self, batch: &MatrixSyncBatch, now_ms: i64) -> bool {
        let mut changed = false;
        for room in batch.rooms() {
            if let Some(typing) = room.typing() {
                let users: Vec<&str> = typing.iter().map(MatrixUserId::as_str).collect();
                changed |= self.set_room(room.room_id().as_str(), &users, now_ms);
            }
        }
        self.forget_old(now_ms);
        changed
    }

    /// 记下 Bridge 等消息时交来的完整名单：名单上的在打字，别的都停了。
    pub fn observe(&mut self, snapshot: &[IpcTyping], now_ms: i64) {
        let gone: Vec<String> = self
            .rooms
            .keys()
            .filter(|room_id| !snapshot.iter().any(|entry| entry.room_id == **room_id))
            .cloned()
            .collect();
        for room_id in gone {
            self.set_room(&room_id, &[], now_ms);
        }
        for entry in snapshot {
            let users: Vec<&str> = entry.user_ids.iter().map(String::as_str).collect();
            self.set_room(&entry.room_id, &users, now_ms);
        }
        self.forget_old(now_ms);
    }

    /// 此刻还在打字的（`TYPING_TTL` 内听说的），Bridge 交给客户端。给了房间就只看这个房间。
    pub fn now(&self, room_id: Option<&str>, now_ms: i64) -> Vec<IpcTyping> {
        let mut typing: Vec<IpcTyping> = self
            .rooms
            .iter()
            .filter(|(id, _)| room_id.is_none_or(|room_id| room_id == id.as_str()))
            .filter_map(|(id, users)| {
                let mut user_ids: Vec<String> = users
                    .iter()
                    .filter(|(_, seen)| typing_now(**seen, now_ms))
                    .map(|(user, _)| user.clone())
                    .collect();
                user_ids.sort();
                (!user_ids.is_empty()).then(|| IpcTyping {
                    room_id: id.clone(),
                    user_ids,
                })
            })
            .collect();
        typing.sort_by(|left, right| left.room_id.cmp(&right.room_id));
        typing
    }

    /// 规则用的：还在打字的，和最近停下的（太久没再听说的算在最后一次听说时停下）。
    pub fn typists(&self, now_ms: i64) -> Vec<Typist> {
        self.rooms
            .iter()
            .flat_map(|(room_id, users)| {
                users.iter().map(move |(user, seen)| Typist {
                    room_id: room_id.clone(),
                    user_id: user.clone(),
                    typing: typing_now(*seen, now_ms),
                    at_ms: seen.at_ms,
                })
            })
            .collect()
    }

    /// 什么也没记着。
    pub fn is_empty(&self) -> bool {
        self.rooms.is_empty()
    }

    /// 换掉一个房间的名单；有人开始或停下打字时返回 true。
    fn set_room(&mut self, room_id: &str, users: &[&str], now_ms: i64) -> bool {
        let room = self.rooms.entry(room_id.to_owned()).or_default();
        let mut changed = false;
        for (user, seen) in room.iter_mut() {
            if seen.typing && !users.contains(&user.as_str()) {
                *seen = Seen {
                    typing: false,
                    at_ms: now_ms,
                };
                changed = true;
            }
        }
        for &user in users {
            let seen = room.entry(user.to_owned()).or_insert(Seen {
                typing: false,
                at_ms: now_ms,
            });
            changed |= !seen.typing;
            *seen = Seen {
                typing: true,
                at_ms: now_ms,
            };
        }
        if room.is_empty() {
            self.rooms.remove(room_id);
        }
        changed
    }

    /// 停下（或者没再听说）超过两倍防抖上限的不再记着：那时它早就影响不到要交的消息了。
    fn forget_old(&mut self, now_ms: i64) {
        let keep = millis(MAX_SETTLE).saturating_mul(2);
        self.rooms.retain(|_, users| {
            users.retain(|_, seen| now_ms.saturating_sub(seen.at_ms) <= keep);
            !users.is_empty()
        });
    }
}

fn typing_now(seen: Seen, now_ms: i64) -> bool {
    seen.typing && now_ms.saturating_sub(seen.at_ms) < millis(TYPING_TTL)
}

fn millis(duration: std::time::Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use agent_room_application::ports::{
        MatrixRoomId, MatrixRoomSync, MatrixRoomSyncKind, MatrixSyncBatch, MatrixSyncToken,
        MatrixUserId,
    };

    use super::TypingRooms;
    use crate::wake::{IpcTyping, Typist};

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

    fn typist(user: &str, typing: bool, at_ms: i64) -> Typist {
        Typist {
            room_id: "!a:matrix.test".to_owned(),
            user_id: user.to_owned(),
            typing,
            at_ms,
        }
    }

    #[test]
    fn 同步带回的名单_在打的和停下的都记着() {
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

        // 这一段没带回正在输入的房间照旧；名单没变不算变化，但重新算时间。
        assert!(!rooms.record(&batch(&[("!a:matrix.test", None)]), 3_000));
        assert!(!rooms.record(
            &batch(&[("!a:matrix.test", Some(&["@ada:matrix.test"]))]),
            20_000
        ));
        assert_eq!(rooms.now(None, 31_000).len(), 1, "续上了就还在打");
        assert!(
            rooms.now(None, 50_000).is_empty(),
            "30 秒没再听说就不算在打"
        );

        // 带回空名单就是停了，记下什么时候停的。
        assert!(rooms.record(&batch(&[("!a:matrix.test", Some(&[]))]), 32_000));
        assert!(rooms.now(None, 32_000).is_empty());
        assert_eq!(
            rooms.typists(32_000),
            [typist("@ada:matrix.test", false, 32_000)]
        );
        assert!(!rooms.record(&batch(&[("!a:matrix.test", Some(&[]))]), 33_000));
        // 停下一分钟以后不再记着。
        rooms.record(&batch(&[]), 93_000);
        assert!(rooms.is_empty());
    }

    #[test]
    fn bridge_交来的完整名单_不在上面的就是停了() {
        let mut rooms = TypingRooms::default();
        rooms.observe(
            &[typing(
                "!a:matrix.test",
                &["@ada:matrix.test", "@bob:matrix.test"],
            )],
            1_000,
        );
        rooms.observe(&[typing("!a:matrix.test", &["@bob:matrix.test"])], 2_000);
        let mut typists = rooms.typists(2_500);
        typists.sort_by(|left, right| left.user_id.cmp(&right.user_id));
        assert_eq!(
            typists,
            [
                typist("@ada:matrix.test", false, 2_000),
                typist("@bob:matrix.test", true, 2_000)
            ]
        );
        rooms.observe(&[], 3_000);
        assert!(rooms.typists(3_000).iter().all(|typist| !typist.typing));
    }
}
