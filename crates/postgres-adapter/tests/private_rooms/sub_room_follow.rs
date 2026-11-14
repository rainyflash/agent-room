//! 子房间跟着父房间变（`specs/redesign/sub-rooms.md` 的“对账”，2c-1）：存父房间的那个事务里，受这次
//! 改动影响的子房间照父房间重算成员记录、房主和状态，聊天服务器上要跟着做的排进
//! `private_room_sub_room_sync`；照到的版本落后的由补对账补上。

use std::collections::BTreeMap;

use agent_room_application::ports::{
    PrivateRoomListingSaveOutcome, PrivateRoomListingStore, RoomRoleChangeLog, RoomRoleChangeRecord,
};

use super::{
    membership::{change, enter},
    *,
};

const JOINED: PrivateRoomMembershipStatus = PrivateRoomMembershipStatus::Joined;
const REMOVED: PrivateRoomMembershipStatus = PrivateRoomMembershipStatus::Removed;
const NOBODY: PrivateRoomPermissions = PrivateRoomPermissions::NONE;
const EVERYTHING: PrivateRoomPermissions = PrivateRoomPermissions::ALL;

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 被移出_退出_被封禁的人从子房间一起出去_管理员记录跟着删_要做的事排上() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "sub-room-follow-out-owner").await;
    let creator = seed_principal(pool, "sub-room-follow-out-creator").await;
    let leaver = seed_principal(pool, "sub-room-follow-out-leaver").await;
    let barred = seed_principal(pool, "sub-room-follow-out-barred").await;
    let stayer = seed_principal(pool, "sub-room-follow-out-stayer").await;
    let parent = create_room(&repositories, owner).await;
    for person in [creator, leaver, barred, stayer] {
        admit(&repositories, parent, owner, person).await;
    }
    let alpha = draft(&repositories, parent, creator, "Alpha", time(10)).await;
    let beta = draft(&repositories, parent, owner, "Beta", time(11)).await;
    for sub_room in [&alpha, &beta] {
        assert_eq!(create(&repositories, sub_room).await, None);
    }
    let (alpha, beta) = (id(&alpha), id(&beta));
    for (person, at) in [(leaver, 20), (barred, 21), (stayer, 22)] {
        assert!(enter(&repositories, parent, alpha, person, time(at)).await);
    }
    assert!(enter(&repositories, parent, beta, stayer, time(23)).await);
    let stale = load(&repositories, alpha).await;
    let untouched = load(&repositories, beta).await.room().version();

    // 建这一间的人被移出父房间：这一间里他一起出去，管理员记录跟着删，记一笔没有人动手的身份变动，
    // 聊天服务器上踢他的事排上。他不在的那一间不动，只记照到了父房间的新版本。
    change(&repositories, parent, time(50), |room| {
        room.remove_member(owner, creator).expect("房主能移出");
    })
    .await;
    let above = load(&repositories, parent).await.room().version();
    let followed = load(&repositories, alpha).await;
    assert_eq!(
        followed.room().version(),
        stale.room().version().next().expect("版本有效")
    );
    assert_eq!(
        standing(&repositories, alpha, creator).await,
        Some((REMOVED, NOBODY))
    );
    assert!(
        list(&repositories, parent, owner).await[0]
            .managers
            .is_empty()
    );
    let log = role_log(&repositories, alpha).await;
    assert_eq!(log.len(), 1);
    assert_eq!((log[0].actor, log[0].target), (None, creator));
    assert_eq!(log[0].next.status, REMOVED);
    assert_eq!(log[0].changed_at, time(50));
    assert_eq!(queued(pool, alpha).await, waiting([(creator, 1)]));
    assert_eq!(load(&repositories, beta).await.room().version(), untouched);
    assert!(queued(pool, beta).await.is_empty());
    assert_eq!(applied(pool, alpha).await, above);
    assert_eq!(applied(pool, beta).await, above);

    // 读到子房间以后它跟着父房间改过一次：拿旧的那份去存，版本对不上。
    let mut outdated = stale.room().clone();
    outdated.remove_member(owner, stayer).expect("房主能移出");
    let refused =
        PrivateRoomStore::save(&repositories, &outdated, stale.room().version(), time(51))
            .await
            .expect_err("子房间已经改过");
    assert_eq!(refused.kind(), RepositoryErrorKind::Conflict);

    // 自己退出父房间的、被父房间封禁的：子房间里都记成已移出（封禁记在父房间上，进不了父房间就进不了
    // 子房间）。留在父房间里的人原样。
    change(&repositories, parent, time(60), |room| {
        room.leave(leaver).expect("能退出");
    })
    .await;
    change(&repositories, parent, time(70), |room| {
        room.ban_member(owner, barred).expect("房主能封禁");
    })
    .await;
    for gone in [leaver, barred] {
        assert_eq!(
            standing(&repositories, alpha, gone).await,
            Some((REMOVED, NOBODY))
        );
    }
    assert_eq!(
        queued(pool, alpha).await,
        waiting([(creator, 1), (leaver, 1), (barred, 1)])
    );
    for sub_room in [alpha, beta] {
        assert_eq!(
            standing(&repositories, sub_room, stayer).await,
            Some((JOINED, ordinary_permissions()))
        );
    }
    assert_eq!(load(&repositories, beta).await.room().version(), untouched);
    let above = load(&repositories, parent).await.room().version();
    assert_eq!(applied(pool, beta).await, above);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 父房间里改了权限_子房间里照着重算_这一间的管理员还管这一间() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "sub-room-follow-rights-owner").await;
    let admin = seed_principal(pool, "sub-room-follow-rights-admin").await;
    let member = seed_principal(pool, "sub-room-follow-rights-member").await;
    let parent = create_room(&repositories, owner).await;
    for person in [admin, member] {
        admit(&repositories, parent, owner, person).await;
    }
    let alpha = draft(&repositories, parent, owner, "Alpha", time(10)).await;
    let gamma = draft(&repositories, parent, member, "Gamma", time(11)).await;
    let beta = draft(&repositories, parent, owner, "Beta", time(12)).await;
    for sub_room in [&alpha, &gamma, &beta] {
        assert_eq!(create(&repositories, sub_room).await, None);
    }
    let (alpha, gamma, beta) = (id(&alpha), id(&gamma), id(&beta));
    assert!(enter(&repositories, parent, alpha, admin, time(20)).await);
    assert!(enter(&repositories, parent, alpha, member, time(21)).await);
    let view = PrivateRoomCapability::View;
    let speak = PrivateRoomCapability::Speak;
    let manage = PrivateRoomCapability::Manage;

    // 设成父房间的管理员：他在的子房间里也有“管理”，记一笔；别人不动。
    change(&repositories, parent, time(30), |room| {
        room.update_permissions(owner, admin, holding([view, speak, manage]))
            .expect("房主能设管理员");
    })
    .await;
    assert_eq!(
        standing(&repositories, alpha, admin).await,
        Some((JOINED, holding([view, speak, manage])))
    );
    assert_eq!(
        standing(&repositories, alpha, member).await,
        Some((JOINED, ordinary_permissions()))
    );
    let log = role_log(&repositories, alpha).await;
    assert_eq!(log.len(), 1);
    assert_eq!((log[0].actor, log[0].target), (None, admin));
    assert_eq!(log[0].next.permissions, holding([view, speak, manage]));
    assert_eq!(queued(pool, alpha).await, waiting([(admin, 1)]));
    assert!(queued(pool, gamma).await.is_empty());

    // 父房间里改成只能看：子房间里也不能说话了；他自己建的那一间里“管理”还在（他是那一间的管理员）。
    change(&repositories, parent, time(40), |room| {
        room.update_permissions(owner, member, holding([view]))
            .expect("房主能改权限");
    })
    .await;
    assert_eq!(
        standing(&repositories, alpha, member).await,
        Some((JOINED, holding([view])))
    );
    assert_eq!(
        standing(&repositories, gamma, member).await,
        Some((JOINED, holding([view, manage])))
    );
    assert_eq!(
        list(&repositories, parent, owner).await[1].managers,
        [member]
    );
    assert_eq!(queued(pool, gamma).await, waiting([(member, 1)]));

    // 撤掉父房间的管理员：子房间里的“管理”一起没。同一个人又排一次，换成新的一版。
    change(&repositories, parent, time(50), |room| {
        room.update_permissions(owner, admin, ordinary_permissions())
            .expect("房主能撤管理员");
    })
    .await;
    assert_eq!(
        standing(&repositories, alpha, admin).await,
        Some((JOINED, ordinary_permissions()))
    );
    assert_eq!(
        queued(pool, alpha).await,
        waiting([(admin, 2), (member, 1)])
    );

    // 谁都没进过的那一间一次也没改，照到的版本跟着父房间走。
    let above = load(&repositories, parent).await.room().version();
    assert_eq!(
        load(&repositories, beta).await.room().version(),
        AggregateVersion::INITIAL
    );
    for sub_room in [alpha, gamma, beta] {
        assert_eq!(applied(pool, sub_room).await, above);
    }

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 父房间转让了_子房间的房主一起换_新房主没进过的_退出过的_归档了的都一样() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "sub-room-follow-heir-owner").await;
    let heir = seed_principal(pool, "sub-room-follow-heir-heir").await;
    let parent = create_room(&repositories, owner).await;
    admit(&repositories, parent, owner, heir).await;
    let mut sub_rooms = Vec::new();
    for (name, at) in [("Alpha", 10), ("Beta", 11), ("Gamma", 12), ("Delta", 13)] {
        let sub_room = draft(&repositories, parent, owner, name, time(at)).await;
        assert_eq!(create(&repositories, &sub_room).await, None);
        sub_rooms.push(id(&sub_room));
    }
    let [alpha, beta, gamma, delta] = sub_rooms[..] else {
        unreachable!("建了四间");
    };
    // 接手的人在第一间里；第二间进过又退出了；第三间没进过；第四间归档了。
    assert!(enter(&repositories, parent, alpha, heir, time(20)).await);
    assert!(enter(&repositories, parent, beta, heir, time(21)).await);
    change(&repositories, beta, time(22), |room| {
        room.leave(heir).expect("能退出");
    })
    .await;
    change(&repositories, delta, time(23), |room| {
        room.archive(owner).expect("房主能归档");
    })
    .await;

    let above = load(&repositories, parent).await;
    let mut room = above.room().clone();
    room.transfer_ownership(owner, heir, ordinary_permissions())
        .expect("房主能转让");
    let outcome = PrivateRoomListingStore::save_transferred_room(
        &repositories,
        &room,
        above.room().version(),
        time(50),
    )
    .await
    .expect("写入不报错");
    assert_eq!(outcome, PrivateRoomListingSaveOutcome::Saved);

    for sub_room in [alpha, beta, gamma, delta] {
        let stored = load(&repositories, sub_room).await;
        assert_eq!(stored.catalog().owner_principal_id(), Some(heir));
        assert_eq!(stored.room().owner_principal_id(), heir);
        assert_eq!(
            standing(&repositories, sub_room, heir).await,
            Some((JOINED, EVERYTHING))
        );
        // 原房主照他在父房间里剩下的权限留在里面。
        assert_eq!(
            standing(&repositories, sub_room, owner).await,
            Some((JOINED, ordinary_permissions()))
        );
        // 两个人的身份都变了，都不是谁在这一间里动的手。
        let log = role_log(&repositories, sub_room).await;
        assert_eq!(log.len(), 2);
        assert!(log.iter().all(|change| change.actor.is_none()));
        assert_eq!(
            queued(pool, sub_room).await,
            waiting([(owner, 1), (heir, 1)])
        );
        assert_eq!(applied(pool, sub_room).await, room.version());
    }
    // 归档了的还是归档着，没有因为换房主被当成跟着父房间归档。
    let closed = load(&repositories, delta).await;
    assert_eq!(closed.room().status(), PrivateRoomLifecycleStatus::Archived);
    assert_eq!(archived_as(pool, delta).await, None);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 父房间归档了_没归档的子房间跟着归档_记下原因_自己归档过的不动() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "sub-room-follow-close-owner").await;
    let member = seed_principal(pool, "sub-room-follow-close-member").await;
    let parent = create_room(&repositories, owner).await;
    admit(&repositories, parent, owner, member).await;
    let alpha = draft(&repositories, parent, owner, "Alpha", time(10)).await;
    let beta = draft(&repositories, parent, member, "Beta", time(11)).await;
    let gamma = draft(&repositories, parent, owner, "Gamma", time(12)).await;
    for sub_room in [&alpha, &beta, &gamma] {
        assert_eq!(create(&repositories, sub_room).await, None);
    }
    let (alpha, beta, gamma) = (id(&alpha), id(&beta), id(&gamma));
    // 第三间自己先归档了（归档子房间的入口是 2e 的事，这里照它要写的样子直接写表）。
    change(&repositories, gamma, time(30), |room| {
        room.archive(owner).expect("房主能归档");
    })
    .await;
    sqlx::query(
        r"UPDATE agent_room.private_room_parent
             SET archived_at = to_timestamp($2::double precision / 1000.0),
                 archived_by_principal_id = $3,
                 archived_reason = 'manual'
           WHERE catalog_entry_id = $1",
    )
    .bind(gamma.as_uuid())
    .bind(time(30).value())
    .bind(owner.as_uuid())
    .execute(pool)
    .await
    .expect("记下归档");
    let manual = load(&repositories, gamma).await.room().version();

    change(&repositories, parent, time(50), |room| {
        room.archive(owner).expect("房主能归档");
    })
    .await;
    let above = load(&repositories, parent).await.room().version();

    for sub_room in [alpha, beta] {
        let stored = load(&repositories, sub_room).await;
        assert_eq!(stored.room().status(), PrivateRoomLifecycleStatus::Archived);
        assert_eq!(stored.catalog().status(), RoomCatalogStatus::Archived);
        assert_eq!(stored.instance().state(), RoomInstanceState::Archived);
        // 记下什么时候、为什么；没有“谁”。聊天服务器上锁住它的事排上，成员一个没动。
        assert_eq!(
            archived_as(pool, sub_room).await,
            Some((time(50).value(), "parent".to_owned(), None))
        );
        assert_eq!(queued(pool, sub_room).await, BTreeMap::from([(None, 1)]));
    }
    assert_eq!(
        standing(&repositories, beta, member).await,
        Some((
            JOINED,
            holding([
                PrivateRoomCapability::View,
                PrivateRoomCapability::Speak,
                PrivateRoomCapability::Manage
            ])
        ))
    );
    let listed = list(&repositories, parent, owner).await;
    assert!(listed.iter().all(|sub_room| sub_room.archived));
    assert_eq!(listed[1].managers, [member]);

    // 自己归档过的那一间一点没动：版本、原因、谁归档的都是原来的。
    assert_eq!(load(&repositories, gamma).await.room().version(), manual);
    assert_eq!(
        archived_as(pool, gamma).await,
        Some((time(30).value(), "manual".to_owned(), Some(owner.as_uuid())))
    );
    assert!(queued(pool, gamma).await.is_empty());
    for sub_room in [alpha, beta, gamma] {
        assert_eq!(applied(pool, sub_room).await, above);
    }

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 照到的版本落后的子房间由补对账补上_补过的不再补() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "sub-room-follow-stale-owner").await;
    let member = seed_principal(pool, "sub-room-follow-stale-member").await;
    let parent = create_room(&repositories, owner).await;
    admit(&repositories, parent, owner, member).await;
    let alpha = draft(&repositories, parent, owner, "Alpha", time(10)).await;
    let beta = draft(&repositories, parent, owner, "Beta", time(11)).await;
    for sub_room in [&alpha, &beta] {
        assert_eq!(create(&repositories, sub_room).await, None);
    }
    let (alpha, beta) = (id(&alpha), id(&beta));
    assert!(enter(&repositories, parent, alpha, member, time(20)).await);
    let untouched = load(&repositories, beta).await.room().version();

    // 回滚到不认得子房间的旧版时，存父房间只改父房间自己的几张表：照它留下的样子直接写表。
    sqlx::query(
        r"UPDATE agent_room.private_room_membership
             SET membership_status = 'removed', permission_bits = 0,
                 status_changed_at = to_timestamp($3::double precision / 1000.0)
           WHERE catalog_entry_id = $1 AND principal_id = $2",
    )
    .bind(parent.as_uuid())
    .bind(member.as_uuid())
    .bind(time(40).value())
    .execute(pool)
    .await
    .expect("旧版移出了他");
    sqlx::query(
        r"UPDATE agent_room.private_room_state
             SET version = version + 1,
                 updated_at = to_timestamp($2::double precision / 1000.0)
           WHERE catalog_entry_id = $1",
    )
    .bind(parent.as_uuid())
    .bind(time(40).value())
    .execute(pool)
    .await
    .expect("旧版存了父房间");
    let above = load(&repositories, parent).await.room().version();
    assert_ne!(applied(pool, alpha).await, above);
    assert_eq!(
        standing(&repositories, alpha, member).await,
        Some((JOINED, ordinary_permissions())),
        "还没补"
    );

    // 两间都落后：他在的那一间把他记成移出、排上踢他的事；另一间没有要改的，只记照到了哪一版。
    let followed = SubRoomStore::follow_stale_sub_rooms(&repositories, 10, time(60))
        .await
        .expect("补得上");
    assert_eq!(followed, 2);
    assert_eq!(
        standing(&repositories, alpha, member).await,
        Some((REMOVED, NOBODY))
    );
    assert_eq!(queued(pool, alpha).await, waiting([(member, 1)]));
    assert!(
        role_log(&repositories, alpha).await.is_empty(),
        "普通成员出去不记"
    );
    assert_eq!(load(&repositories, beta).await.room().version(), untouched);
    assert!(queued(pool, beta).await.is_empty());
    for sub_room in [alpha, beta] {
        assert_eq!(applied(pool, sub_room).await, above);
    }

    let again = SubRoomStore::follow_stale_sub_rooms(&repositories, 10, time(70))
        .await
        .expect("可查");
    assert_eq!(again, 0);
    assert_eq!(queued(pool, alpha).await, waiting([(member, 1)]));

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 建子房间的当口父房间变了_照现在的父房间再算一遍() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let pool = &database.runtime;
    let owner = seed_principal(pool, "sub-room-follow-birth-owner").await;
    let creator = seed_principal(pool, "sub-room-follow-birth-creator").await;
    let parent = create_room(&repositories, owner).await;
    admit(&repositories, parent, owner, creator).await;

    // 照着读到的父房间备好了子房间，存之前建的人被移出了父房间。
    let alpha = draft(&repositories, parent, creator, "Alpha", time(10)).await;
    change(&repositories, parent, time(20), |room| {
        room.remove_member(owner, creator).expect("房主能移出");
    })
    .await;
    let above = load(&repositories, parent).await.room().version();
    assert_ne!(alpha.parent_version, above);
    assert_eq!(create(&repositories, &alpha).await, None);
    let alpha = id(&alpha);
    assert_eq!(
        standing(&repositories, alpha, creator).await,
        Some((REMOVED, NOBODY))
    );
    assert_eq!(
        standing(&repositories, alpha, owner).await,
        Some((JOINED, EVERYTHING))
    );
    assert_eq!(
        load(&repositories, alpha).await.room().version(),
        AggregateVersion::INITIAL.next().expect("版本有效")
    );
    assert!(
        list(&repositories, parent, owner).await[0]
            .managers
            .is_empty()
    );
    assert_eq!(queued(pool, alpha).await, waiting([(creator, 1)]));
    assert_eq!(applied(pool, alpha).await, above);

    // 父房间没变过的照旧：存下来的就是备好的那一份。
    let beta = draft(&repositories, parent, owner, "Beta", time(30)).await;
    assert_eq!(create(&repositories, &beta).await, None);
    let beta = id(&beta);
    assert_eq!(
        load(&repositories, beta).await.room().version(),
        AggregateVersion::INITIAL
    );
    assert!(queued(pool, beta).await.is_empty());
    assert_eq!(applied(pool, beta).await, above);

    database.close().await;
}

