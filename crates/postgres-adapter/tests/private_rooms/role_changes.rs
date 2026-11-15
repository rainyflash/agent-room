//! 身份变动记录（`specs/redesign/room-roles.md` 的“记录”）：和房间在同一个事务里记，读回来新的在前，
//! 没存成的不记；转让走另一条保存的路，也记；下载的数据里只带和这个人有关的。

use agent_room_application::ports::{
    AccountDeletionRepository, PrivateRoomListingSaveOutcome, PrivateRoomListingStore,
    RoomRoleChange, RoomRoleChangeLog,
};
use agent_room_domain::{
    error::DomainError,
    room_role_changes::{RoomStanding, standing_changes},
};

use super::{listing::create_room, *};

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 身份变动跟着房间一起存_读回来新的在前_没存成的不记() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "role-changes-owner").await;
    let member = seed_principal(&database.runtime, "role-changes-member").await;
    let catalog = create_room(&repositories, owner).await;
    assert_eq!(recent(&repositories, catalog).await, [], "刚建的房间没有");

    // 请成员进来、成员进来：名单的事，不记。
    let mut clock = 0;
    change(&repositories, catalog, owner, &mut clock, |room| {
        room.invite(owner, member, ordinary_permissions())
    })
    .await;
    change(&repositories, catalog, member, &mut clock, |room| {
        room.accept_invitation(member)
    })
    .await;
    assert_eq!(recent(&repositories, catalog).await, []);

    // 两头同时读到了同一份房间，都要把这个成员设成管理员。
    let stale = load(&repositories, catalog).await;
    let mut attempt = stale.clone();
    attempt
        .update_permissions(owner, member, admin_permissions())
        .expect("房主能设");
    let late = changes(&stale, &attempt, owner);
    assert_eq!(late.len(), 1, "后到的那一头本来也要记一条");

    // 先到的那一头：一条，动手的是房主，时间是这次保存的时间。
    change(&repositories, catalog, owner, &mut clock, |room| {
        room.update_permissions(owner, member, admin_permissions())
    })
    .await;
    let promoted_at = time(clock);
    let promoted = recent(&repositories, catalog).await;
    assert_eq!(promoted.len(), 1);
    assert_eq!(promoted[0].actor, Some(owner));
    assert_eq!(promoted[0].target, member);
    assert_eq!(promoted[0].previous, Some(joined(ordinary_permissions())));
    assert_eq!(promoted[0].next, joined(admin_permissions()));
    assert_eq!(promoted[0].changed_at, promoted_at);

    // 后到的那一头照它读到的版本存：版本对不上，整个不算。房间没动，记录也没多。
    let refused =
        PrivateRoomStore::save_recording(&repositories, &attempt, stale.version(), time(99), &late)
            .await
            .expect_err("旧版本存不进去");
    assert_eq!(refused.kind(), RepositoryErrorKind::Conflict);
    assert_eq!(recent(&repositories, catalog).await, promoted);
    assert_eq!(
        load(&repositories, catalog).await.version(),
        attempt.version(),
        "房间只往前走了先到的那一步"
    );

    // 转让房主走另一条保存的路：原房主一条、接手的人一条，在同一个事务里。
    clock += 1;
    transfer(&repositories, catalog, owner, member, time(clock)).await;
    let all = recent(&repositories, catalog).await;
    assert_eq!(all.len(), 3, "{all:?}");
    assert!(all[0].id > all[1].id && all[1].id > all[2].id, "新的在前");
    assert_eq!(all[2], promoted[0]);
    let handed_over: Vec<_> = all[..2].iter().map(|record| record.target).collect();
    assert!(handed_over.contains(&owner) && handed_over.contains(&member));
    for record in &all[..2] {
        assert_eq!(record.actor, Some(owner));
        assert_eq!(record.changed_at, time(clock));
        assert_eq!(record.next.owner, record.target == member);
        assert_eq!(
            record.previous.map(|standing| standing.owner),
            Some(record.target == owner)
        );
    }

    // 只要最近的一条。
    let latest = RoomRoleChangeLog::recent_role_changes(&repositories, catalog, 1)
        .await
        .expect("可读");
    assert_eq!(latest, [all[0].clone()]);

    assert_exports(&repositories, catalog, owner, member).await;

    database.close().await;
}

