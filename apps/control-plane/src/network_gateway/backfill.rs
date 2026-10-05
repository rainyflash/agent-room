//! 同步时一个房间一次来得太多（Matrix 的 `limited`）就往回补（`specs/agent-reading/design.md`
//! 第 5 步）。
//!
//! 两次同步之间一个房间来的消息超过一次能带回的条数时，Matrix 只给最近一段和一个往回翻的令牌。
//! 这里从令牌往回翻页，直到碰到消息记录里已有的消息、翻到房间最早的历史，或者这个房间这一次凑够
//! 消息记录能留的条数。补回来的排在这次同步到的前面，一起验签、一起进收件箱：Agent 处理得慢、
//! 积压超过一次同步的条数也不会悄悄丢。
//!
//! 补不全的，更早的那部分就补不回来了：记在它后面第一条进收件箱的消息上（`too_many`，在之前
//! 最后一条和这条之间），交出这条时告诉 Agent。只补已经在跟的房间（消息记录里有它的消息）：
//! 第一次同步到的房间本来就只取最近一段。往回翻暂时读不到（限流、超时、服务不可用）时这次同步
//! 不算数，和同步失败一样；下次从同一位置再来。

use std::{collections::HashSet, num::NonZeroU16};

use agent_room_application::{
    network_agents::NetworkAgentSession,
    ports::{
        MatrixBackfillPage, MatrixBackfillRequest, MatrixBackfillToken, MatrixEventId,
        MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId,
        MatrixRoomSync, MatrixRoomSyncKind, MatrixSyncBatch, MatrixTimelineEvent,
        NetworkAgentGapReason, NetworkAgentHistoryDirection, NetworkAgentHistoryFilter,
        NetworkAgentInboxChange, NetworkAgentMessageRef, NetworkAgentSyncRequest,
        NetworkAgentTimelineGap,
    },
};

use super::{HISTORY_CAPACITY, NetworkGateway, NetworkGatewayFailure};

/// 每页往回读多少事件。
const PAGE_EVENTS: usize = 100;

/// 一次同步补完以后：补过的房间换成“补回来的 + 原来的”，补不全的房间记下来。
pub(super) struct Backfilled {
    pub(super) batch: MatrixSyncBatch,
    pub(super) losses: Vec<RoomLoss>,
}

/// 补不全的房间：更早的那部分丢了，在 `after_event_id`（之前最后一条）之后。
pub(super) struct RoomLoss {
    room_id: MatrixRoomId,
    after_event_id: Option<MatrixEventId>,
}

/// 往回翻到的事件，旧的在前；`complete` 表示接上了已有的消息或者翻到了房间最早的历史。
struct Collected {
    events: Vec<MatrixTimelineEvent>,
    complete: bool,
}

impl Collected {
    fn from_newest_first(mut newest_first: Vec<MatrixTimelineEvent>, complete: bool) -> Self {
        newest_first.reverse();
        Self {
            events: newest_first,
            complete,
        }
    }
}

impl NetworkGateway {
    /// 同步一次；不是第一次同步时，被截断的房间往回补。
    pub(super) async fn sync_and_backfill(
        &self,
        session: &NetworkAgentSession,
        request: &NetworkAgentSyncRequest,
        first: bool,
    ) -> Result<Backfilled, NetworkGatewayFailure> {
        let batch = self.sync(session, request).await?;
        if first || !batch.rooms().iter().any(MatrixRoomSync::timeline_limited) {
            return Ok(Backfilled {
                batch,
                losses: Vec::new(),
            });
        }
        let mut rooms = Vec::with_capacity(batch.rooms().len());
        let mut losses = Vec::new();
        for room in batch.rooms() {
            if !room.timeline_limited() || room.kind() != MatrixRoomSyncKind::Joined {
                rooms.push(room.clone());
                continue;
            }
            // 消息记录里没有这个房间的消息：刚进来的房间，本来就只取最近一段。
            let Some(last) = self.last_recorded(session, room.room_id()).await? else {
                rooms.push(room.clone());
                continue;
            };
            let budget = usize::try_from(HISTORY_CAPACITY)
                .unwrap_or(usize::MAX)
                .saturating_sub(room.timeline().len());
            let collected = match room.previous_batch() {
                Some(token) => self.collect(session, room.room_id(), token, budget).await?,
                None => Collected::from_newest_first(Vec::new(), false),
            };
            if !collected.complete {
                tracing::warn!(
                    network_agent.id = %session.network_agent_id,
                    room = %room.room_id().as_str(),
                    backfilled = collected.events.len(),
                    "网络 Agent 两次同步之间这个房间来得太多，往回补到上限还没接上，更早的那段告诉它补不回来"
                );
                losses.push(RoomLoss {
                    room_id: room.room_id().clone(),
                    after_event_id: Some(last),
                });
            }
            rooms.push(with_earlier(room, collected.events));
        }
        Ok(Backfilled {
            batch: MatrixSyncBatch::new(batch.next_batch().clone(), rooms),
            losses,
        })
    }

    /// 消息记录里这个房间最新的一条；没有就是还没在跟这个房间。
    async fn last_recorded(
        &self,
        session: &NetworkAgentSession,
        room_id: &MatrixRoomId,
    ) -> Result<Option<MatrixEventId>, NetworkGatewayFailure> {
        let latest = self
            .history
            .room_messages(
                session.network_agent_id,
                room_id,
                NetworkAgentHistoryDirection::Before(None),
                &NetworkAgentHistoryFilter::default(),
                1,
            )
            .await
            .map_err(|_| NetworkGatewayFailure::Unavailable)?;
        Ok(latest.into_iter().next().map(|message| message.event_id))
    }

