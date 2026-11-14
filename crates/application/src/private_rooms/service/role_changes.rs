//! 保存房间时顺手记下这一次动了谁的身份（`specs/redesign/room-roles.md` 的“记录”）。

use agent_room_domain::{
    ids::PrincipalId, private_rooms::PrivateRoom, room_role_changes::standing_changes,
};

use crate::ports::RoomRoleChange;

/// 拿改之前和改之后的两份房间一比。哪些变动要记由领域规则定，这里只补上是谁动的手。
pub(super) fn role_changes(
    before: &PrivateRoom,
    after: &PrivateRoom,
    actor: PrincipalId,
) -> Vec<RoomRoleChange> {
    standing_changes(before, after)
        .into_iter()
        .map(|change| RoomRoleChange {
            actor: Some(actor),
            change,
        })
        .collect()
}
