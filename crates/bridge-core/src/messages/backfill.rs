//! 把同步时漏掉的一段时间线补回来。
//!
//! 一次同步每个房间只带最近一段（上限 50 条）；离线期间来的消息更多时，Matrix 标出
//! `limited` 并给一个往回翻的令牌。这里从令牌往回翻页，直到碰到已经记下的事件（缺口接上）、
//! 翻到房间最早的历史或达到上限，再按时间先后交给与同步相同的校验流程。补回的消息排在已收到
//! 的消息之后，收件箱把它们当作新到的消息交给 Agent，所以离线期间的消息不会悄悄丢掉。

use std::num::NonZeroU16;

use agent_room_application::ports::{
    MatrixBackfillPage, MatrixBackfillRequest, MatrixFailure, MatrixFailureKind, MatrixGateway,
    MatrixResult, MatrixRoomId, MatrixTimelineEvent, PortFuture,
};

use super::{MessageBackfillBatch, MessageSyncFailure, MessageSyncService, PendingTimelineGap};

/// 每页往回读多少事件。
const PAGE_EVENTS: NonZeroU16 = NonZeroU16::new(100).expect("每页事件数大于零");
/// 一段缺口最多往回翻几页；更早的部分不再补，免得一次离线很久就把整段历史都当新消息推给 Agent。
const MAX_PAGES: usize = 5;
/// 一轮最多补几段缺口，剩下的下一轮再补。
const MAX_GAPS_PER_ROUND: u16 = 16;

/// 往回读一页时间线的能力；Matrix 网关天然具备，测试只需实现这一个方法。
pub trait MessageBackfillSource: Send + Sync {
    fn backfill_page<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        request: &'a MatrixBackfillRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixBackfillPage>>;
}

impl<T: MatrixGateway + ?Sized> MessageBackfillSource for T {
    fn backfill_page<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        request: &'a MatrixBackfillRequest,
    ) -> PortFuture<'a, MatrixResult<MatrixBackfillPage>> {
        self.backfill(room_id, request)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MessageBackfillOutcome {
    /// 已补完（或已放弃）并结清的缺口。
    pub filled_gaps: usize,
    pub accepted_events: usize,
    pub isolated_events: usize,
    /// 翻到上限还没接上已有消息的缺口：更早的部分没有补。
    pub truncated_gaps: usize,
    /// 暂时读不到、留到下一轮再补的缺口。
    pub deferred_gaps: usize,
}

enum GapCollection {
    /// 连接或服务端暂时不可用：保留缺口，下一轮再补。
    Deferred,
    /// 按时间先后排好的事件；`complete` 表示已经接上已有消息或翻到了房间最早的历史。
    Collected {
        events: Vec<MatrixTimelineEvent>,
        complete: bool,
    },
}

impl MessageSyncService {
    /// 补回之前同步时记下的缺口。
    ///
    /// # Errors
    ///
    /// 投影存储或验签服务不可用时返回错误；这一轮已补的缺口保留，其余留到下一轮。
    pub async fn fill_gaps<S: MessageBackfillSource + ?Sized>(
        &self,
        source: &S,
    ) -> Result<MessageBackfillOutcome, MessageSyncFailure> {
        let mut outcome = MessageBackfillOutcome::default();
        let gaps = self
            .projections
            .pending_gaps(MAX_GAPS_PER_ROUND)
            .await
            .map_err(MessageSyncFailure::projection_store)?;
        for gap in gaps {
            let GapCollection::Collected { events, complete } =
                self.collect_gap(source, &gap).await?
            else {
                outcome.deferred_gaps += 1;
                continue;
            };
            let mut mutations = Vec::new();
            let mut issues = Vec::new();
            self.project_room_events(&gap.room_id, &events, &mut mutations, &mut issues)
                .await?;
            outcome.accepted_events += mutations.len();
            outcome.isolated_events += issues.len();
            if !complete {
                outcome.truncated_gaps += 1;
            }
            self.projections
                .apply_backfill(&MessageBackfillBatch::new(gap, mutations, issues))
                .await
                .map_err(MessageSyncFailure::projection_store)?;
            outcome.filled_gaps += 1;
        }
        Ok(outcome)
    }

    async fn collect_gap<S: MessageBackfillSource + ?Sized>(
        &self,
        source: &S,
        gap: &PendingTimelineGap,
    ) -> Result<GapCollection, MessageSyncFailure> {
        let mut from = gap.previous_batch.clone();
        // Matrix 往回翻页时每页从新到旧，最后整体倒过来。
        let mut newest_first = Vec::new();
        for _ in 0..MAX_PAGES {
            let Ok(request) = MatrixBackfillRequest::new(from.clone(), PAGE_EVENTS) else {
                return Ok(collected(newest_first, false));
            };
            let page = match source.backfill_page(&gap.room_id, &request).await {
                Ok(page) => page,
                Err(failure) if is_transient(failure) => return Ok(GapCollection::Deferred),
                // 房间已经不让读（离开、被移出）或令牌失效：放弃这一段，免得每轮都重试。
                Err(_) => return Ok(collected(newest_first, false)),
            };
            let ids = page
                .events()
                .iter()
                .filter_map(|event| event.event_id().cloned())
                .collect::<Vec<_>>();
            let known = self
                .projections
                .known_events(&gap.room_id, &ids)
                .await
                .map_err(MessageSyncFailure::projection_store)?;
            for event in page.events() {
                if event.event_id().is_some_and(|id| known.contains(id)) {
                    return Ok(collected(newest_first, true));
                }
                newest_first.push(event.clone());
            }
            match page.end() {
                Some(end) if !page.events().is_empty() => from = end.clone(),
                // 翻到了房间最早的历史。
                _ => return Ok(collected(newest_first, true)),
            }
        }
        Ok(collected(newest_first, false))
    }
}

fn collected(mut newest_first: Vec<MatrixTimelineEvent>, complete: bool) -> GapCollection {
    newest_first.reverse();
    GapCollection::Collected {
        events: newest_first,
        complete,
    }
}

const fn is_transient(failure: MatrixFailure) -> bool {
    matches!(
        failure.kind(),
        MatrixFailureKind::RateLimited
            | MatrixFailureKind::Timeout
            | MatrixFailureKind::DependencyUnavailable
            | MatrixFailureKind::Unauthenticated
    )
}
