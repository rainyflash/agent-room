//! 本机按规则等消息（`specs/agent-reading/waiting.md`）。Bridge 只管给消息；什么消息叫醒、
//! 防抖、等齐和定时看一眼，都在这里按 `bridge_ipc::wake` 判断。命令行 `listen` 跨轮用同一个
//! `InboxWaiter`，攒着的消息和它们到的时间不会丢。

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use agent_room_bridge_ipc::{
    IpcErrorCategory, IpcListPreviewsRequest, IpcMessagePreviewSummary, IpcMethod, IpcResponse,
    limits::INBOX_BLOCK_MILLIS,
    typing::TypingRooms,
    wake::{
        Arrival, DEFAULT_WAIT_FOR_LIMIT, Delivery, IpcTyping, IpcWake, WaitDecision, WaitOptions,
        WaitRules, WakeContext, WakeReason, decide, decide_with, mentioned_people,
    },
};
use tokio::time::Instant;

use crate::{BridgeToolClient, BridgeToolFailure, MessageWait};

/// 最多攒这么多条还没交的，和网络 Agent 的收件箱一样；再多就丢掉最早的，交的时候算进跳过的。
const HELD_CAPACITY: usize = 200;
/// 向 Bridge 一次取这么多条；一轮最多取这么多页，剩下的下一轮接着取。
const FETCH_PAGE: u16 = 50;
const FETCH_PAGES_PER_ROUND: usize = 8;
/// Bridge 立刻空手返回（不认挂着等）时，最多隔这么久再问一次，别空转。
const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// 挂着等的调用多给这么久再截断：Bridge 刚好到点才回时，不能算成超时。
const BLOCK_GRACE: Duration = Duration::from_secs(2);

/// 交给 Agent 的一批。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WokenBatch {
    /// 旧的在前；自己发的不在里面。
    pub previews: Vec<IpcMessagePreviewSummary>,
    pub wake: IpcWake,
    /// 交出去的最后一条之前没交的条数；从游标接着等时，它们也算看过。
    pub skipped: usize,
    /// 交出去的最后一条之后还攒着、没交的条数。
    pub remaining: usize,
    /// 下次从这一条之后接着等：交出去的最后一条，或者已经看过的自己发的那几条之后。
    pub cursor: Option<String>,
}

impl WokenBatch {
    /// MCP 和命令行给 Agent 看的形状：和原来的消息预览一样，多了为什么交、跳过和还攒着的条数。
    pub fn to_json(&self) -> serde_json::Value {
        let mut value = serde_json::json!({
            "type": "message_previews",
            "previews": self.previews,
            "wake": self.wake,
            "skipped": self.skipped,
            "remaining": self.remaining,
        });
        if let Some(cursor) = &self.cursor {
            value["nextCursor"] = serde_json::Value::from(cursor.as_str());
        }
        value
    }
}

/// 在一个房间里等消息。一次性的读每次新建；`listen` 一直用同一个。
pub struct InboxWaiter {
    session_id: String,
    room_id: Option<String>,
    limit: usize,
    options: WaitOptions,
    wait_for_mentioned: bool,
    /// 交到哪一条了：之前的都看过。
    delivered_through: Option<String>,
    /// 向 Bridge 取到哪一条了。
    fetched_through: Option<String>,
    /// 攒着的第一条之前是哪一条：交过的、或者攒满以后丢掉的最后一条。重读只读这之后的。
    held_after: Option<String>,
    /// 取回来还没交的，和本机看到它的时间（Unix 毫秒）。自己发的不留。
    held: Vec<(IpcMessagePreviewSummary, i64)>,
    /// 攒满以后丢掉的最早那些，交的时候算进跳过的。
    dropped: usize,
    fetched_once: bool,
    clock: WaitClock,
    /// 调用方自己的叫醒判断（后台回复）；没有就按 `wake` 规则。
    wakes: Option<WakeCheck>,
    /// 开始等时向 Bridge 问一次主人是谁：主人说话总能叫醒它。
    lookup_owner: bool,
    /// 主人的 Matrix 用户 ID；不知道时没有。
    owner: Option<String>,
    /// Bridge 等消息时交来的“正在输入”，记下谁在打、谁什么时候停下；旧版 Bridge 不给，
    /// 就当没人在打字。
    typing: TypingRooms,
    /// 没给位置时从哪里开始。
    start: Start,
}

/// 没给位置时从哪里开始读。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Start {
    /// 从最早一条（后台回复：它有服务器上的进度）。
    Earliest,
    /// 从 Bridge 记的确认位置之后（MCP 和命令行）。
    Acknowledged,
}

