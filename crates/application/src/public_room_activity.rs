//! 发现页的“最近有人说话”（`specs/public-rooms/design.md`）：控制面每 5 分钟以应用服务读一次每间
//! 公开房间最新一条消息事件的时间，只取时间、不读内容，写进公开设置那一行。“一小时内有人说话”和
//! “最活跃”都按它。
//!
//! 子房间（`specs/redesign/sub-rooms.md` 第 2j-1 步）在同一轮里照样读，写进父子关系那一行：“浏览子
//! 房间”上的最近说话和 30 天没人说话自动归档按它。先读公开房间，子房间列不出来时公开房间这一轮照样
//! 记下了。
//!
//! 别的服务器发来的事件，时间是对方写的：比此刻还晚的按此刻记，免得一间房一直排在最前面。

use std::sync::Arc;

use agent_room_domain::{ids::RoomCatalogId, time::UtcMillis};

use crate::{
    persistence::RepositoryError,
    ports::{
        Clock, MatrixRoomId, PortFuture, PublicRoomActivityMatrix, PublicRoomActivityStore,
        SubRoomActivityStore,
    },
};

/// 一轮做了多少，公开房间和子房间一起数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PublicRoomActivityRound {
    /// 看了几间房。
    pub checked: usize,
    /// 记下了几间房的时间。
    pub recorded: usize,
    /// 几间房在 Matrix 上没读成，下一轮再读。
    pub unreadable: usize,
}

pub trait PublicRoomActivityUseCases: Send + Sync {
    fn refresh(&self) -> PortFuture<'_, Result<PublicRoomActivityRound, RepositoryError>>;
}

pub struct PublicRoomActivityDependencies {
    pub store: Arc<dyn PublicRoomActivityStore>,
    pub sub_rooms: Arc<dyn SubRoomActivityStore>,
    pub matrix: Arc<dyn PublicRoomActivityMatrix>,
    pub clock: Arc<dyn Clock>,
}

pub struct PublicRoomActivityService {
    store: Arc<dyn PublicRoomActivityStore>,
    sub_rooms: Arc<dyn SubRoomActivityStore>,
    matrix: Arc<dyn PublicRoomActivityMatrix>,
    clock: Arc<dyn Clock>,
}

/// 读到的时间记到哪儿。
#[derive(Debug, Clone, Copy)]
enum Target {
    PublicRoom,
    SubRoom,
}

impl PublicRoomActivityService {
    pub fn new(dependencies: PublicRoomActivityDependencies) -> Self {
        Self {
            store: dependencies.store,
            sub_rooms: dependencies.sub_rooms,
            matrix: dependencies.matrix,
            clock: dependencies.clock,
        }
    }

    async fn refresh_internal(&self) -> Result<PublicRoomActivityRound, RepositoryError> {
        let mut round = PublicRoomActivityRound::default();
        let public = self.store.listed_rooms_for_activity().await?;
        self.read(Target::PublicRoom, public, &mut round).await?;
        let sub_rooms = self.sub_rooms.sub_rooms_for_activity().await?;
        self.read(Target::SubRoom, sub_rooms, &mut round).await?;
        Ok(round)
    }

    async fn read(
        &self,
        target: Target,
        rooms: Vec<(RoomCatalogId, MatrixRoomId)>,
        round: &mut PublicRoomActivityRound,
    ) -> Result<(), RepositoryError> {
        round.checked += rooms.len();
        for (catalog_id, matrix_room) in rooms {
            match self.matrix.latest_message_at(&matrix_room).await {
                Ok(Some(at)) => {
                    self.record(target, catalog_id, at.min(self.clock.now()))
                        .await?;
                    round.recorded += 1;
                }
                Ok(None) => {}
                Err(_) => round.unreadable += 1,
            }
        }
        Ok(())
    }

    async fn record(
        &self,
        target: Target,
        catalog_id: RoomCatalogId,
        at: UtcMillis,
    ) -> Result<(), RepositoryError> {
        match target {
            Target::PublicRoom => self.store.record_last_message(catalog_id, at).await,
            Target::SubRoom => {
                self.sub_rooms
                    .record_sub_room_last_message(catalog_id, at)
                    .await
            }
        }
    }
}

impl PublicRoomActivityUseCases for PublicRoomActivityService {
    fn refresh(&self) -> PortFuture<'_, Result<PublicRoomActivityRound, RepositoryError>> {
        Box::pin(self.refresh_internal())
    }
}