fn id(sub_room: &NewSubRoom) -> RoomCatalogId {
    sub_room.snapshot.catalog().id()
}

fn holding<const N: usize>(capabilities: [PrivateRoomCapability; N]) -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities(capabilities).expect("权限有效")
}

/// 排着的成员的事：谁、第几版。
fn waiting<const N: usize>(members: [(PrincipalId, i64); N]) -> BTreeMap<Option<PrincipalId>, i64> {
    members
        .into_iter()
        .map(|(principal, generation)| (Some(principal), generation))
        .collect()
}

/// 一个人在这间房里的成员状态和权限；名单里没有他时为空。
async fn standing(
    repositories: &PostgresRepositories,
    catalog: RoomCatalogId,
    principal: PrincipalId,
) -> Option<(PrivateRoomMembershipStatus, PrivateRoomPermissions)> {
    load(repositories, catalog)
        .await
        .room()
        .member(principal)
        .map(|member| (member.status(), member.permissions()))
}

async fn role_log(
    repositories: &PostgresRepositories,
    catalog: RoomCatalogId,
) -> Vec<RoomRoleChangeRecord> {
    RoomRoleChangeLog::recent_role_changes(repositories, catalog, 10)
        .await
        .expect("可读")
}

/// 这间子房间照到了父房间的哪个版本。
async fn applied(pool: &PgPool, sub_room: RoomCatalogId) -> AggregateVersion {
    let applied: i64 = sqlx::query_scalar(
        r"SELECT parent_version_applied
            FROM agent_room.private_room_parent
           WHERE catalog_entry_id = $1",
    )
    .bind(sub_room.as_uuid())
    .fetch_one(pool)
    .await
    .expect("父子关系可读");
    version(applied)
}