type WakeCheck = Box<dyn Fn(&IpcMessagePreviewSummary) -> bool + Send + Sync>;

impl InboxWaiter {
    pub fn new(
        session_id: String,
        room_id: Option<String>,
        after_event_id: Option<String>,
        limit: u16,
        rules: WaitRules,
    ) -> Self {
        Self {
            session_id,
            room_id,
            limit: usize::from(limit.max(1)),
            options: rules.options,
            wait_for_mentioned: rules.wait_for_mentioned,
            delivered_through: after_event_id.clone(),
            fetched_through: after_event_id.clone(),
            held_after: after_event_id,
            held: Vec::new(),
            dropped: 0,
            fetched_once: false,
            clock: WaitClock::start(),
            wakes: None,
            lookup_owner: false,
            owner: None,
            typing: TypingRooms::default(),
            start: Start::Earliest,
        }
    }

    /// 没给位置时从这个房间的确认位置之后开始，没确认过就从最早一条开始（MCP 和命令行用）。
    /// 后台回复用服务器上的进度，不用它。
    #[must_use]
    pub const fn from_acknowledged(mut self) -> Self {
        self.start = Start::Acknowledged;
        self
    }

    /// 开始等时向 Bridge 问一次主人是谁，主人说话总能叫醒它（MCP 和命令行用）。问不到就当没有主人。
    #[must_use]
    pub const fn with_owner_lookup(mut self) -> Self {
        self.lookup_owner = true;
        self
    }

    /// 哪条叫醒它由调用方判断，防抖、定时看一眼和交哪些照旧（后台回复用）。
    #[must_use]
    pub fn with_wakes(
        mut self,
        wakes: impl Fn(&IpcMessagePreviewSummary) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.wakes = Some(Box::new(wakes));
        self
    }

    /// 等下一批。`For(0)` 只读一次、有什么给什么；`UntilMessage` 一直等到有事（等齐时最多
    /// 10 分钟）；`For` 和 `Continuous` 等满时间就空手返回，没叫醒它的留着下次一起给。
    ///
    /// # Errors
    ///
    /// Bridge 不通、游标找不到，或者 `waitFor` 写了 `mentioned` 却找不到上一条点到的人时。
    pub async fn next(
        &mut self,
        backend: &dyn BridgeToolClient,
        wait: MessageWait,
    ) -> Result<WokenBatch, BridgeToolFailure> {
        self.resolve_mentioned(backend).await?;
        if wait == MessageWait::For(Duration::ZERO) {
            return self.peek(backend).await;
        }
        self.find_owner(backend).await;
        let started = Instant::now();
        let deadline = match wait {
            MessageWait::UntilMessage => {
                (!self.options.wait_for.is_empty()).then(|| started + DEFAULT_WAIT_FOR_LIMIT)
            }
            MessageWait::For(duration) | MessageWait::Continuous(duration) => {
                Some(started + duration)
            }
        };
        // 只有到点就得交差的调用，才能把卡住的 Bridge 请求截断；`listen` 只是结束这一轮。
        let cut_off = deadline.filter(|_| !matches!(wait, MessageWait::Continuous(_)));
        // 第一次不挂着等：先把已经到了的取回来判断。之后没有可交的，就请 Bridge 挂着等。
        let mut block = Duration::ZERO;
        loop {
            // 到点了就按攒着的交差，不再去取：带着已经过去的期限去取，真的 IPC 一次没就绪就被截断。
            let expired = deadline.is_some_and(|deadline| Instant::now() >= deadline);
            if !expired {
                let asked = Instant::now();
                let added = self.fetch(backend, true, block, cut_off).await?;
                // 不认挂着等的 Bridge 会立刻空手返回：退回每秒问一次，别空转。
                let waited = asked.elapsed();
                if !block.is_zero() && !added && waited < block {
                    tokio::time::sleep(POLL_INTERVAL.min(block.saturating_sub(waited))).await;
                }
            }
            let mut decision = self.decide(deadline);
            if matches!(decision, WaitDecision::Deliver(_)) && !expired {
                // 攒着的可能被改过或撤回了：交之前按 Bridge 现在的样子重读一遍再判断。重读很快，
                // 不跟着期限截断，免得刚好到点时把要交的弄丢。
                self.refresh(backend, None).await?;
                decision = self.decide(deadline);
            }
            match decision {
                WaitDecision::Deliver(delivery) => {
                    let batch = self.take(delivery);
                    if !matches!(wait, MessageWait::Continuous(_)) {
                        self.stop_waiting(backend).await;
                    }
                    return Ok(batch);
                }
                WaitDecision::Wait { recheck_at_ms } => {
                    // 下一次取请 Bridge 挂着等：最多 8 秒，不超过防抖或定时看一眼到点、也不超过期限。
                    let now = Instant::now();
                    let mut until_at = now + Duration::from_millis(u64::from(INBOX_BLOCK_MILLIS));
                    if let Some(at_ms) = recheck_at_ms {
                        until_at = until_at.min(now + until(self.clock.now_ms(), at_ms));
                    }
                    if let Some(deadline) = deadline {
                        until_at = until_at.min(deadline);
                    }
                    block = until_at.saturating_duration_since(now);
                }
            }
        }
    }

