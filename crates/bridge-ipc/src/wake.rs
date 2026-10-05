//! 等消息的规则：什么消息叫醒 Agent、叫醒以后多久交、交哪些。
//!
//! 网络网关、Bridge 和后台回复共用这一份，三种接入的行为才会一样，规则见
//! `specs/agent-reading/waiting.md`。这里只做判断，不碰时钟和 I/O：调用方给出每条消息
//! 什么时候到的、现在几点、最晚等到几点，再按结果交消息或者接着等。

use std::{
    collections::{BTreeSet, HashSet},
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::{IpcActorSummary, IpcMessagePreviewSummary};

/// 防抖默认等对话停这么久。
pub const DEFAULT_SETTLE: Duration = Duration::from_secs(5);
/// 防抖最长：从第一条叫醒它的消息算起最多多等这么久，也是防抖能设的上限。
pub const MAX_SETTLE: Duration = Duration::from_secs(30);
/// 定时看一眼最短、最长隔多久。
pub const MIN_DIGEST: Duration = Duration::from_mins(1);
pub const MAX_DIGEST: Duration = Duration::from_hours(24);
/// 本机不给等多久时，等齐最多等这么久，免得永远等一个不回话的人。
pub const DEFAULT_WAIT_FOR_LIMIT: Duration = Duration::from_mins(10);
/// `from`、`waitFor` 各最多几个人，和发消息时最多点名几个人一样；ID 加起来的字节数也一样。
pub const MAX_PEOPLE: usize = crate::limits::MENTIONS;
pub const MAX_PEOPLE_BYTES: usize = crate::limits::MENTION_BYTES;
/// “正在输入”最多算这么久：网页端每次说自己在打字时要的就是 30 秒，过了还没有新的就当停了，
/// 免得漏掉一次“停了”就一直等。
pub const TYPING_TTL: Duration = Duration::from_secs(30);

/// 什么消息算“有事”。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeRule {
    /// 跟我有关：人说的都算，点了别人或回复别人、又没点我的除外；Agent 说的要点我或回复我；
    /// 只有我和对方两个成员的房间，对方说的都算。
    #[default]
    Related,
    /// 点了我或回复我的。
    Mentions,
    /// 别人说的都算。
    All,
}

/// 一次等消息的选项，三种接入的参数都换算成它。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitOptions {
    pub wake: WakeRule,
    /// 这几个人里有人说话就叫醒（Matrix 用户 ID）。
    pub from: Vec<String>,
    /// 这几个人都说过话才叫醒（Matrix 用户 ID）。
    pub wait_for: Vec<String>,
    /// 有人回复这条消息就叫醒（消息 ID）。
    pub reply_to: Option<String>,
    /// 只听这个房间。
    pub room_id: Option<String>,
    /// 有事以后等对话停多久再交；零就是立刻交。
    pub settle: Duration,
    /// 没叫醒它的消息，最早那条攒到这么久就交给它看一眼。
    pub digest: Option<Duration>,
    /// 只给提到我或回复我的：只有它们叫得醒，交的时候也只给它们，别的算跳过。主人说的也一样。
    pub mentions_only: bool,
}

impl Default for WaitOptions {
    fn default() -> Self {
        Self {
            wake: WakeRule::default(),
            from: Vec::new(),
            wait_for: Vec::new(),
            reply_to: None,
            room_id: None,
            settle: DEFAULT_SETTLE,
            digest: None,
            mentions_only: false,
        }
    }
}

/// 哪一项选项不对。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOptionsField {
    Settle,
    Digest,
    From,
    WaitFor,
}

impl WaitOptions {
    /// 检查选项在允许的范围里。
    ///
    /// # Errors
    ///
    /// 防抖超过 30 秒、定时看一眼不在 1 分钟到 1 天之间、`from` 或 `waitFor` 超过 200 个人、
    /// ID 加起来超过 12 KB 或有空的 ID 时，返回是哪一项。
    pub fn validate(&self) -> Result<(), WaitOptionsField> {
        if self.settle > MAX_SETTLE {
            return Err(WaitOptionsField::Settle);
        }
        if self
            .digest
            .is_some_and(|digest| !(MIN_DIGEST..=MAX_DIGEST).contains(&digest))
        {
            return Err(WaitOptionsField::Digest);
        }
        if !people_valid(&self.from) {
            return Err(WaitOptionsField::From);
        }
        if !people_valid(&self.wait_for) {
            return Err(WaitOptionsField::WaitFor);
        }
        Ok(())
    }

