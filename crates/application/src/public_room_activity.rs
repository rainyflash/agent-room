//! 发现页的“最近有人说话”（`specs/public-rooms/design.md`）：控制面每 5 分钟以应用服务读一次每间
//! 公开房间最新一条消息事件的时间，只取时间、不读内容，写进公开设置那一行。“一小时内有人说话”和
//! “最活跃”都按它。
//!
//! 别的服务器发来的事件，时间是对方写的：比此刻还晚的按此刻记，免得一间房一直排在最前面。

use std::sync::Arc;

use crate::{
    persistence::RepositoryError,
    ports::{Clock, PortFuture, PublicRoomActivityMatrix, PublicRoomActivityStore},
};

/// 一轮做了多少。
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
    pub matrix: Arc<dyn PublicRoomActivityMatrix>,
    pub clock: Arc<dyn Clock>,
}

pub struct PublicRoomActivityService {
    store: Arc<dyn PublicRoomActivityStore>,
    matrix: Arc<dyn PublicRoomActivityMatrix>,
    clock: Arc<dyn Clock>,
}

impl PublicRoomActivityService {
    pub fn new(dependencies: PublicRoomActivityDependencies) -> Self {
        Self {
            store: dependencies.store,
            matrix: dependencies.matrix,
            clock: dependencies.clock,
        }
    }

    async fn refresh_internal(&self) -> Result<PublicRoomActivityRound, RepositoryError> {
        let rooms = self.store.listed_rooms_for_activity().await?;
        let mut round = PublicRoomActivityRound {
            checked: rooms.len(),
            ..PublicRoomActivityRound::default()
        };
        for (catalog_id, matrix_room) in rooms {
            match self.matrix.latest_message_at(&matrix_room).await {
                Ok(Some(at)) => {
                    let at = at.min(self.clock.now());
                    self.store.record_last_message(catalog_id, at).await?;
                    round.recorded += 1;
                }
                Ok(None) => {}
                Err(_) => round.unreadable += 1,
            }
        }
        Ok(round)
    }
}

impl PublicRoomActivityUseCases for PublicRoomActivityService {
    fn refresh(&self) -> PortFuture<'_, Result<PublicRoomActivityRound, RepositoryError>> {
        Box::pin(self.refresh_internal())
    }
}