/// 房主把房间转让给已经在房间里的人，照用例的做法连记录一起存。
async fn transfer(
    repositories: &PostgresRepositories,
    catalog: RoomCatalogId,
    owner: PrincipalId,
    receiver: PrincipalId,
    at: UtcMillis,
) {
    let before = load(repositories, catalog).await;
    let mut transferred = before.clone();
    transferred
        .transfer_ownership(owner, receiver, ordinary_permissions())
        .expect("房主能转让");
    assert_eq!(
        PrivateRoomListingStore::save_transferred_room_recording(
            repositories,
            &transferred,
            before.version(),
            at,
            &changes(&before, &transferred, owner),
        )
        .await
        .expect("写入不报错"),
        PrivateRoomListingSaveOutcome::Saved
    );
}

/// 下载的数据里只带和这个人有关的，写的是“是不是关于我”“是不是我动的手”，不带别人的 ID。
async fn assert_exports(
    repositories: &PostgresRepositories,
    catalog: RoomCatalogId,
    former_owner: PrincipalId,
    new_owner: PrincipalId,
) {
    // 接手的人那份有两条，说的都是他（被设成管理员、接手房主），都不是他动的手。
    let own = exported_role_changes(repositories, new_owner).await;
    assert_eq!(own.len(), 2, "{own:?}");
    for entry in &own {
        assert_eq!(entry["roomId"], catalog.to_string());
        assert_eq!(entry["aboutMe"], true);
        assert_eq!(entry["byMe"], false);
        assert!(
            entry.get("actorPrincipalId").is_none() && entry.get("targetPrincipalId").is_none()
        );
    }
    assert_eq!(own[0]["nextOwner"], false);
    assert_eq!(own[0]["nextPermissionBits"], admin_permissions().bits());
    assert_eq!(own[1]["nextOwner"], true);

    // 原房主那份：三条都是他动的手，其中一条说的也是他。
    let made = exported_role_changes(repositories, former_owner).await;
    assert_eq!(made.len(), 3, "{made:?}");
    assert!(made.iter().all(|entry| entry["byMe"] == true));
    assert_eq!(
        made.iter().filter(|entry| entry["aboutMe"] == true).count(),
        1
    );
}

async fn exported_role_changes(
    repositories: &PostgresRepositories,
    principal: PrincipalId,
) -> Vec<serde_json::Value> {
    AccountDeletionRepository::export(repositories, principal, time(1_000))
        .await
        .expect("导出成功")
        .expect("账户在")
        .data["roleChanges"]
        .as_array()
        .expect("是一个列表")
        .clone()
}

/// 读出房间、做一步、连同这一步动了谁的身份一起存回去。
async fn change(
    repositories: &PostgresRepositories,
    catalog: RoomCatalogId,
    actor: PrincipalId,
    clock: &mut i64,
    step: impl FnOnce(&mut PrivateRoom) -> Result<bool, DomainError>,
) {
    let before = load(repositories, catalog).await;
    let mut room = before.clone();
    assert!(step(&mut room).expect("这一步做得了"), "这一步真的改了房间");
    *clock += 1;
    PrivateRoomStore::save_recording(
        repositories,
        &room,
        before.version(),
        time(*clock),
        &changes(&before, &room, actor),
    )
    .await
    .expect("存得进去");
}

/// 和用例里一样：拿改之前、改之后的两份一比，补上是谁动的手。
fn changes(before: &PrivateRoom, after: &PrivateRoom, actor: PrincipalId) -> Vec<RoomRoleChange> {
    standing_changes(before, after)
        .into_iter()
        .map(|change| RoomRoleChange {
            actor: Some(actor),
            change,
        })
        .collect()
}

async fn recent(
    repositories: &PostgresRepositories,
    catalog: RoomCatalogId,
) -> Vec<agent_room_application::ports::RoomRoleChangeRecord> {
    RoomRoleChangeLog::recent_role_changes(repositories, catalog, 100)
        .await
        .expect("可读")
}

async fn load(repositories: &PostgresRepositories, catalog: RoomCatalogId) -> PrivateRoom {
    PrivateRoomStore::find_by_catalog(repositories, catalog)
        .await
        .expect("可读")
        .expect("有房间")
        .room()
        .clone()
}

fn joined(permissions: PrivateRoomPermissions) -> RoomStanding {
    RoomStanding {
        status: PrivateRoomMembershipStatus::Joined,
        owner: false,
        permissions,
    }
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