    /// 从令牌往回翻页，直到碰到消息记录里已有的、翻到房间最早的历史，或者凑够 `budget` 条。
    async fn collect(
        &self,
        session: &NetworkAgentSession,
        room_id: &MatrixRoomId,
        from: &MatrixBackfillToken,
        budget: usize,
    ) -> Result<Collected, NetworkGatewayFailure> {
        let mut from = from.clone();
        let mut newest_first: Vec<MatrixTimelineEvent> = Vec::new();
        loop {
            let wanted = budget.saturating_sub(newest_first.len()).min(PAGE_EVENTS);
            let Some(limit) = u16::try_from(wanted).ok().and_then(NonZeroU16::new) else {
                return Ok(Collected::from_newest_first(newest_first, false));
            };
            let request = MatrixBackfillRequest::new(from.clone(), limit)
                .map_err(|_| NetworkGatewayFailure::Internal)?;
            let page = match self.backfill_page(session, room_id, &request).await {
                Ok(page) => page,
                Err(failure) if transient(failure) => {
                    return Err(NetworkGatewayFailure::Unavailable);
                }
                // 房间已经不让读（离开、被移出）或令牌失效：更早的补不了。
                Err(_) => return Ok(Collected::from_newest_first(newest_first, false)),
            };
            let known = self.known_events(session, page.events()).await?;
            for event in page.events() {
                if event.event_id().is_some_and(|id| known.contains(id)) {
                    return Ok(Collected::from_newest_first(newest_first, true));
                }
                newest_first.push(event.clone());
            }
            match page.end() {
                Some(end) if !page.events().is_empty() => from = end.clone(),
                // 翻到了房间最早的历史。
                _ => return Ok(Collected::from_newest_first(newest_first, true)),
            }
        }
    }

    /// 这一页里哪些事件消息记录里已经有了。
    async fn known_events(
        &self,
        session: &NetworkAgentSession,
        events: &[MatrixTimelineEvent],
    ) -> Result<HashSet<MatrixEventId>, NetworkGatewayFailure> {
        let refs: Vec<NetworkAgentMessageRef> = events
            .iter()
            .filter_map(MatrixTimelineEvent::event_id)
            .map(|id| NetworkAgentMessageRef::Event(id.clone()))
            .collect();
        if refs.is_empty() {
            return Ok(HashSet::new());
        }
        let found = self
            .history
            .messages_by_id(session.network_agent_id, &refs)
            .await
            .map_err(|_| NetworkGatewayFailure::Unavailable)?;
        Ok(found.into_iter().map(|message| message.event_id).collect())
    }

    /// 进过加密房间的用它的加密客户端往回读（事件要解密），别的直接用它自己的 Matrix 会话。
    async fn backfill_page(
        &self,
        session: &NetworkAgentSession,
        room_id: &MatrixRoomId,
        request: &MatrixBackfillRequest,
    ) -> MatrixResult<MatrixBackfillPage> {
        if !self.is_encrypted(session) {
            return self
                .matrix
                .backfill(&session.matrix_access_token, room_id, request)
                .await;
        }
        match &self.encrypted {
            Some(encrypted) => encrypted.backfill(session, room_id, request).await,
            None => Err(MatrixFailure::new(
                MatrixOperation::Backfill,
                MatrixFailureKind::DependencyUnavailable,
            )),
        }
    }
}

/// 补不回来的一段记在它后面第一条进收件箱的消息上（它自己发的不进收件箱）。后面只有它自己
/// 发的就没处可挂：它自己在说话，用不着提醒。
pub(super) fn mark_losses(changes: &mut [NetworkAgentInboxChange], losses: &[RoomLoss]) {
    for loss in losses {
        let first = changes.iter_mut().find_map(|change| match change {
            NetworkAgentInboxChange::Message(message)
                if message.room_id == loss.room_id && !message.from_me =>
            {
                Some(message)
            }
            _ => None,
        });
        if let Some(message) = first {
            message.gap = Some(NetworkAgentTimelineGap {
                after_event_id: loss.after_event_id.clone(),
                reason: NetworkAgentGapReason::TooMany,
            });
        }
    }
}

/// 补回来的排在这次同步到的前面。
fn with_earlier(room: &MatrixRoomSync, earlier: Vec<MatrixTimelineEvent>) -> MatrixRoomSync {
    if earlier.is_empty() {
        return room.clone();
    }
    let mut timeline = earlier;
    timeline.extend_from_slice(room.timeline());
    let rebuilt = MatrixRoomSync::new(
        room.room_id().clone(),
        room.kind(),
        room.timeline_limited(),
        room.previous_batch().cloned(),
        timeline,
        room.state().to_vec(),
    )
    .with_state_position(room.state_position());
    match room.typing() {
        Some(typing) => rebuilt.with_typing(typing.to_vec()),
        None => rebuilt,
    }
}

/// 暂时读不到：这次同步不算数，下次从同一位置再补。
const fn transient(failure: MatrixFailure) -> bool {
    matches!(
        failure.kind(),
        MatrixFailureKind::RateLimited
            | MatrixFailureKind::Timeout
            | MatrixFailureKind::DependencyUnavailable
            | MatrixFailureKind::Unauthenticated
    )
}
