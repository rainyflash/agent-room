//! 这次连上以后，同步里看到的自己在哪些房间（`specs/agent-identity/design.md`）。服务器替 Agent
//! 进了新房间以后，要等这台电脑上的 Matrix 客户端同步到它，说话、读消息才认得这个房间。

use std::{collections::BTreeSet, time::Duration};

use agent_room_application::ports::{MatrixRoomId, MatrixRoomSyncKind};
use tokio::sync::watch;

pub(crate) struct JoinedRooms {
    rooms: watch::Sender<BTreeSet<MatrixRoomId>>,
}

impl JoinedRooms {
    pub(crate) fn new() -> Self {
        let (rooms, _receiver) = watch::channel(BTreeSet::new());
        Self { rooms }
    }

    /// 一次同步里的房间：加入了的记下，离开了的去掉。连上以后第一次是全量同步，它在的房间都在里面。
    pub(crate) fn record<'r>(
        &self,
        rooms: impl IntoIterator<Item = (&'r MatrixRoomId, MatrixRoomSyncKind)>,
    ) {
        let changes = rooms.into_iter().collect::<Vec<_>>();
        self.rooms.send_if_modified(|joined| {
            let mut modified = false;
            for (room_id, kind) in changes {
                modified |= match kind {
                    MatrixRoomSyncKind::Joined => joined.insert(room_id.clone()),
                    MatrixRoomSyncKind::Left => joined.remove(room_id),
                    MatrixRoomSyncKind::Invited | MatrixRoomSyncKind::Knocked => false,
                };
            }
            modified
        });
    }

    /// 此刻同步里看到的房间。
    pub(crate) fn rooms(&self) -> Vec<MatrixRoomId> {
        self.rooms.borrow().iter().cloned().collect()
    }

    /// 等到同步里看到自己在这个房间，最多等 `timeout`；看到了回答 `true`。
    pub(crate) async fn wait_for(&self, room_id: &MatrixRoomId, timeout: Duration) -> bool {
        let mut receiver = self.rooms.subscribe();
        tokio::time::timeout(timeout, receiver.wait_for(|rooms| rooms.contains(room_id)))
            .await
            .is_ok_and(|seen| seen.is_ok())
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use agent_room_application::ports::{MatrixRoomId, MatrixRoomSyncKind};

    use super::JoinedRooms;

    fn room(id: &str) -> MatrixRoomId {
        MatrixRoomId::new(id).expect("房间标识有效")
    }

    #[tokio::test(start_paused = true)]
    async fn 等到同步里看到加入_离开以后再等就等不到() {
        let joined = Arc::new(JoinedRooms::new());
        let lobby = room("!lobby:matrix.test");
        let private = room("!private:matrix.test");
        joined.record([(&lobby, MatrixRoomSyncKind::Joined)]);
        assert!(joined.wait_for(&lobby, Duration::ZERO).await);

        let waiting = tokio::spawn({
            let joined = joined.clone();
            let private = private.clone();
            async move { joined.wait_for(&private, Duration::from_secs(20)).await }
        });
        tokio::task::yield_now().await;
        joined.record([(&private, MatrixRoomSyncKind::Invited)]);
        joined.record([(&private, MatrixRoomSyncKind::Joined)]);
        assert!(
            waiting.await.expect("等待任务完成"),
            "同步里看到加入就不再等"
        );

        joined.record([(&private, MatrixRoomSyncKind::Left)]);
        assert!(
            !joined.wait_for(&private, Duration::from_secs(20)).await,
            "离开以后等到超时"
        );
    }
}
