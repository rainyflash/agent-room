//! 每小时那一轮里，替一个账户算该不该发提醒、说哪几间房（`specs/redesign/reminders.md` 的“怎么算”）。
//! 只算，不发、不写库：发信是 4R-5，记“上一封”的地方由调它的人定（演练时记在内存里）。

use std::sync::Arc;

use agent_room_domain::{ids::PrincipalId, time::UtcMillis};

use super::{
    plan::{ReminderDecision, ReminderHold, ReminderPlan, Standing, decide, tally},
    workspace::PersonalWorkspace,
};
use crate::ports::{ReminderCandidate, ReminderMatrixReader, ReminderRecipientRepository};

pub struct ReminderPlannerDependencies {
    pub recipients: Arc<dyn ReminderRecipientRepository>,
    pub matrix: Arc<dyn ReminderMatrixReader>,
}

/// 这个账户这一轮算不了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReminderPlanFailure {
    /// 库暂时读不了。
    Repository,
    /// 聊天服务器上他的个人工作区读不了。
    Workspace,
}

pub struct ReminderPlanner {
    recipients: Arc<dyn ReminderRecipientRepository>,
    matrix: Arc<dyn ReminderMatrixReader>,
}

impl ReminderPlanner {
    pub fn new(dependencies: ReminderPlannerDependencies) -> Self {
        Self {
            recipients: dependencies.recipients,
            matrix: dependencies.matrix,
        }
    }

    /// 按账户 ID 从小到大，`after` 之后的最多 `limit` 个候选。
    ///
    /// # Errors
    ///
    /// 库读不了时返回 `Repository`。
    pub async fn candidates(
        &self,
        after: Option<PrincipalId>,
        limit: u16,
    ) -> Result<Vec<ReminderCandidate>, ReminderPlanFailure> {
        self.recipients
            .reminder_candidates(after, limit)
            .await
            .map_err(|_| ReminderPlanFailure::Repository)
    }

    /// 替这个账户算一次。先看不用读聊天服务器就能定的（离上一封不满 24 小时），再读个人工作区看是不是
    /// 在暂停，最后一间一间读新消息。某一间读不了只跳过这一间。
    ///
    /// # Errors
    ///
    /// 他在的房间列不出来、个人工作区读不了时返回失败。
    pub async fn plan(
        &self,
        candidate: &ReminderCandidate,
        now: UtcMillis,
    ) -> Result<ReminderPlan, ReminderPlanFailure> {
        if Standing::too_soon(candidate.last_email_at, now) {
            return Ok(held(ReminderHold::TooSoon));
        }
        let document = self
            .matrix
            .personal_workspace(&candidate.matrix_user_id)
            .await
            .map_err(|_| ReminderPlanFailure::Workspace)?;
        let workspace = PersonalWorkspace::parse(document.as_ref());
        let standing = Standing {
            now,
            covered_until: candidate.covered_until,
            workspace: &workspace,
        };
        if standing.paused() {
            return Ok(held(ReminderHold::Paused));
        }
        let rooms = self
            .recipients
            .reminder_rooms(candidate.principal_id)
            .await
            .map_err(|_| ReminderPlanFailure::Repository)?;
        let mut tallies = Vec::new();
        let mut unreadable_rooms = 0_u32;
        for room in rooms
            .iter()
            .filter(|room| !workspace.muted.contains(room.matrix_room_id.as_str()))
        {
            let start = standing.room_start(room);
            let since = UtcMillis::new(start.at.max(0)).unwrap_or(now);
            match self
                .matrix
                .messages_since(
                    &candidate.matrix_user_id,
                    &room.matrix_room_id,
                    room.end_to_end_encrypted,
                    since,
                )
                .await
            {
                Ok(page) => tallies.extend(tally(room, &start, &page, &candidate.matrix_user_id)),
                Err(_) => unreadable_rooms = unreadable_rooms.saturating_add(1),
            }
        }
        Ok(ReminderPlan {
            decision: decide(now, tallies),
            unreadable_rooms,
        })
    }
}

const fn held(hold: ReminderHold) -> ReminderPlan {
    ReminderPlan {
        decision: ReminderDecision::Hold(hold),
        unreadable_rooms: 0,
    }
}

#[cfg(test)]
#[path = "planner/tests.rs"]
mod tests;