    /// 只看一眼（等 0 秒、或者第一次取消息时）：有什么给什么，不看叫醒规则，也不防抖。
    pub fn peek(room_id: Option<String>) -> Self {
        Self {
            wake: WakeRule::All,
            room_id,
            settle: Duration::ZERO,
            ..Self::default()
        }
    }

    /// 给了 `from`、`waitFor`、`replyTo` 时只等这些，不再看 `wake`。
    fn narrowed(&self) -> bool {
        !self.from.is_empty() || !self.wait_for.is_empty() || self.reply_to.is_some()
    }
}

/// `waitFor` 只写这个时，等上一条点到的人。
pub const WAIT_FOR_MENTIONED: &str = "mentioned";

/// 三种接入收到的等消息参数；换算成规则都用这一份。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WaitParams {
    pub wake: Option<WakeRule>,
    pub from: Vec<String>,
    pub wait_for: Vec<String>,
    pub reply_to: Option<String>,
    pub settle_seconds: Option<u64>,
    pub digest_minutes: Option<u64>,
    /// 只给提到我或回复我的；不能和别的叫醒条件、定时看一眼一起用。
    pub mentions_only: bool,
}

/// 换算好的规则。`waitFor` 只写了 `mentioned` 时 `options.wait_for` 空着，由调用方换成
/// 上一条点到的人。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WaitRules {
    pub options: WaitOptions,
    pub wait_for_mentioned: bool,
}

impl WaitParams {
    /// 换算成规则并检查范围。
    ///
    /// # Errors
    ///
    /// 哪一项不对就返回它的名字（`settle`、`digest`、`from`、`waitFor`、`replyTo`、
    /// `mentionsOnly`），调用方放进错误的 `details.field`。
    pub fn parse(self) -> Result<WaitRules, &'static str> {
        // 只要点我的时，叫醒的只能是点我的：别的叫醒条件、定时看一眼都会交出空的一批。
        if self.mentions_only
            && (self.wake.is_some_and(|wake| wake != WakeRule::Mentions)
                || !self.from.is_empty()
                || !self.wait_for.is_empty()
                || self.reply_to.is_some()
                || self.digest_minutes.is_some())
        {
            return Err("mentionsOnly");
        }
        let mentioned = self
            .wait_for
            .iter()
            .any(|person| person == WAIT_FOR_MENTIONED);
        if mentioned && self.wait_for.len() > 1 {
            return Err("waitFor");
        }
        if self
            .reply_to
            .as_deref()
            .is_some_and(|message_id| uuid::Uuid::parse_str(message_id).is_err())
        {
            return Err("replyTo");
        }
        let digest = match self.digest_minutes {
            None => None,
            Some(minutes) if minutes.checked_mul(60).is_some() => {
                Some(Duration::from_mins(minutes))
            }
            Some(_) => return Err("digest"),
        };
        let options = WaitOptions {
            wake: if self.mentions_only {
                WakeRule::Mentions
            } else {
                self.wake.unwrap_or_default()
            },
            from: self.from,
            wait_for: if mentioned { Vec::new() } else { self.wait_for },
            reply_to: self.reply_to,
            room_id: None,
            settle: self
                .settle_seconds
                .map_or(DEFAULT_SETTLE, Duration::from_secs),
            digest,
            mentions_only: self.mentions_only,
        };
        options.validate().map_err(|field| match field {
            WaitOptionsField::Settle => "settle",
            WaitOptionsField::Digest => "digest",
            WaitOptionsField::From => "from",
            WaitOptionsField::WaitFor => "waitFor",
        })?;
        Ok(WaitRules {
            options,
            wait_for_mentioned: mentioned,
        })
    }
}

fn people_valid(people: &[String]) -> bool {
    people.len() <= MAX_PEOPLE
        && people.iter().map(String::len).sum::<usize>() <= MAX_PEOPLE_BYTES
        && people.iter().all(|person| !person.is_empty())
}

/// 判断时要知道的“我是谁”，和房间里此刻的动静。
#[derive(Debug, Clone, Copy)]
pub struct WakeContext<'a> {
    /// 主人的 Matrix 用户 ID。只有本机 Agent 有主人；主人说话总能叫醒它。
    pub owner: Option<&'a str>,
    /// 只有我和另一个成员的房间。
    pub direct_rooms: &'a HashSet<String>,
    /// 谁在打字、谁刚停下。
    pub typing: &'a [Typist],
}

/// 此刻在一个房间里打字的人（Matrix 用户 ID），Bridge 等消息时交给客户端。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcTyping {
    pub room_id: String,
    pub user_ids: Vec<String>,
}

