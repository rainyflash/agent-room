//! 身份变动记录（`specs/redesign/room-roles.md` 的“记录”）：改身份的那次保存一起记下是谁、把谁、
//! 从什么改成了什么。哪些变动要记是领域规则（`room_role_changes`），这里看用例把它和动手的人一起
//! 交给了存储，没存成的不记。

use agent_room_domain::room_role_changes::{RoomStanding, RoomStandingChange};

use super::*;

#[tokio::test]
async fn 设撤管理员各记一条_动手的是房主_管理员自己不当了记他自己() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    assert_eq!(recorded(&fixture), [], "请成员进来、成员进来都不记");
    let change = |actor, permissions| {
        fixture
            .service
            .update_permissions(ChangePrivateRoomPermissions {
                actor,
                catalog_id: fixture.catalog,
                target_principal_id: fixture.member,
                permissions,
            })
    };

    change(fixture.owner_actor(), admin_permissions())
        .await
        .expect("房主设管理员");
    assert_eq!(
        recorded(&fixture),
        [by(
            fixture.owner,
            fixture.member,
            joined(speaker_permissions()),
            joined(admin_permissions())
        )]
    );

    // 同样的权限再存一次：没变，不存也不记。
    change(fixture.owner_actor(), admin_permissions())
        .await
        .expect("没变也算成功");
    assert_eq!(recorded(&fixture), []);

    change(member_actor(&fixture), speaker_permissions())
        .await
        .expect("管理员自己不当了");
    assert_eq!(
        recorded(&fixture),
        [by(
            fixture.member,
            fixture.member,
            joined(admin_permissions()),
            joined(speaker_permissions())
        )]
    );
}

#[tokio::test]
async fn 转让房主记两个人_动手的是原房主() {
    let fixture = Fixture::joined(speaker_permissions()).await;

    fixture
        .service
        .transfer_ownership(TransferPrivateRoomOwnership {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
            former_owner_permissions: speaker_permissions(),
        })
        .await
        .expect("转让成功");

    let owner = |permissions| RoomStanding {
        owner: true,
        ..joined(permissions)
    };
    assert_eq!(
        recorded(&fixture),
        [
            by(
                fixture.owner,
                fixture.owner,
                owner(PrivateRoomPermissions::ALL),
                joined(speaker_permissions())
            ),
            by(
                fixture.owner,
                fixture.member,
                joined(speaker_permissions()),
                owner(PrivateRoomPermissions::ALL)
            ),
        ]
    );
}

#[tokio::test]
async fn 移出成员不记_移出管理员要记_没存成的不记() {
    let ordinary = Fixture::joined(speaker_permissions()).await;
    remove_member(&ordinary).await.expect("移出成员");
    assert_eq!(recorded(&ordinary), [], "成员来来去去是名单的事");

    let fixture = Fixture::joined(admin_permissions()).await;
    fixture.store.fail_next_save.store(true, Ordering::SeqCst);
    remove_member(&fixture).await.expect_err("这一次没存成");
    assert_eq!(recorded(&fixture), [], "房间没存成，记录也没有");

    remove_member(&fixture).await.expect("移出管理员");
    assert_eq!(
        recorded(&fixture),
        [by(
            fixture.owner,
            fixture.member,
            joined(admin_permissions()),
            RoomStanding {
                status: PrivateRoomMembershipStatus::Removed,
                owner: false,
                permissions: PrivateRoomPermissions::NONE,
            }
        )]
    );
}

async fn remove_member(fixture: &Fixture) -> Result<PrivateRoomSnapshot, PrivateRoomFailure> {
    fixture
        .service
        .remove(GovernPrivateRoomMember {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
        })
        .await
}

/// 取走到现在为止记下的。
fn recorded(fixture: &Fixture) -> Vec<RoomRoleChange> {
    std::mem::take(&mut *fixture.store.role_changes.lock().expect("记录锁正常"))
}

fn by(
    actor: PrincipalId,
    target: PrincipalId,
    previous: RoomStanding,
    next: RoomStanding,
) -> RoomRoleChange {
    RoomRoleChange {
        actor: Some(actor),
        change: RoomStandingChange {
            target,
            previous: Some(previous),
            next,
        },
    }
}

fn joined(permissions: PrivateRoomPermissions) -> RoomStanding {
    RoomStanding {
        status: PrivateRoomMembershipStatus::Joined,
        owner: false,
        permissions,
    }
}

fn member_actor(fixture: &Fixture) -> AuthenticatedPrincipal {
    actor(fixture.member, "@member:matrix.test")
}

fn admin_permissions() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([
        PrivateRoomCapability::View,
        PrivateRoomCapability::Speak,
        PrivateRoomCapability::Invite,
        PrivateRoomCapability::Manage,
    ])
    .expect("管理员权限有效")
}
