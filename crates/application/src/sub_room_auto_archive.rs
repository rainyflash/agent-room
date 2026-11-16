//! 子房间 30 天没人说话自动归档（`specs/redesign/sub-rooms.md` 的“归档和恢复”，第 2j-2 步）：控制面每
//! 小时一轮，领走父房间开着自动归档、最近说话（恢复过的按恢复的时间，没说过话的按建的时间）早于 30 天
//! 的子房间，动手之前再向聊天服务器问一次最新一条的时间：确实超过了才归档，原因记 `inactive`；有人说过
//! 话的把时间记下来，不动它。免得照一个过时的时间把正在说话的子房间关掉。
//!
//! 一次领一批、租 10 分钟：开多个控制面时同一间不做两遍；聊天服务器读不了、存的时候撞上的，到期以后下
//! 一轮再领。

use std::sync::Arc;

use agent_room_domain::{
    sub_rooms::SUB_ROOM_AUTO_ARCHIVE_DAYS,
    time::{DurationMillis, UtcMillis},
};

use crate::{
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{Clock, PortFuture, PublicRoomActivityMatrix, QuietSubRoom, SubRoomActivityStore},
    private_rooms::InactiveSubRoomArchival,
};

/// 一次领几间。
const BATCH: u16 = 50;
/// 一轮最多领几批：一批比一批少的时候就停了，这只是防着出错时一直转。
const MAXIMUM_BATCHES: usize = 20;
/// 领走以后租多久。
const LEASE_MILLIS: u64 = 10 * 60 * 1000;
const DAY_MILLIS: i64 = 24 * 60 * 60 * 1000;

/// 一轮做了多少。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SubRoomAutoArchiveRound {
    /// 领走了几间。
    pub claimed: usize,
    /// 归档了几间。
    pub archived: usize,
    /// 几间问过聊天服务器发现有人说过话，记下时间、没动。
    pub still_talking: usize,
    /// 几间在聊天服务器上没读成，下一轮再来。
    pub unreadable: usize,
    /// 几间存的时候出错（撞上别人刚改过、库一时不通），下一轮再来。
    pub failed: usize,
}

pub trait SubRoomAutoArchiveUseCases: Send + Sync {
    fn archive_quiet(&self) -> PortFuture<'_, Result<SubRoomAutoArchiveRound, RepositoryError>>;
}

pub struct SubRoomAutoArchiveDependencies {
    pub store: Arc<dyn SubRoomActivityStore>,
    pub matrix: Arc<dyn PublicRoomActivityMatrix>,
    pub archival: Arc<dyn InactiveSubRoomArchival>,
    pub clock: Arc<dyn Clock>,
}

pub struct SubRoomAutoArchiveService {
    store: Arc<dyn SubRoomActivityStore>,
    matrix: Arc<dyn PublicRoomActivityMatrix>,
    archival: Arc<dyn InactiveSubRoomArchival>,
    clock: Arc<dyn Clock>,
}

impl SubRoomAutoArchiveService {
    pub fn new(dependencies: SubRoomAutoArchiveDependencies) -> Self {
        Self {
            store: dependencies.store,
            matrix: dependencies.matrix,
            archival: dependencies.archival,
            clock: dependencies.clock,
        }
    }

    async fn archive_quiet_internal(&self) -> Result<SubRoomAutoArchiveRound, RepositoryError> {
        const OPERATION: &str = "sub_room.auto_archive.round";
        let now = self.clock.now();
        let quiet_before =
            UtcMillis::new(now.value() - i64::from(SUB_ROOM_AUTO_ARCHIVE_DAYS) * DAY_MILLIS)
                .map_err(|_| invalid_time(OPERATION))?;
        let lease_until = DurationMillis::new(LEASE_MILLIS)
            .and_then(|lease| now.checked_add(lease))
            .map_err(|_| invalid_time(OPERATION))?;
        let mut round = SubRoomAutoArchiveRound::default();
        for _ in 0..MAXIMUM_BATCHES {
            let batch = self
                .store
                .claim_quiet_sub_rooms(quiet_before, now, lease_until, BATCH)
                .await?;
            round.claimed += batch.len();
            let last = batch.len() < usize::from(BATCH);
            for room in batch {
                self.settle(&room, quiet_before, now, &mut round).await;
            }
            if last {
                break;
            }
        }
        Ok(round)
    }

    /// 问过聊天服务器再定：有人说过话的记下时间，确实安静的归档。
    async fn settle(
        &self,
        room: &QuietSubRoom,
        quiet_before: UtcMillis,
        now: UtcMillis,
        round: &mut SubRoomAutoArchiveRound,
    ) {
        match self.matrix.latest_message_at(&room.matrix_room).await {
            Err(_) => round.unreadable += 1,
            Ok(Some(at)) if at.min(now) >= quiet_before => {
                match self
                    .store
                    .record_sub_room_last_message(room.catalog_id, at.min(now))
                    .await
                {
                    Ok(()) => round.still_talking += 1,
                    Err(_) => round.failed += 1,
                }
            }
            Ok(_) => match self
                .archival
                .archive_inactive_sub_room(room.parent_catalog_id, room.catalog_id)
                .await
            {
                Ok(true) => round.archived += 1,
                Ok(false) => {}
                Err(_) => round.failed += 1,
            },
        }
    }
}

impl SubRoomAutoArchiveUseCases for SubRoomAutoArchiveService {
    fn archive_quiet(&self) -> PortFuture<'_, Result<SubRoomAutoArchiveRound, RepositoryError>> {
        Box::pin(self.archive_quiet_internal())
    }
}

/// 此刻往前推 30 天、往后推 10 分钟都出了时间的范围：只有钟坏了才会这样。
fn invalid_time(operation: &'static str) -> RepositoryError {
    RepositoryError::new(operation, RepositoryErrorKind::Unavailable)
}