    /// 只看一眼：读一次，有什么给什么。
    async fn peek(
        &mut self,
        backend: &dyn BridgeToolClient,
    ) -> Result<WokenBatch, BridgeToolFailure> {
        let request = IpcListPreviewsRequest {
            after_event_id: self.delivered_through.clone(),
            room_id: self.room_id.clone(),
            before_event_id: None,
            limit: u16::try_from(self.limit).unwrap_or(FETCH_PAGE),
            keep_waiting: false,
            wait_ms: None,
            from_ack: self.start == Start::Acknowledged,
        };
        let (previews, _, _) = self
            .read_page(backend, IpcMethod::ReadInbox(request), None)
            .await?;
        let cursor = previews
            .last()
            .map(|preview| preview.event_id.clone())
            .or_else(|| self.delivered_through.clone());
        self.delivered_through.clone_from(&cursor);
        self.fetched_through.clone_from(&cursor);
        self.held_after.clone_from(&cursor);
        self.held.clear();
        let previews: Vec<_> = previews
            .into_iter()
            .filter(|preview| !preview.from_me)
            .collect();
        // 只要点我的：别的算跳过，确认到游标时也算看过。
        let heard = previews.len();
        let previews: Vec<_> = previews
            .into_iter()
            .filter(|preview| !self.options.mentions_only || preview.mentions_me)
            .collect();
        let skipped = heard - previews.len();
        let wake = if previews.is_empty() {
            IpcWake::empty(WakeReason::Timeout)
        } else {
            IpcWake::empty(WakeReason::Messages)
        };
        Ok(WokenBatch {
            previews,
            wake,
            skipped,
            remaining: 0,
            cursor,
        })
    }

    /// 从上次取到的地方接着取，直到取完或者攒满；`block` 不为零时，第一页请 Bridge 挂着等。
    /// 返回这次有没有取到新的。
    async fn fetch(
        &mut self,
        backend: &dyn BridgeToolClient,
        keep_waiting: bool,
        block: Duration,
        cut_off: Option<Instant>,
    ) -> Result<bool, BridgeToolFailure> {
        let before = self.held.len() + self.dropped;
        let first = !self.fetched_once;
        self.fetched_once = true;
        let known: HashSet<String> = self
            .held
            .iter()
            .map(|(preview, _)| preview.event_id.clone())
            .collect();
        let mut fetched_own = false;
        for page in 0..FETCH_PAGES_PER_ROUND {
            let wait_ms = (page == 0 && !block.is_zero())
                .then(|| u32::try_from(block.as_millis()).unwrap_or(INBOX_BLOCK_MILLIS))
                .map(|wait| wait.min(INBOX_BLOCK_MILLIS));
            let request = IpcListPreviewsRequest {
                after_event_id: self.fetched_through.clone(),
                room_id: self.room_id.clone(),
                before_event_id: None,
                limit: FETCH_PAGE,
                keep_waiting,
                wait_ms,
                from_ack: self.start == Start::Acknowledged,
            };
            // 挂着等的那一页多给一点再截断：Bridge 刚好到点才回时不能算成超时。
            let cut_off = cut_off.map(|cut_off| {
                if wait_ms.is_some() {
                    cut_off + BLOCK_GRACE
                } else {
                    cut_off
                }
            });
            let method = if keep_waiting {
                IpcMethod::WaitInbox(request)
            } else {
                IpcMethod::ReadInbox(request)
            };
            let (previews, more, typing) = self.read_page(backend, method, cut_off).await?;
            // 挂着等可能等了好几秒：到的时间按这一页回来的时候算。
            let now_ms = self.clock.now_ms();
            if keep_waiting {
                self.typing.observe(&typing, now_ms);
            }
            for preview in previews {
                self.fetched_through = Some(preview.event_id.clone());
                if preview.from_me || known.contains(&preview.event_id) {
                    fetched_own |= preview.from_me;
                    continue;
                }
                // 开始等之前就到了的，按它发出的时间算（不晚于现在），早就安静了的不用再防抖。
                let arrived = if first {
                    preview.created_at_unix_ms.min(now_ms)
                } else {
                    now_ms
                };
                self.held.push((preview, arrived));
            }
            if !more {
                break;
            }
        }
        // 攒满了照样取新的，丢掉最早的：不然后来叫醒它的消息进不来。丢掉的最早那条到的时间
        // 留给剩下最早的一条，定时看一眼才不会因为一直在丢而永远不到点。
        if self.held.len() > HELD_CAPACITY {
            let excess = self.held.len() - HELD_CAPACITY;
            let dropped: Vec<_> = self.held.drain(..excess).collect();
            self.held_after = dropped.last().map(|(preview, _)| preview.event_id.clone());
            let oldest_dropped = dropped.iter().map(|(_, at_ms)| *at_ms).min();
            if let (Some(dropped_at_ms), Some((_, first_at_ms))) =
                (oldest_dropped, self.held.first_mut())
            {
                *first_at_ms = (*first_at_ms).min(dropped_at_ms);
            }
            self.dropped += excess;
        }
        Ok(fetched_own || self.held.len() + self.dropped > before)
    }

