//! 结束一条禁言之前（到期解除，或者管理员撤回），先看这个人此刻该不该禁着：他在这个房间的全部
//! 禁言、房间此刻的活跃分片、他的 Matrix 账号，私人房间里再看他此刻有没有发言权
//! （specs/moderation/timed-mute-expiry.md）。
//!
//! 父房间的禁言在子房间里一起算（specs/redesign/sub-rooms.md 的“禁言”）：落的时候连他在里面的
//! 子房间一起落；结束的时候子房间一起放开，哪一间自己还压着他就不放那一间。结束子房间自己的禁言时，
//! 父房间的禁言也算在“别的禁言”里。

use agent_room_domain::{
    moderation::{ModerationAction, ModerationActionKind, ModerationRole},
    rooms::RoomCatalogKind,
    time::UtcMillis,
};

use crate::ports::{
    MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId,
    ModerationMuteStanding, ModerationRoomContext, ModerationSubRoomStanding,
};

use super::{
    ModerationFailureKind, ModerationResult,
    service::{ModerationService, failure, repository_failure},
};

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
        if others.clone().any(|mute| mute.is_landing_at(now)) {
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

    /// 撤回禁言时动完再看一眼变了，照新的情况再动一次：正在落的那条当它会生效，确保禁着。手动撤销不能
    /// 叫管理员等下一轮。
    pub(super) fn insisting(
        standing: &ModerationMuteStanding,
        ending: &ModerationAction,
        now: UtcMillis,
    ) -> Self {
        match Self::of(standing, ending, now) {
            Self::Unsettled
                if standing.matrix_room_ids.is_empty()
                    || standing.target_matrix_user_id.is_none() =>
            {
                Self::LeaveAlone
            }
            Self::Unsettled => standing
                .mutes
                .iter()
                .find(|mute| mute.id() != ending.id() && mute.is_landing_at(now))
                .cloned()
                .map_or(Self::LeaveAlone, Self::KeepMuted),
            course => course,
        }
    }

    /// 两次的结论一样不一样。确保禁着时落的是哪一条不要紧，Matrix 上动的都一样。
    fn same_as(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// 这一间子房间自己还压着他的那条禁言：已经生效没到期的，或者正在落的。
fn own_mute(sub_room: &ModerationSubRoomStanding, now: UtcMillis) -> Option<&ModerationAction> {
    sub_room.mutes.iter().find(|mute| mute.holds_at(now))
}

/// 每间子房间该怎么动：这一间自己还压着他没有。排过序，只用来比变没变。
fn sub_room_plan(standing: &ModerationMuteStanding, now: UtcMillis) -> Vec<(&str, bool)> {
    let mut plan: Vec<(&str, bool)> = standing
        .sub_rooms
        .iter()
        .map(|sub_room| {
            (
                sub_room.matrix_room_id.as_str(),
                own_mute(sub_room, now).is_some(),
            )
        })
        .collect();
    plan.sort_unstable();
    plan
}

/// 动完再看一眼：结论、活跃分片（不看先后）、他在的子房间和每一间自己压没压着他，都和动之前一样，
/// 才算定下来。
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
            && sub_room_plan(before, now) == sub_room_plan(after, now)
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
    /// 撤回一条禁言：和到期解除一样，先拿这个人的禁言锁，看他此刻该不该禁着，动完再看一眼。拿不到锁、
    /// 有一条禁言正在落时什么也不动，回冲突，管理员过一会儿再点。
    pub(super) async fn reverse_mute(
        &self,
        action: ModerationAction,
        operation: &'static str,
    ) -> ModerationResult<ModerationAction> {
        let lock = self
            .mutes
            .try_lock_mutes(action.room_catalog_id(), action.target())
            .await
            .map_err(|error| repository_failure(operation, &error))?
            .ok_or_else(|| failure(operation, ModerationFailureKind::Conflict))?;
        let reversed = self.reverse_mute_locked(action, operation).await;
        lock.release().await;
        reversed
    }

    async fn reverse_mute_locked(
        &self,
        action: ModerationAction,
        operation: &'static str,
    ) -> ModerationResult<ModerationAction> {
        let now = self.clock.now();
        let standing = self
            .mute_standing(&action, operation)
            .await?
            .ok_or_else(|| failure(operation, ModerationFailureKind::NotFound))?;
        let course = MuteCourse::of(&standing, &action, now);
        if course == MuteCourse::Unsettled {
            return Err(failure(operation, ModerationFailureKind::Conflict));
        }
        self.follow_when_reversing(&course, &action, &standing, now, operation)
            .await?;
        // 期间有人又禁了他、那条生效的被撤了、新开了分片，或者他进出了哪间子房间：照新的情况再动
        // 一次，然后照常记成撤回。
        let after = self.mute_standing(&action, operation).await?;
        if !settled_after(&standing, &course, after.as_ref(), &action, now)
            && let Some(after) = after
        {
            let course = MuteCourse::insisting(&after, &action, now);
            self.follow_when_reversing(&course, &action, &after, now, operation)
                .await?;
        }
        self.record_reversed(action, operation).await
    }

    /// 撤回时照结论动 Matrix，没动成就记一条撤回失败。
    async fn follow_when_reversing(
        &self,
        course: &MuteCourse,
        action: &ModerationAction,
        standing: &ModerationMuteStanding,
        now: UtcMillis,
        operation: &'static str,
    ) -> ModerationResult<()> {
        if self
            .follow_mute_course(course, action, standing, now)
            .await
            .is_err()
        {
            return Err(self.reverse_failed(action, operation).await);
        }
        Ok(())
    }

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

    /// 刚预留的动作落下去。私人房间里的禁言连他此刻在里面的子房间一起落。子房间在预留（记成正在落）
    /// 以后才读：这以后才进子房间的人，进的时候看得到这条正在落的禁言，不会带着发言级别进去。
    pub(super) async fn land_effects(
        &self,
        action: &ModerationAction,
        context: &ModerationRoomContext,
        rooms: &[MatrixRoomId],
    ) -> MatrixResult<()> {
        if action.kind() != ModerationActionKind::Mute
            || context.room_kind != RoomCatalogKind::PrivateRoom
        {
            return self.apply_effects(action, context, rooms).await;
        }
        // 子房间没读成：这条禁言照没落成记，管理员再做一次。不能只落父房间就记成已经落下。
        let standing = self
            .mutes
            .mute_standing(action.room_catalog_id(), action.target())
            .await
            .map_err(|_| {
                MatrixFailure::new(
                    MatrixOperation::UpdatePowerLevels,
                    MatrixFailureKind::DependencyUnavailable,
                )
            })?;
        let mut rooms = rooms.to_vec();
        rooms.extend(
            standing
                .into_iter()
                .flat_map(|standing| standing.sub_rooms)
                .map(|sub_room| sub_room.matrix_room_id),
        );
        self.apply_effects(action, context, &rooms).await
    }

    /// 照结论去动 Matrix：确保禁着就在每个分片、他在的每间子房间上把那一条再落一次；撤掉就在每个
    /// 分片上撤 `ending`，子房间一间一间看：这一间自己还压着他的，照那一条再落一次（之前被别处误解的
    /// 也就改回来了），不压着的才放开。
    pub(super) async fn follow_mute_course(
        &self,
        course: &MuteCourse,
        ending: &ModerationAction,
        standing: &ModerationMuteStanding,
        now: UtcMillis,
    ) -> MatrixResult<()> {
        let context = ModerationRoomContext {
            role: ModerationRole::None,
            room_kind: standing.room_kind,
            matrix_room_ids: standing.matrix_room_ids.clone(),
            target_matrix_user_id: standing.target_matrix_user_id.clone(),
            target_agent_matrix_user_ids: standing.target_agent_matrix_user_ids.clone(),
        };
        match course {
            MuteCourse::KeepMuted(effective) => {
                let mut rooms = standing.matrix_room_ids.clone();
                rooms.extend(
                    standing
                        .sub_rooms
                        .iter()
                        .map(|sub_room| sub_room.matrix_room_id.clone()),
                );
                self.apply_effects(effective, &context, &rooms).await
            }
            MuteCourse::Lift => {
                self.reverse_effects(ending, &context, &standing.matrix_room_ids)
                    .await?;
                for sub_room in &standing.sub_rooms {
                    let room = std::slice::from_ref(&sub_room.matrix_room_id);
                    match own_mute(sub_room, now) {
                        Some(own) => self.apply_effects(own, &context, room).await?,
                        None => self.reverse_effects(ending, &context, room).await?,
                    }
                }
                Ok(())
            }
            MuteCourse::Unsettled | MuteCourse::LeaveAlone => Ok(()),
        }
    }
}