/// 判断时用的“正在输入”：叫醒它的人在打字也算对话没停。还在打就接着等，停下以后再等
/// 防抖的时间，免得他点了发送、话还没到就先交了；从有事算起同样最多多等 `MAX_SETTLE`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Typist {
    pub room_id: String,
    pub user_id: String,
    /// 此刻还在打字。
    pub typing: bool,
    /// 还在打字时是最后一次听说的时刻，停了是发现他停下的时刻（和消息到的时间同一个时钟）。
    pub at_ms: i64,
}

/// 一条还没交出去的消息，和它什么时候到的（调用方自己的时钟，Unix 毫秒）。
#[derive(Debug, Clone, Copy)]
pub struct Arrival<'a> {
    pub preview: &'a IpcMessagePreviewSummary,
    pub arrived_at_ms: i64,
}

/// 为什么叫醒。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeReason {
    /// 有叫醒它的消息。
    Messages,
    /// 等的人都说过话了。
    AllReplied,
    /// 到了定时看一眼的时候。
    Digest,
    /// 等满时间。
    Timeout,
    /// 被新的等待顶掉。
    Superseded,
}

/// 回答里的 `wake`：为什么叫醒、叫醒它的是哪几条、等齐时谁还没说话。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcWake {
    pub reason: WakeReason,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
}

impl IpcWake {
    /// 没有消息要交：等满时间，或者被新的等待顶掉。
    pub const fn empty(reason: WakeReason) -> Self {
        Self {
            reason,
            event_ids: Vec::new(),
            missing: Vec::new(),
        }
    }
}

/// 交出去的一批。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    /// 要交的消息在 `pending` 里的下标，旧的在前。
    pub picks: Vec<usize>,
    /// 交出去的最后一条之前没交的条数；确认到最后一条时，它们也算看过。
    pub skipped: usize,
    /// 交出去的最后一条之后还没交的条数，下次再给。
    pub remaining: usize,
    pub wake: IpcWake,
}

/// 判断结果：现在交，或者接着等。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitDecision {
    Deliver(Delivery),
    /// 先不交。来了新消息要重新判断；到了 `recheck_at_ms`（防抖到点、定时看一眼到点）也要。
    Wait {
        recheck_at_ms: Option<i64>,
    },
}

/// 这一条会不会叫醒它。等齐（`waitFor`）看的是一组消息，不在这里判断。
pub fn wakes(
    preview: &IpcMessagePreviewSummary,
    options: &WaitOptions,
    context: WakeContext<'_>,
) -> bool {
    if preview.from_me {
        return false;
    }
    let actor = actor_matrix_id(&preview.actor);
    if context.owner == Some(actor) && !options.mentions_only {
        return true;
    }
    if options.narrowed() {
        let replies_to_target = options
            .reply_to
            .as_deref()
            .is_some_and(|target| preview.reply_to_message_id.as_deref() == Some(target));
        return replies_to_target || options.from.iter().any(|person| person == actor);
    }
    match options.wake {
        WakeRule::All => true,
        WakeRule::Mentions => preview.mentions_me,
        WakeRule::Related => {
            preview.mentions_me
                || context.direct_rooms.contains(&preview.room_id)
                || (matches!(preview.actor, IpcActorSummary::Human { .. })
                    && !addressed_to_others(preview))
        }
    }
}

/// 点了别人或回复别人、又没点我。被回复的那条找不到（只有 `replyToMessageId`）时不算，宁可叫醒。
fn addressed_to_others(preview: &IpcMessagePreviewSummary) -> bool {
    !preview.mentions_me
        && (preview.reply_to.is_some()
            || preview
                .conversation
                .as_ref()
                .is_some_and(|conversation| !conversation.mentions.is_empty()))
}

fn actor_matrix_id(actor: &IpcActorSummary) -> &str {
    match actor {
        IpcActorSummary::Agent { agent, .. } => &agent.matrix_user_id,
        IpcActorSummary::Human { matrix_user_id, .. } => matrix_user_id,
    }
}

/// `waitFor` 写 `mentioned` 时：我上一条点到的人。
pub fn mentioned_people(last_sent: Option<&IpcMessagePreviewSummary>) -> Vec<String> {
    last_sent
        .and_then(|preview| preview.conversation.as_ref())
        .map(|conversation| conversation.mentions.clone())
        .unwrap_or_default()
}