    /// 从交到的地方重读攒着的那些：改过的换成新的，撤回的去掉，到的时间不变。
    async fn refresh(
        &mut self,
        backend: &dyn BridgeToolClient,
        cut_off: Option<Instant>,
    ) -> Result<(), BridgeToolFailure> {
        let arrived: HashMap<String, i64> = self
            .held
            .drain(..)
            .map(|(preview, at_ms)| (preview.event_id, at_ms))
            .collect();
        self.fetched_through.clone_from(&self.held_after);
        self.fetch(backend, true, Duration::ZERO, cut_off).await?;
        for (preview, at_ms) in &mut self.held {
            if let Some(original) = arrived.get(&preview.event_id) {
                *at_ms = *original;
            }
        }
        // 重读时又丢掉的那几条，到的时间同样留给剩下的第一条。
        if let (Some(oldest), Some((_, first_at_ms))) =
            (arrived.values().min(), self.held.first_mut())
        {
            *first_at_ms = (*first_at_ms).min(*oldest);
        }
        Ok(())
    }

    fn decide(&self, deadline: Option<Instant>) -> WaitDecision {
        let now_ms = self.clock.now_ms();
        let deadline_ms = deadline.map(|deadline| {
            now_ms.saturating_add(millis(deadline.saturating_duration_since(Instant::now())))
        });
        let arrivals: Vec<Arrival<'_>> = self
            .held
            .iter()
            .map(|(preview, arrived_at_ms)| Arrival {
                preview,
                arrived_at_ms: *arrived_at_ms,
            })
            .collect();
        let typists = self.typing.typists(now_ms);
        if let Some(wakes) = &self.wakes {
            return decide_with(
                &arrivals,
                &self.options,
                wakes.as_ref(),
                &typists,
                self.limit,
                now_ms,
                deadline_ms,
            );
        }
        let direct_rooms = HashSet::new();
        decide(
            &arrivals,
            &self.options,
            WakeContext {
                owner: self.owner.as_deref(),
                direct_rooms: &direct_rooms,
                typing: &typists,
            },
            self.limit,
            now_ms,
            deadline_ms,
        )
    }

    /// 交出选中的那几条；最后一条之前的都算看过，从攒着的里面拿掉。
    fn take(&mut self, delivery: Delivery) -> WokenBatch {
        let consumed = delivery.picks.last().map_or(0, |last| last + 1);
        let mut previews = Vec::with_capacity(delivery.picks.len());
        for (index, (preview, _)) in self.held.drain(..consumed).enumerate() {
            if delivery.picks.contains(&index) {
                previews.push(preview);
            }
        }
        if let Some(last) = previews.last() {
            self.delivered_through = Some(last.event_id.clone());
            self.held_after.clone_from(&self.delivered_through);
        } else if self.held.is_empty() {
            // 什么都没交、也没攒着别人的：看过的只有自己发的，游标直接跟上。
            self.delivered_through.clone_from(&self.fetched_through);
            self.held_after.clone_from(&self.delivered_through);
        }
        let skipped = if consumed > 0 {
            delivery.skipped + std::mem::take(&mut self.dropped)
        } else {
            delivery.skipped
        };
        WokenBatch {
            previews,
            wake: delivery.wake,
            skipped,
            remaining: delivery.remaining,
            cursor: self.delivered_through.clone(),
        }
    }

