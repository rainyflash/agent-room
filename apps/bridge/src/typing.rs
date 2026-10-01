//! 本机 Agent 的房间里此刻谁在打字：同步带回的“正在输入”记在这里，等消息时交给客户端；
//! 打字的人变了，挂着等的请求马上返回，客户端好重新判断（`specs/agent-reading/waiting.md`）。

use std::sync::{Mutex, PoisonError};

use agent_room_application::ports::MatrixSyncBatch;
use agent_room_bridge_ipc::{typing::TypingRooms, wake::IpcTyping};
use tokio::{sync::watch, time::Instant};

pub(crate) struct TypingWatch {
    rooms: Mutex<TypingRooms>,
    /// 打字的人每变一次加一。
    changes: watch::Sender<u64>,
    started: Instant,
}

impl TypingWatch {
    pub(crate) fn new() -> Self {
        let (changes, _receiver) = watch::channel(0);
        Self {
            rooms: Mutex::new(TypingRooms::default()),
            changes,
            started: Instant::now(),
        }
    }

    /// 记下一次同步带回的；打字的人变了就通知挂着等的请求。
    pub(crate) fn record(&self, batch: &MatrixSyncBatch) {
        let changed = self
            .rooms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record(batch, self.now_ms());
        if changed {
            self.changes
                .send_modify(|version| *version = version.wrapping_add(1));
        }
    }

    /// 这个房间里此刻还算在打字的。
    pub(crate) fn now(&self, room_id: &str) -> Vec<IpcTyping> {
        self.rooms
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .now(Some(room_id), self.now_ms())
    }

    /// 打字的人变了时收到通知。
    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    fn now_ms(&self) -> i64 {
        i64::try_from(self.started.elapsed().as_millis()).unwrap_or(i64::MAX)
    }
}