/// 看一眼还没交的消息，决定现在交还是接着等。
///
/// `pending` 是位置之后还没交的消息，旧的在前；`deadline_ms` 是这次等消息最晚等到几点，
/// 没有就是一直等。
pub fn decide(
    pending: &[Arrival<'_>],
    options: &WaitOptions,
    context: WakeContext<'_>,
    limit: usize,
    now_ms: i64,
    deadline_ms: Option<i64>,
) -> WaitDecision {
    decide_with(
        pending,
        options,
        &|preview| wakes(preview, options, context),
        context.typing,
        limit,
        now_ms,
        deadline_ms,
    )
}

/// 和 [`decide`] 一样，只是哪条叫醒它由调用方判断。后台回复有自己的规则：主人和私人房间里
/// 点名它的人才叫得醒，见 `agent_client::reception`。
pub fn decide_with(
    pending: &[Arrival<'_>],
    options: &WaitOptions,
    wakes: &dyn Fn(&IpcMessagePreviewSummary) -> bool,
    typing: &[Typist],
    limit: usize,
    now_ms: i64,
    deadline_ms: Option<i64>,
) -> WaitDecision {
    let limit = limit.max(1);
    let scope = Scope::new(pending, options, wakes);
    let timed_out = deadline_ms.is_some_and(|deadline| now_ms >= deadline);
    let Some(trigger) = scope.trigger(now_ms) else {
        if timed_out {
            // 等齐没等齐时先给已经到的；别的情况没叫醒它的消息留着，下次一起给。
            return WaitDecision::Deliver(scope.deliver(&scope.waited, limit, WakeReason::Timeout));
        }
        return WaitDecision::Wait {
            recheck_at_ms: scope.digest_due(),
        };
    };
    if options.settle.is_zero() || scope.heard.len() >= limit || timed_out {
        return WaitDecision::Deliver(scope.deliver(&trigger.priority, limit, trigger.reason));
    }
    // 叫醒它的人在打字也算对话没停：还在打就接着等，停下以后再等 `settle`；从有事算起同样
    // 最多多等 `MAX_SETTLE`。打字的人变了调用方会再问一次（网关的同步、Bridge 挂着等都会因此
    // 返回），所以还在打字时只需在上限时再看。
    let cap = trigger.at_ms.saturating_add(millis(MAX_SETTLE));
    let (typing_now, stopped_at) = scope.typing(&trigger.priority, typing);
    let typing = typing_now && now_ms < cap;
    let settle_at = scope.settle_at(trigger.at_ms, stopped_at);
    if now_ms >= settle_at && !typing {
        return WaitDecision::Deliver(scope.deliver(&trigger.priority, limit, trigger.reason));
    }
    let recheck_at_ms = if typing { cap } else { settle_at };
    WaitDecision::Wait {
        recheck_at_ms: Some(
            deadline_ms.map_or(recheck_at_ms, |deadline| deadline.min(recheck_at_ms)),
        ),
    }
}

struct Trigger {
    reason: WakeReason,
    /// 什么时候算有事了：防抖最多从这里再等 `MAX_SETTLE`。
    at_ms: i64,
    /// 一定要交的那几条。
    priority: Vec<usize>,
}

/// 这次等消息要看的那些：自己发的和别的房间的不算。
struct Scope<'p, 'a> {
    pending: &'p [Arrival<'a>],
    options: &'p WaitOptions,
    heard: Vec<usize>,
    waking: Vec<usize>,
    /// `waitFor` 名单上的人说的。
    waited: Vec<usize>,
    /// `waitFor` 名单上还没说话的人。
    missing: Vec<String>,
}

impl<'p, 'a> Scope<'p, 'a> {
    fn new(
        pending: &'p [Arrival<'a>],
        options: &'p WaitOptions,
        wakes: &dyn Fn(&IpcMessagePreviewSummary) -> bool,
    ) -> Self {
        let heard: Vec<usize> = (0..pending.len())
            .filter(|&index| {
                let preview = pending[index].preview;
                !preview.from_me
                    && options
                        .room_id
                        .as_ref()
                        .is_none_or(|room| *room == preview.room_id)
            })
            .collect();
        let waking = heard
            .iter()
            .copied()
            .filter(|&index| wakes(pending[index].preview))
            .collect();
        let speaker = |index: usize| actor_matrix_id(&pending[index].preview.actor);
        let waited: Vec<usize> = heard
            .iter()
            .copied()
            .filter(|&index| {
                options
                    .wait_for
                    .iter()
                    .any(|person| person == speaker(index))
            })
            .collect();
        let mut missing: Vec<String> = Vec::new();
        for person in &options.wait_for {
            if !missing.contains(person) && !waited.iter().any(|&index| speaker(index) == person) {
                missing.push(person.clone());
            }
        }
        Self {
            pending,
            options,
            heard,
            waking,
            waited,
            missing,
        }
    }

    fn arrived_at(&self, index: usize) -> i64 {
        self.pending[index].arrived_at_ms
    }

    fn trigger(&self, now_ms: i64) -> Option<Trigger> {
        if !self.options.wait_for.is_empty() && self.missing.is_empty() {
            // 名单上最后一个人第一次说话时，算等齐了。
            let at_ms = self
                .options
                .wait_for
                .iter()
                .filter_map(|person| {
                    self.waited
                        .iter()
                        .filter(|&&index| {
                            actor_matrix_id(&self.pending[index].preview.actor) == person
                        })
                        .map(|&index| self.arrived_at(index))
                        .min()
                })
                .max()
                .unwrap_or(now_ms);
            let priority: BTreeSet<usize> =
                self.waited.iter().chain(&self.waking).copied().collect();
            return Some(Trigger {
                reason: WakeReason::AllReplied,
                at_ms,
                priority: priority.into_iter().collect(),
            });
        }
        if let Some(at_ms) = self
            .waking
            .iter()
            .map(|&index| self.arrived_at(index))
            .min()
        {
            return Some(Trigger {
                reason: WakeReason::Messages,
                at_ms,
                priority: self.waking.clone(),
            });
        }
        let due = self.digest_due()?;
        (now_ms >= due).then(|| Trigger {
            reason: WakeReason::Digest,
            at_ms: due,
            priority: Vec::new(),
        })
    }

    /// 没叫醒它的消息里最早那条，攒到定时看一眼的时候。
    fn digest_due(&self) -> Option<i64> {
        let digest = self.options.digest?;
        let oldest = self
            .heard
            .iter()
            .map(|&index| self.arrived_at(index))
            .min()?;
        Some(oldest.saturating_add(millis(digest)))
    }

    /// 叫醒它的那几条的作者在同一个房间里打字的情况：此刻还有没有人在打，最晚的是几点停下的。
    fn typing(&self, priority: &[usize], typing: &[Typist]) -> (bool, Option<i64>) {
        let mut typing_now = false;
        let mut stopped_at = None;
        for typist in typing.iter().filter(|typist| {
            priority.iter().any(|&index| {
                let preview = self.pending[index].preview;
                typist.room_id == preview.room_id
                    && typist.user_id == actor_matrix_id(&preview.actor)
            })
        }) {
            if typist.typing {
                typing_now = true;
            } else {
                stopped_at = stopped_at.max(Some(typist.at_ms));
            }
        }
        (typing_now, stopped_at)
    }

    /// 防抖到点：最后一条消息（或者叫醒它的人停下打字）之后安静了 `settle`，
    /// 但从有事算起不超过 `MAX_SETTLE`。
    fn settle_at(&self, triggered_at_ms: i64, stopped_typing_at_ms: Option<i64>) -> i64 {
        let last = self
            .heard
            .iter()
            .map(|&index| self.arrived_at(index))
            .max()
            .unwrap_or(triggered_at_ms);
        let last = stopped_typing_at_ms.map_or(last, |stopped| last.max(stopped));
        let quiet = last.saturating_add(millis(self.options.settle.min(MAX_SETTLE)));
        quiet.min(triggered_at_ms.saturating_add(millis(MAX_SETTLE)))
    }

    fn deliver(&self, priority: &[usize], limit: usize, reason: WakeReason) -> Delivery {
        let picks = if reason == WakeReason::Timeout && priority.is_empty() {
            Vec::new()
        } else if self.options.mentions_only {
            // 只给叫醒它的（提到它或回复它的），旧的在前；别的算跳过。
            priority.iter().copied().take(limit).collect()
        } else {
            select(&self.heard, priority, limit)
        };
        let last_position = picks
            .last()
            .and_then(|last| self.heard.iter().position(|index| index == last));
        let (skipped, remaining) = last_position.map_or((0, self.heard.len()), |position| {
            (position + 1 - picks.len(), self.heard.len() - position - 1)
        });
        let event_ids = picks
            .iter()
            .filter(|index| priority.contains(index))
            .map(|&index| self.pending[index].preview.event_id.clone())
            .collect();
        Delivery {
            picks,
            skipped,
            remaining,
            wake: IpcWake {
                reason,
                event_ids,
                missing: self.missing.clone(),
            },
        }
    }
}

/// 叫醒它的那几条一定给，剩下的名额给最新的；交出去时旧的在前。
fn select(heard: &[usize], priority: &[usize], limit: usize) -> Vec<usize> {
    if heard.len() <= limit {
        return heard.to_vec();
    }
    let mut chosen: BTreeSet<usize> = priority.iter().copied().take(limit).collect();
    for &index in heard.iter().rev() {
        if chosen.len() >= limit {
            break;
        }
        chosen.insert(index);
    }
    chosen.into_iter().collect()
}

fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests;
