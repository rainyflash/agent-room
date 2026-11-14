//! 只有房主能设、撤管理员，管理员之间谁也动不了谁（`specs/redesign/room-roles.md` R1）。规则在领域层，
//! 这里看存进库、读回来以后照旧认得出谁是管理员：被拒的改动没写进去，房主撤了以后才动得了他。

use agent_room_domain::error::DomainError;

use super::{listing::create_room, *};

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 管理员存进库读回来_照旧只有房主动得了() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "admins-owner").await;
    let first = seed_principal(&database.runtime, "admins-first").await;
    let second = seed_principal(&database.runtime, "admins-second").await;
    let catalog = create_room(&repositories, owner).await;

    let mut clock = 0;
    for admin in [first, second] {
        change(&repositories, catalog, &mut clock, |room| {
            room.invite(owner, admin, admin_permissions())
        })
        .await;
        change(&repositories, catalog, &mut clock, |room| {
            room.accept_invitation(admin)
        })
        .await;
    }

    // 读回来的房间里两位都是管理员，彼此动不了：设不了、撤不了、移不出、封不了。
    let stored = load(&repositories, catalog).await;
    assert!(stored.is_admin(first) && stored.is_admin(second));
    assert!(!stored.is_admin(owner), "房主不算管理员");
    let version = stored.version();
    let mut attempt = stored.clone();
    for refused in [
        attempt.update_permissions(first, second, ordinary_permissions()),
        attempt.remove_member(first, second),
        attempt.ban_member(first, second),
    ] {
        assert!(
            matches!(refused, Err(DomainError::OwnerOnly { .. })),
            "{refused:?}"
        );
    }
    assert_eq!(attempt.version(), version, "被拒的改动不动版本");
    assert_eq!(load(&repositories, catalog).await, stored);

    // 房主撤掉第二位：他成了成员，第一位这才移得出他。
    change(&repositories, catalog, &mut clock, |room| {
        room.update_permissions(owner, second, ordinary_permissions())
    })
    .await;
    let demoted = load(&repositories, catalog).await;
    assert!(demoted.is_admin(first) && !demoted.is_admin(second));
    assert!(demoted.allows(second, PrivateRoomCapability::Speak));
    change(&repositories, catalog, &mut clock, |room| {
        room.remove_member(first, second)
    })
    .await;

    // 第一位自己不当了：存进去以后他只是成员，再设不了别人。
    change(&repositories, catalog, &mut clock, |room| {
        room.update_permissions(first, first, ordinary_permissions())
    })
    .await;
    let settled = load(&repositories, catalog).await;
    assert!(!settled.is_admin(first));
    assert_eq!(
        settled.member(second).expect("成员记录留着").status(),
        PrivateRoomMembershipStatus::Removed
    );
    assert_eq!(
        settled.member(first).expect("还在房间里").status(),
        PrivateRoomMembershipStatus::Joined
    );

    database.close().await;
}

/// 读出房间、做一步、按读出来的版本存回去。
async fn change(
    repositories: &PostgresRepositories,
    catalog: RoomCatalogId,
    clock: &mut i64,
    step: impl FnOnce(&mut PrivateRoom) -> Result<bool, DomainError>,
) {
    let mut room = load(repositories, catalog).await;
    let expected = room.version();
    assert!(step(&mut room).expect("这一步做得了"), "这一步真的改了房间");
    *clock += 1;
    PrivateRoomStore::save(repositories, &room, expected, time(*clock))
        .await
        .expect("存得进去");
}

async fn load(repositories: &PostgresRepositories, catalog: RoomCatalogId) -> PrivateRoom {
    PrivateRoomStore::find_by_catalog(repositories, catalog)
        .await
        .expect("可读")
        .expect("有房间")
        .room()
        .clone()
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
