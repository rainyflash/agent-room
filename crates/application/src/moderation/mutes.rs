//! 结束一条禁言之前，先看这个人此刻该不该禁着：他在这个房间的全部禁言、房间此刻的活跃分片、他的
//! Matrix 账号，私人房间里再看他此刻有没有发言权（specs/moderation/timed-mute-expiry.md）。

use agent_room_domain::{
    moderation::{ModerationAction, ModerationActionStatus, ModerationRole},
    time::UtcMillis,
};

use crate::ports::{MatrixResult, MatrixRoomId, ModerationMuteStanding, ModerationRoomContext};

use super::{
    ModerationResult,
    service::{ModerationService, repository_failure},
};

/// 正在落的禁言最多当它在落这么久。正常落一条禁言是每个分片读一次、写一次权限，几秒钟；超过这个时间
/// 还是 `pending` 的，是没落完控制面就断了的，当它没有。
const PENDING_GRACE_MILLIS: i64 = 5 * 60 * 1000;

/// 撇开要结束的这一条，这个人此刻该怎样。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MuteCourse {
    /// 有一条禁言正在落，定不下来。
    Unsettled,
    /// 房间没了（没有活跃分片）、人没了（没有 Matrix 账号），或者私人房间里他此刻本来就不能说话：
    /// 不动 Matrix。
    LeaveAlone,
    /// 还有别的生效的禁言：在每个分片上把它再落一次，确保他禁着。之前可能被误解的（比如它落下的同一刻
    /// 有人在撤另一条）也就改回来了；已经禁着的分片什么也不写。
    KeepMuted(ModerationAction),
    /// 在每个分片上撤掉禁言。
    Lift,
}

impl MuteCourse {
    /// 按此刻的情况，结束 `ending` 这一条禁言时该怎么做。
    pub(super) fn of(
        standing: &ModerationMuteStanding,
        ending: &ModerationAction,
        now: UtcMillis,
    ) -> Self {
        let mut others = standing
            .mutes
            .iter()
            .filter(|mute| mute.id() != ending.id());
        if others.clone().any(|mute| landing_at(mute, now)) {
            return Self::Unsettled;
        }
        if standing.matrix_room_ids.is_empty() || standing.target_matrix_user_id.is_none() {
            return Self::LeaveAlone;
        }
        if let Some(effective) = others.find(|mute| mute.is_effective_at(now)) {
            return Self::KeepMuted(effective.clone());
        }
        if standing.may_speak {
            Self::Lift
        } else {
            Self::LeaveAlone
        }
    }

    /// 两次的结论一样不一样。确保禁着时落的是哪一条不要紧，Matrix 上动的都一样。
    fn same_as(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// 正在落：`pending`、开始不到 5 分钟。时钟不齐、开始时间比此刻还晚的也算。
fn landing_at(mute: &ModerationAction, now: UtcMillis) -> bool {
    mute.status() == ModerationActionStatus::Pending
        && now.value().saturating_sub(mute.starts_at().value()) < PENDING_GRACE_MILLIS
}

/// 动完再看一眼：结论和活跃分片（不看先后）都和动之前一样，才算定下来。
pub(super) fn settled_after(
    before: &ModerationMuteStanding,
    course: &MuteCourse,
    after: Option<&ModerationMuteStanding>,
    ending: &ModerationAction,
    now: UtcMillis,
) -> bool {
    after.is_some_and(|after| {
        MuteCourse::of(after, ending, now).same_as(course)
            && same_rooms(&before.matrix_room_ids, &after.matrix_room_ids)
    })
}

fn same_rooms(left: &[MatrixRoomId], right: &[MatrixRoomId]) -> bool {
    let mut left: Vec<&str> = left.iter().map(MatrixRoomId::as_str).collect();
    let mut right: Vec<&str> = right.iter().map(MatrixRoomId::as_str).collect();
    left.sort_unstable();
    right.sort_unstable();
    left == right
}

impl ModerationService {
    pub(super) async fn mute_standing(
        &self,
        action: &ModerationAction,
        operation: &'static str,
    ) -> ModerationResult<Option<ModerationMuteStanding>> {
        self.mutes
            .mute_standing(action.room_catalog_id(), action.target())
            .await
            .map_err(|error| repository_failure(operation, &error))
    }

    /// 照结论去动 Matrix：确保禁着就在每个分片上把那一条再落一次，撤掉就在每个分片上撤 `ending`。
    pub(super) async fn follow_mute_course(
        &self,
        course: &MuteCourse,
        ending: &ModerationAction,
        standing: &ModerationMuteStanding,
    ) -> MatrixResult<()> {
        let context = ModerationRoomContext {
            role: ModerationRole::None,
            room_kind: standing.room_kind,
            matrix_room_ids: standing.matrix_room_ids.clone(),
            target_matrix_user_id: standing.target_matrix_user_id.clone(),
        };
        match course {
            MuteCourse::KeepMuted(effective) => {
                self.apply_effects(effective, &context, &standing.matrix_room_ids)
                    .await
            }
            MuteCourse::Lift => {
                self.reverse_effects(ending, &context, &standing.matrix_room_ids)
                    .await
            }
            MuteCourse::Unsettled | MuteCourse::LeaveAlone => Ok(()),
        }
    }
}
