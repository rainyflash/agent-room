//! 房主删除了账户（`specs/redesign/room-roles.md` 的“房主不在了（R6）”）：房间交给当管理员最久的
//! 那位。这里只排先后；把房间交出去的那一下是 [`PrivateRoom::hand_over_from_deleted_owner`]。

use crate::{ids::PrincipalId, private_rooms::PrivateRoom, time::UtcMillis};

/// 一位管理员当了多久。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdminTenure {
    pub principal_id: PrincipalId,
    /// 身份变动记录里他最近一次拿到“管理”的时间。R3 上线以前就是管理员的没有这条记录，为空。
    pub admin_since: Option<UtcMillis>,
    /// 他的成员记录建的时间。
    pub member_since: UtcMillis,
}

/// 能接手的人，先轮到的在前：此刻已加入、有“管理”、不是房主的人里，当管理员最久的在前。没有记录的
/// 算当得最久，他们之间按成员记录的先后，再按账户号。`tenure` 里没有的人（读的当口刚设上的）排在最后，
/// 他们之间按账户号。
#[must_use]
pub fn handover_candidates(room: &PrivateRoom, tenure: &[AdminTenure]) -> Vec<PrincipalId> {
    let mut candidates: Vec<_> = room
        .members()
        .filter(|member| {
            member.principal_id() != room.owner_principal_id()
                && member.has_joined()
                && member.permissions().manage()
        })
        .map(|member| {
            let known = tenure
                .iter()
                .find(|entry| entry.principal_id == member.principal_id());
            (
                known.is_none(),
                known.and_then(|entry| entry.admin_since),
                known.map(|entry| entry.member_since),
                member.principal_id(),
            )
        })
        .collect();
    candidates.sort_unstable();
    candidates
        .into_iter()
        .map(|(_, _, _, principal)| principal)
        .collect()
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::{
        ids::RoomCatalogId,
        private_rooms::{PrivateRoomMember, PrivateRoomMembershipStatus, PrivateRoomPermissions},
    };

    fn principal(last: u8) -> PrincipalId {
        PrincipalId::from_uuid(Uuid::from_bytes([
            0x01, 0x93, 0, 0, 0, 0, 0x70, 0, 0x80, 0, 0, 0, 0, 0, 0, last,
        ]))
    }

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(1_790_000_000_000 + value).expect("时间有效")
    }

    fn admin() -> PrivateRoomPermissions {
        PrivateRoomPermissions::from_bits(0b0_1111).expect("管理员的权限")
    }

    fn member() -> PrivateRoomPermissions {
        PrivateRoomPermissions::from_bits(0b1_0011).expect("成员的权限")
    }

    /// 房主 1，管理员 2、3、4、5，成员 6，受邀的管理员 7，被移出的 8。
    fn room() -> PrivateRoom {
        let owner = principal(1);
        let mut room = PrivateRoom::create(RoomCatalogId::from_uuid(Uuid::now_v7()), owner);
        for (last, permissions) in [
            (2, admin()),
            (3, admin()),
            (4, admin()),
            (5, admin()),
            (6, member()),
            (7, admin()),
            (8, admin()),
        ] {
            room.invite(owner, principal(last), permissions)
                .expect("房主能邀请");
            if last != 7 {
                room.accept_invitation(principal(last)).expect("接受邀请");
            }
        }
        room.remove_member(owner, principal(8))
            .expect("房主能移出管理员");
        assert_eq!(
            room.member(principal(8)).map(PrivateRoomMember::status),
            Some(PrivateRoomMembershipStatus::Removed)
        );
        room
    }

    #[test]
    fn 已加入的管理员里当得最久的在前_没有记录的算最久_读不到的排最后() {
        let tenure = [
            AdminTenure {
                principal_id: principal(2),
                admin_since: Some(time(500)),
                member_since: time(10),
            },
            AdminTenure {
                principal_id: principal(3),
                admin_since: Some(time(300)),
                member_since: time(30),
            },
            AdminTenure {
                principal_id: principal(4),
                admin_since: None,
                member_since: time(40),
            },
            // 成员、受邀的、被移出的就算读到了也不算。
            AdminTenure {
                principal_id: principal(6),
                admin_since: None,
                member_since: time(1),
            },
            AdminTenure {
                principal_id: principal(7),
                admin_since: None,
                member_since: time(1),
            },
        ];
        assert_eq!(
            handover_candidates(&room(), &tenure),
            [principal(4), principal(3), principal(2), principal(5)]
        );
    }

    #[test]
    fn 都没有记录时按成员记录的先后_再按账户号() {
        let tenure = [
            AdminTenure {
                principal_id: principal(5),
                admin_since: None,
                member_since: time(10),
            },
            AdminTenure {
                principal_id: principal(3),
                admin_since: None,
                member_since: time(20),
            },
            AdminTenure {
                principal_id: principal(2),
                admin_since: None,
                member_since: time(20),
            },
        ];
        assert_eq!(
            handover_candidates(&room(), &tenure),
            [principal(5), principal(2), principal(3), principal(4)]
        );
    }
}
