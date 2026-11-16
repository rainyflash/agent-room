//! 发不发、每间房从哪一刻算起、算几条（`specs/redesign/reminders.md` 的“什么时候发”“哪些消息该提醒”
//! “怎么算”）。这里只有规则，不读库、不读聊天服务器。

use std::cmp::Ordering;

use agent_room_domain::{ids::RoomCatalogId, time::UtcMillis};

use super::workspace::{Pause, PersonalWorkspace};
use crate::ports::{MatrixUserId, ReminderMessagePage, ReminderRoom, ReminderRoomKind};

/// 两封提醒之间至少隔这么久。
pub const MINIMUM_GAP_MS: i64 = 24 * 60 * 60 * 1_000;
/// 最早的一条新消息过了这么久还没看，才提醒：给他时间自己看到。
pub const SETTLE_MS: i64 = 60 * 60 * 1_000;
/// 最多往回看这么久：从没打开过网页的人，第一封不说一个月前的事。
pub const LOOKBACK_MS: i64 = 7 * 24 * 60 * 60 * 1_000;

/// 一个账户这一轮的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderPlan {
    pub decision: ReminderDecision,
    /// 读不了、跳过的房间有几间（比如 Matrix 上他还只是受邀）。
    pub unreadable_rooms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReminderDecision {
    /// 该发一封：按新消息多少排的房间，和这一封说到哪一刻。
    Send {
        rooms: Vec<ReminderRoomTally>,
        covered_until: UtcMillis,
    },
    Hold(ReminderHold),
}

/// 没发的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReminderHold {
    /// 在“暂停提醒”里。
    Paused,
    /// 离上一封不满 24 小时。
    TooSoon,
    /// 没有该提醒的消息。
    NothingNew,
    /// 有新消息，可最早的一条还没满 1 小时。
    TooFresh,
}

impl ReminderHold {
    /// 指标和日志里的名字。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Paused => "paused",
            Self::TooSoon => "too_soon",
            Self::NothingNew => "nothing_new",
            Self::TooFresh => "too_fresh",
        }
    }
}

/// 一间房里该提醒的消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderRoomTally {
    pub catalog_id: RoomCatalogId,
    pub kind: ReminderRoomKind,
    /// 不加密的房间（公开房间）只算点了他名字的，邮件里写“有人提到了你”。
    pub mentions_only: bool,
    pub messages: u32,
    /// 翻到上限还没翻完：实际比 `messages` 多。
    pub more: bool,
    /// 最早那一条的服务器时间。
    pub earliest_at: i64,
}

/// 一个账户此刻的情况：上一封的记录和他的个人工作区。
pub struct Standing<'a> {
    pub now: UtcMillis,
    pub covered_until: Option<UtcMillis>,
    pub workspace: &'a PersonalWorkspace,
}

impl Standing<'_> {
    /// 读消息之前就能定的：离上一封不满 24 小时。
    pub fn too_soon(last_email_at: Option<UtcMillis>, now: UtcMillis) -> bool {
        last_email_at.is_some_and(|last| now.value() - last.value() < MINIMUM_GAP_MS)
    }

    /// 读了个人工作区就能定的：还在暂停里。
    pub const fn paused(&self) -> bool {
        match self.workspace.pause {
            Pause::Off => false,
            Pause::Until(until) => until > self.now.value(),
            Pause::Indefinite => true,
        }
    }

    /// 这间房从哪一刻以后算新消息：读到的位置、上一封说到的那一刻、他进来的时候、暂停结束的
    /// 时候、7 天前，取最晚的那个。
    pub fn room_start(&self, room: &ReminderRoom) -> Start {
        let mut start = Start::through(self.now.value().saturating_sub(LOOKBACK_MS))
            .later(Start::through(room.joined_at.value()));
        if let Some(covered) = self.covered_until {
            start = start.later(Start::through(covered.value()));
        }
        if let Pause::Until(until) = self.workspace.pause {
            start = start.later(Start::through(until));
        }
        if let Some(read) = self.workspace.read.get(room.matrix_room_id.as_str()) {
            start = start.later(Start {
                at: read.at,
                after_event: Some(read.event_id.clone()),
            });
        }
        start
    }
}

/// 从哪一刻以后算：`after_event` 是读到的那一条，同一毫秒里只算 ID 排在它后面的；没有时这一毫秒
/// 整个算过了。比较照网页的 `compareCursors`：先比时间，再比事件 ID。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    pub at: i64,
    pub after_event: Option<String>,
}

impl Start {
    const fn through(at: i64) -> Self {
        Self {
            at,
            after_event: None,
        }
    }

    fn later(self, other: Self) -> Self {
        let order =
            self.at
                .cmp(&other.at)
                .then_with(|| match (&self.after_event, &other.after_event) {
                    (None, None) => Ordering::Equal,
                    (None, Some(_)) => Ordering::Greater,
                    (Some(_), None) => Ordering::Less,
                    (Some(left), Some(right)) => left.cmp(right),
                });
        if order == Ordering::Less { other } else { self }
    }

    pub fn admits(&self, sent_at: i64, event_id: &str) -> bool {
        match sent_at.cmp(&self.at) {
            Ordering::Greater => true,
            Ordering::Less => false,
            Ordering::Equal => self
                .after_event
                .as_deref()
                .is_some_and(|read| event_id > read),
        }
    }
}

/// 一间房里该提醒的有几条：别人发的、在起点以后的；不加密的房间只算点了他名字的。没有就是 `None`。
pub fn tally(
    room: &ReminderRoom,
    start: &Start,
    page: &ReminderMessagePage,
    reader: &MatrixUserId,
) -> Option<ReminderRoomTally> {
    let mentions_only = !room.end_to_end_encrypted;
    let counted = page.messages.iter().filter(|message| {
        &message.sender != reader
            && start.admits(message.sent_at.value(), &message.event_id)
            && (!mentions_only || message.mentions_reader)
    });
    let (messages, earliest_at) = counted.fold((0_u32, i64::MAX), |(count, earliest), message| {
        (
            count.saturating_add(1),
            earliest.min(message.sent_at.value()),
        )
    });
    (messages > 0).then_some(ReminderRoomTally {
        catalog_id: room.catalog_id,
        kind: room.kind,
        mentions_only,
        messages,
        more: page.truncated,
        earliest_at,
    })
}

/// 有该提醒的、最早一条也满 1 小时了才发；房间按新消息多少排。
pub fn decide(now: UtcMillis, mut rooms: Vec<ReminderRoomTally>) -> ReminderDecision {
    let Some(earliest) = rooms.iter().map(|room| room.earliest_at).min() else {
        return ReminderDecision::Hold(ReminderHold::NothingNew);
    };
    if now.value() - earliest < SETTLE_MS {
        return ReminderDecision::Hold(ReminderHold::TooFresh);
    }
    rooms.sort_by(|left, right| {
        right
            .messages
            .cmp(&left.messages)
            .then_with(|| left.catalog_id.cmp(&right.catalog_id))
    });
    ReminderDecision::Send {
        rooms,
        covered_until: now,
    }
}

#[cfg(test)]
#[path = "plan/tests.rs"]
mod tests;
