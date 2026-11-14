//! 房间里谁的身份变了（`specs/redesign/room-roles.md` 的“记录”）。
//!
//! 身份是五项权限上面的一层名字，所以这里记的是一个人在房间里的位置：成员状态、是不是房主、身上的
//! 权限。拿保存前后的两份房间一比就知道这一次动了谁，不用每个改成员的入口各记各的。

use crate::{
    ids::PrincipalId,
    private_rooms::{
        PrivateRoom, PrivateRoomMember, PrivateRoomMembershipStatus, PrivateRoomPermissions,
    },
};

/// 一个人在一间房里的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomStanding {
    pub status: PrivateRoomMembershipStatus,
    pub owner: bool,
    pub permissions: PrivateRoomPermissions,
}

impl RoomStanding {
    fn of(room: &PrivateRoom, member: &PrivateRoomMember) -> Self {
        Self {
            status: member.status(),
            owner: member.principal_id() == room.owner_principal_id(),
            permissions: member.permissions(),
        }
    }

    /// 还在房间里：受邀或者已加入。
    #[must_use]
    pub const fn active(self) -> bool {
        self.status.carries_permissions()
    }

    /// 管整间房的人：房主，或者管理员（受邀还没进来的也算）。
    #[must_use]
    pub const fn runs_room(self) -> bool {
        self.active() && (self.owner || self.permissions.manage())
    }
}

/// 一个人的位置从什么变成了什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomStandingChange {
    pub target: PrincipalId,
    /// 之前名单里没有这个人时为空。
    pub previous: Option<RoomStanding>,
    pub next: RoomStanding,
}

/// 从 `before` 到 `after` 要记下来的变动，按主体 ID 排。
///
/// - 两头都在房间里：房主换了人、权限改了才记。受邀变成已加入不算。
/// - 进来、出去：只记房主和管理员的（请一位管理员进来、管理员退出或者被移出、封禁）。成员和只能看
///   的人来来去去不记，那是名单的事。
/// - 两头都不在房间里（移出改成封禁、解封）：不记。
#[must_use]
pub fn standing_changes(before: &PrivateRoom, after: &PrivateRoom) -> Vec<RoomStandingChange> {
    after
        .members()
        .filter_map(|member| {
            let target = member.principal_id();
            let previous = before
                .member(target)
                .map(|member| RoomStanding::of(before, member));
            let next = RoomStanding::of(after, member);
            worth_recording(previous, next).then_some(RoomStandingChange {
                target,
                previous,
                next,
            })
        })
        .collect()
}

