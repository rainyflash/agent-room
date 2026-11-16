//! 每小时一轮（`specs/redesign/reminders.md` 的 4R-4c）：按账户分批算该提醒什么。
//!
//! 现在是演练：不发信、不写库。“上一封什么时候、说到哪一刻”记在内存里（重启就忘），这样一天算
//! 出来的封数接近真发时的量。发信那一步（4R-5）再把它换成库里的那一行。

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

use agent_room_domain::{ids::PrincipalId, time::UtcMillis};

use super::{ReminderDecision, ReminderHold, ReminderPlanner};
use crate::ports::{Clock, ReminderCandidate};

/// 每批这么多个账户。
pub const ROUND_BATCH: u16 = 50;

/// 一轮算下来的数。只有个数，不带账户、房间和邮箱。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReminderRoundReport {
    /// 查了几个账户。
    pub accounts: u32,
    /// 该发几封。
    pub send: u32,
    /// 该发的那几封里一共几间房。
    pub rooms: u32,
    pub paused: u32,
    pub too_soon: u32,
    pub nothing_new: u32,
    pub too_fresh: u32,
    /// 读不了、跳过的房间。
    pub unreadable_rooms: u32,
    /// 整个没算成的账户（库或者个人工作区读不了）。
    pub failed: u32,
    /// 列候选失败，这一轮没走完。
    pub interrupted: bool,
}

impl ReminderRoundReport {
    fn hold(&mut self, hold: ReminderHold) {
        let count = match hold {
            ReminderHold::Paused => &mut self.paused,
            ReminderHold::TooSoon => &mut self.too_soon,
            ReminderHold::NothingNew => &mut self.nothing_new,
            ReminderHold::TooFresh => &mut self.too_fresh,
        };
        *count = count.saturating_add(1);
    }
}

pub struct ReminderDryRun {
    planner: ReminderPlanner,
    clock: Arc<dyn Clock>,
    /// 演练里“发过”的：账户 → 那一封说到的那一刻（也就是发的时候）。
    sent: Mutex<HashMap<PrincipalId, UtcMillis>>,
}

impl ReminderDryRun {
    pub fn new(planner: ReminderPlanner, clock: Arc<dyn Clock>) -> Self {
        Self {
            planner,
            clock,
            sent: Mutex::new(HashMap::new()),
        }
    }

    /// 算一轮。列候选失败就停在那里，已经算了的照样记进报告。
    pub async fn round(&self) -> ReminderRoundReport {
        let mut report = ReminderRoundReport::default();
        let mut after = None;
        loop {
            let Ok(batch) = self.planner.candidates(after, ROUND_BATCH).await else {
                report.interrupted = true;
                break;
            };
            for candidate in &batch {
                self.plan_one(candidate, &mut report).await;
            }
            match batch.last() {
                Some(last) if batch.len() == usize::from(ROUND_BATCH) => {
                    after = Some(last.principal_id);
                }
                _ => break,
            }
        }
        report
    }

    async fn plan_one(&self, candidate: &ReminderCandidate, report: &mut ReminderRoundReport) {
        report.accounts = report.accounts.saturating_add(1);
        let candidate = self.remembered(candidate);
        // 一轮可能要几分钟，每个账户按算它的那一刻判断“一小时还没看”。
        let now = self.clock.now();
        let Ok(plan) = self.planner.plan(&candidate, now).await else {
            report.failed = report.failed.saturating_add(1);
            return;
        };
        report.unreadable_rooms = report
            .unreadable_rooms
            .saturating_add(plan.unreadable_rooms);
        match plan.decision {
            ReminderDecision::Send {
                rooms,
                covered_until,
            } => {
                report.send = report.send.saturating_add(1);
                report.rooms = report
                    .rooms
                    .saturating_add(u32::try_from(rooms.len()).unwrap_or(u32::MAX));
                self.sent
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(candidate.principal_id, covered_until);
            }
            ReminderDecision::Hold(hold) => report.hold(hold),
        }
    }

    /// 库里记着的和演练里记着的，取晚的那个。
    fn remembered(&self, candidate: &ReminderCandidate) -> ReminderCandidate {
        let sent = self
            .sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&candidate.principal_id)
            .copied();
        let later = |stored: Option<UtcMillis>| match (stored, sent) {
            (Some(stored), Some(sent)) => Some(stored.max(sent)),
            (stored, sent) => stored.or(sent),
        };
        ReminderCandidate {
            last_email_at: later(candidate.last_email_at),
            covered_until: later(candidate.covered_until),
            ..candidate.clone()
        }
    }
}

#[cfg(test)]
#[path = "round/tests.rs"]
mod tests;