    /// `waitFor` 写了 `mentioned`：换成我上一条点到的人。只找一次。
    async fn resolve_mentioned(
        &mut self,
        backend: &dyn BridgeToolClient,
    ) -> Result<(), BridgeToolFailure> {
        if !self.wait_for_mentioned {
            return Ok(());
        }
        self.wait_for_mentioned = false;
        let request = IpcListPreviewsRequest {
            after_event_id: None,
            room_id: self.room_id.clone(),
            before_event_id: None,
            limit: FETCH_PAGE,
            keep_waiting: false,
            wait_ms: None,
            from_ack: false,
        };
        let (newest_first, _, _) = self
            .read_page(backend, IpcMethod::ListPreviews(request), None)
            .await?;
        let people = mentioned_people(newest_first.iter().find(|preview| preview.from_me));
        if people.is_empty() {
            return Err(invalid("waitFor"));
        }
        self.options.wait_for = people;
        Ok(())
    }

    /// 问一次主人是谁；问不到（旧版 Bridge、还没授权）就当没有主人，照样等。
    async fn find_owner(&mut self, backend: &dyn BridgeToolClient) {
        if !std::mem::take(&mut self.lookup_owner) {
            return;
        }
        let method = IpcMethod::WithSession {
            session_id: self.session_id.clone(),
            method: Box::new(IpcMethod::GetSelf),
        };
        if let Ok(IpcResponse::SelfSummary { summary }) = backend.invoke(method).await {
            self.owner = summary.owner.map(|owner| owner.matrix_user_id);
        }
    }

    /// 等到了要交的，告诉 Bridge 不在等了；失败了也只是“等待中”晚 10 秒消失。
    async fn stop_waiting(&self, backend: &dyn BridgeToolClient) {
        let request = IpcListPreviewsRequest {
            after_event_id: self.delivered_through.clone(),
            room_id: self.room_id.clone(),
            before_event_id: None,
            limit: 1,
            keep_waiting: false,
            wait_ms: None,
            from_ack: self.start == Start::Acknowledged,
        };
        let method = IpcMethod::WithSession {
            session_id: self.session_id.clone(),
            method: Box::new(IpcMethod::ReadInbox(request)),
        };
        let _ = backend.invoke(method).await;
    }

    async fn read_page(
        &self,
        backend: &dyn BridgeToolClient,
        method: IpcMethod,
        cut_off: Option<Instant>,
    ) -> Result<(Vec<IpcMessagePreviewSummary>, bool, Vec<IpcTyping>), BridgeToolFailure> {
        let method = IpcMethod::WithSession {
            session_id: self.session_id.clone(),
            method: Box::new(method),
        };
        method
            .validate()
            .map_err(|error| failure(error.code(), IpcErrorCategory::Validation, false))?;
        let response = match cut_off {
            Some(cut_off) => tokio::time::timeout_at(cut_off, backend.invoke(method))
                .await
                .map_err(|_| {
                    failure(
                        "agent.inbox.timeout",
                        IpcErrorCategory::DependencyUnavailable,
                        true,
                    )
                })??,
            None => backend.invoke(method).await?,
        };
        match response {
            IpcResponse::MessagePreviews {
                previews,
                next_cursor,
                typing,
            } => Ok((previews, next_cursor.is_some(), typing)),
            _ => Err(failure(
                "agent.inbox.response_invalid",
                IpcErrorCategory::Internal,
                false,
            )),
        }
    }
}

/// 墙上时间加上 tokio 单调时钟走过的时长：等的时候改系统时间也不乱，暂停时间的测试里也会走。
#[derive(Debug, Clone, Copy)]
struct WaitClock {
    started_at_ms: i64,
    started: Instant,
}

impl WaitClock {
    fn start() -> Self {
        let started_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, millis);
        Self {
            started_at_ms,
            started: Instant::now(),
        }
    }

    fn now_ms(self) -> i64 {
        self.started_at_ms
            .saturating_add(millis(self.started.elapsed()))
    }
}

fn until(now_ms: i64, at_ms: i64) -> Duration {
    Duration::from_millis(u64::try_from(at_ms.saturating_sub(now_ms)).unwrap_or(0))
}

fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

fn invalid(field: &str) -> BridgeToolFailure {
    BridgeToolFailure::new(
        "agent.inbox.wait_invalid",
        IpcErrorCategory::Validation,
        false,
        BTreeMap::from([("field".to_owned(), field.to_owned())]),
    )
}

fn failure(code: &str, category: IpcErrorCategory, retryable: bool) -> BridgeToolFailure {
    BridgeToolFailure::new(code, category, retryable, BTreeMap::new())
}

#[cfg(test)]
mod tests;