/// 这间子房间排着的、聊天服务器上还没做的事：成员的按人，房间自己的那一条没有人。值是第几版。
async fn queued(pool: &PgPool, sub_room: RoomCatalogId) -> BTreeMap<Option<PrincipalId>, i64> {
    let rows: Vec<(Option<Uuid>, i64)> = sqlx::query_as(
        r"SELECT principal_id, generation
            FROM agent_room.private_room_sub_room_sync
           WHERE catalog_entry_id = $1",
    )
    .bind(sub_room.as_uuid())
    .fetch_all(pool)
    .await
    .expect("排着的事可读");
    rows.into_iter()
        .map(|(principal, generation)| (principal.map(PrincipalId::from_uuid), generation))
        .collect()
}

/// 子房间那张表里记的归档：什么时候、为什么、谁。没归档为空。
async fn archived_as(
    pool: &PgPool,
    sub_room: RoomCatalogId,
) -> Option<(i64, String, Option<Uuid>)> {
    sqlx::query_as(
        r"SELECT floor(extract(epoch FROM archived_at) * 1000)::bigint,
                 archived_reason,
                 archived_by_principal_id
            FROM agent_room.private_room_parent
           WHERE catalog_entry_id = $1
             AND archived_at IS NOT NULL",
    )
    .bind(sub_room.as_uuid())
    .fetch_optional(pool)
    .await
    .expect("父子关系可读")
}