fn worth_recording(previous: Option<RoomStanding>, next: RoomStanding) -> bool {
    match (previous.filter(|standing| standing.active()), next.active()) {
        (Some(previous), true) => {
            previous.owner != next.owner || previous.permissions != next.permissions
        }
        (Some(previous), false) => previous.runs_room(),
        (None, true) => next.runs_room(),
        (None, false) => false,
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{RoomStanding, RoomStandingChange, standing_changes};
    use crate::{
        ids::{PrincipalId, RoomCatalogId},
        private_rooms::{
            PrivateRoom, PrivateRoomCapability, PrivateRoomMembershipStatus, PrivateRoomPermissions,
        },
    };

    #[test]
    fn 设撤管理员_改成只能看_拨开关都记_受邀变成已加入不记() {
        let mut room = room();
        invite(&mut room, MEMBER, member());
        let invited = room.clone();
        room.accept_invitation(principal(MEMBER)).expect("能接受");
        assert_eq!(standing_changes(&invited, &room), [], "进来不算身份变了");

        let before = room.clone();
        room.update_permissions(principal(OWNER), principal(MEMBER), admin())
            .expect("房主能设管理员");
        assert_eq!(
            standing_changes(&before, &room),
            [change(MEMBER, Some(joined(member())), joined(admin()),)]
        );

        let before = room.clone();
        room.update_permissions(principal(OWNER), principal(MEMBER), viewer())
            .expect("房主能改成只能看");
        assert_eq!(
            standing_changes(&before, &room),
            [change(MEMBER, Some(joined(admin())), joined(viewer()))]
        );

        let before = room.clone();
        let inviting = permissions(&[
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
            PrivateRoomCapability::Invite,
        ]);
        room.update_permissions(principal(OWNER), principal(MEMBER), inviting)
            .expect("房主能改权限");
        assert_eq!(
            standing_changes(&before, &room),
            [change(MEMBER, Some(joined(viewer())), joined(inviting))]
        );
        assert_eq!(standing_changes(&room, &room), [], "没改就没有");
    }

    #[test]
    fn 转让房主记两个人_原房主和接手的人() {
        let mut room = room();
        invite(&mut room, MEMBER, member());
        room.accept_invitation(principal(MEMBER)).expect("能接受");
        let before = room.clone();
        room.transfer_ownership(principal(OWNER), principal(MEMBER), member())
            .expect("房主能转让");

        let all = PrivateRoomPermissions::ALL;
        assert_eq!(
            standing_changes(&before, &room),
            [
                RoomStandingChange {
                    target: principal(OWNER),
                    previous: Some(RoomStanding {
                        owner: true,
                        ..joined(all)
                    }),
                    next: joined(member()),
                },
                RoomStandingChange {
                    target: principal(MEMBER),
                    previous: Some(joined(member())),
                    next: RoomStanding {
                        owner: true,
                        ..joined(all)
                    },
                },
            ]
        );
    }

    #[test]
    fn 进来出去只记管理员的_成员来去不记() {
        let mut room = room();
        let before = room.clone();
        invite(&mut room, MEMBER, member());
        assert_eq!(standing_changes(&before, &room), [], "请成员进来不记");

        let before = room.clone();
        invite(&mut room, ADMIN, admin());
        let invited_admin = RoomStanding {
            status: PrivateRoomMembershipStatus::Invited,
            owner: false,
            permissions: admin(),
        };
        assert_eq!(
            standing_changes(&before, &room),
            [change(ADMIN, None, invited_admin)],
            "请管理员进来要记"
        );

        let before = room.clone();
        room.remove_member(principal(OWNER), principal(MEMBER))
            .expect("房主能移出成员");
        assert_eq!(standing_changes(&before, &room), [], "移出成员不记");

        let before = room.clone();
        room.ban_member(principal(OWNER), principal(ADMIN))
            .expect("房主能封禁管理员");
        assert_eq!(
            standing_changes(&before, &room),
            [change(
                ADMIN,
                Some(invited_admin),
                RoomStanding {
                    status: PrivateRoomMembershipStatus::Banned,
                    owner: false,
                    permissions: PrivateRoomPermissions::NONE,
                },
            )],
            "管理员被封禁要记"
        );

        let before = room.clone();
        room.unban_member(principal(OWNER), principal(ADMIN))
            .expect("房主能解封");
        assert_eq!(standing_changes(&before, &room), [], "两头都不在房间里不记");

        // 被移出的人再请进来当管理员：之前那一份不在房间里，也要记。
        let before = room.clone();
        invite(&mut room, MEMBER, admin());
        assert_eq!(
            standing_changes(&before, &room),
            [change(
                MEMBER,
                Some(RoomStanding {
                    status: PrivateRoomMembershipStatus::Removed,
                    owner: false,
                    permissions: PrivateRoomPermissions::NONE,
                }),
                invited_admin,
            )]
        );
    }

    const OWNER: u128 = 1;
    const MEMBER: u128 = 2;
    const ADMIN: u128 = 3;

    fn room() -> PrivateRoom {
        PrivateRoom::create(
            RoomCatalogId::from_uuid(Uuid::from_u128(0x0198_b601_77a1_7bb8_83eb_a8fe_68c9_7e40)),
            principal(OWNER),
        )
    }

    fn invite(room: &mut PrivateRoom, target: u128, permissions: PrivateRoomPermissions) {
        room.invite(principal(OWNER), principal(target), permissions)
            .expect("房主能邀请");
    }

    fn change(
        target: u128,
        previous: Option<RoomStanding>,
        next: RoomStanding,
    ) -> RoomStandingChange {
        RoomStandingChange {
            target: principal(target),
            previous,
            next,
        }
    }

    fn joined(permissions: PrivateRoomPermissions) -> RoomStanding {
        RoomStanding {
            status: PrivateRoomMembershipStatus::Joined,
            owner: false,
            permissions,
        }
    }

    fn permissions(capabilities: &[PrivateRoomCapability]) -> PrivateRoomPermissions {
        PrivateRoomPermissions::from_capabilities(capabilities.iter().copied()).expect("权限合法")
    }

    fn member() -> PrivateRoomPermissions {
        permissions(&[
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
            PrivateRoomCapability::Automate,
        ])
    }

    fn admin() -> PrivateRoomPermissions {
        permissions(&[
            PrivateRoomCapability::View,
            PrivateRoomCapability::Speak,
            PrivateRoomCapability::Invite,
            PrivateRoomCapability::Manage,
        ])
    }

    fn viewer() -> PrivateRoomPermissions {
        permissions(&[PrivateRoomCapability::View])
    }

    fn principal(sequence: u128) -> PrincipalId {
        PrincipalId::from_uuid(Uuid::from_u128(
            0x0198_b601_77a1_7bb8_83eb_a8fe_0000_0000 + sequence,
        ))
    }
}
