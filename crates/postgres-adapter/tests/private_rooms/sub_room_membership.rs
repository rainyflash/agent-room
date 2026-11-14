//! 子房间的存储（`specs/redesign/sub-rooms.md` 的 2b）：把人记进子房间之前，在同一个事务里再看他
//! 在不在父房间；这一间的管理员不在里面了，记录跟着删；改名字和用途一起存，重名在锁着父房间的
//! 事务里看。

use agent_room_application::ports::{
    RoomRoleChange, RoomRoleChangeLog, SubRoomAdmission, SubRoomRevision, SubRoomRevisionBlock,
};
use agent_room_domain::room_role_changes::standing_changes;

use super::*;

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 记进子房间之前再看一眼他在不在父房间_不在的什么都不写_版本对不上照旧拒() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-room-entry-owner").await;
    let member = seed_principal(&database.runtime, "sub-room-entry-member").await;
    let late = seed_principal(&database.runtime, "sub-room-entry-late").await;
    let leaver = seed_principal(&database.runtime, "sub-room-entry-leaver").await;
    let outsider = seed_principal(&database.runtime, "sub-room-entry-outsider").await;
    let parent = create_room(&repositories, owner).await;
    let elsewhere = create_room(&repositories, owner).await;
    for person in [member, late, leaver] {
        admit(&repositories, parent, owner, person).await;
    }
    admit(&repositories, elsewhere, owner, outsider).await;
    let alpha = draft(&repositories, parent, owner, "Alpha", time(10)).await;
    assert_eq!(create(&repositories, &alpha).await, None);
    let sub_room = alpha.snapshot.catalog().id();

    assert!(enter(&repositories, parent, sub_room, member, time(20)).await);
    let stored = load(&repositories, sub_room).await;
    let inside = stored.room().member(member).expect("记进去了");
    assert!(inside.has_joined(), "直接是已加入");
    assert_eq!(inside.permissions(), ordinary_permissions());
    let seen = list(&repositories, parent, member).await;
    assert_eq!(
        seen[0].viewer_status,
        Some(PrivateRoomMembershipStatus::Joined)
    );
    assert_eq!(seen[0].member_count, 2);

    // 不在父房间里的人；地址里的父房间对不上（他在那间房里，可这间子房间不挂在它下面）；刚被移出
    // 父房间的人。都什么都不写。
    let before = stored.room().version();
    assert!(!enter(&repositories, parent, sub_room, outsider, time(21)).await);
    assert!(!enter(&repositories, elsewhere, sub_room, outsider, time(21)).await);
    let above = load(&repositories, parent).await;
    let mut without = above.room().clone();
    without.remove_member(owner, leaver).expect("房主能移出");
    PrivateRoomStore::save(&repositories, &without, above.room().version(), time(22))
        .await
        .expect("移出存得进去");
    assert!(!enter(&repositories, parent, sub_room, leaver, time(23)).await);
    let untouched = load(&repositories, sub_room).await;
    assert_eq!(untouched.room().version(), before);
    assert!(untouched.room().member(outsider).is_none());
    assert!(untouched.room().member(leaver).is_none());

    // 读到以后别人先存了一次：版本对不上，照旧拒；交过来的房间不是“读到的版本加一”也拒。
    let mut stale = untouched.room().clone();
    stale
        .admit(late, ordinary_permissions())
        .expect("没有门能进");
    assert!(enter(&repositories, parent, sub_room, late, time(24)).await);
    let admission = SubRoomAdmission {
        room: &stale,
        expected_version: before,
        parent_catalog_id: parent,
        principal: late,
        at: time(25),
        role_changes: &[],
    };
    let refused = SubRoomStore::admit_to_sub_room(&repositories, &admission)
        .await
        .expect_err("别人先存了");
    assert_eq!(refused.kind(), RepositoryErrorKind::Conflict);
    let skipped = SubRoomStore::admit_to_sub_room(
        &repositories,
        &SubRoomAdmission {
            expected_version: stale.version(),
            ..admission
        },
    )
    .await
    .expect_err("版本没有加一");
    assert_eq!(skipped.kind(), RepositoryErrorKind::Constraint);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 管这一间的人进来时记一笔身份变动_普通成员进来不记() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-room-roles-owner").await;
    let member = seed_principal(&database.runtime, "sub-room-roles-member").await;
    let admin = seed_principal(&database.runtime, "sub-room-roles-admin").await;
    let parent = create_room(&repositories, owner).await;
    for person in [member, admin] {
        admit(&repositories, parent, owner, person).await;
    }
    let alpha = draft(&repositories, parent, owner, "Alpha", time(10)).await;
    assert_eq!(create(&repositories, &alpha).await, None);
    let sub_room = alpha.snapshot.catalog().id();
    let recorded = || async {
        RoomRoleChangeLog::recent_role_changes(&repositories, sub_room, 10)
            .await
            .expect("可读")
    };

    assert!(enter(&repositories, parent, sub_room, member, time(20)).await);
    assert!(recorded().await.is_empty(), "成员来来去去是名单的事");

    // 父房间的管理员进来就管这一间：和房间在同一个事务里记下。
    let before = load(&repositories, sub_room).await;
    let mut room = before.room().clone();
    let managing = PrivateRoomPermissions::from_capabilities([
        PrivateRoomCapability::View,
        PrivateRoomCapability::Speak,
        PrivateRoomCapability::Manage,
    ])
    .expect("权限有效");
    room.admit(admin, managing).expect("没有门能进");
    let changes: Vec<_> = standing_changes(before.room(), &room)
        .into_iter()
        .map(|change| RoomRoleChange {
            actor: Some(admin),
            change,
        })
        .collect();
    assert_eq!(changes.len(), 1);
    let admitted = SubRoomStore::admit_to_sub_room(
        &repositories,
        &SubRoomAdmission {
            room: &room,
            expected_version: before.room().version(),
            parent_catalog_id: parent,
            principal: admin,
            at: time(30),
            role_changes: &changes,
        },
    )
    .await
    .expect("写入不报错");
    assert!(admitted);
    let log = recorded().await;
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].actor, Some(admin));
    assert_eq!(log[0].target, admin);
    assert_eq!(log[0].previous, None, "之前不在这一间里");
    assert_eq!(log[0].next, changes[0].change.next);
    assert_eq!(log[0].changed_at, time(30));

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 这一间的管理员退出_被移出_被封禁_管理员记录跟着删_还在里面的留着() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-room-managers-owner").await;
    let creator = seed_principal(&database.runtime, "sub-room-managers-creator").await;
    let second = seed_principal(&database.runtime, "sub-room-managers-second").await;
    let third = seed_principal(&database.runtime, "sub-room-managers-third").await;
    let parent = create_room(&repositories, owner).await;
    for person in [creator, second, third] {
        admit(&repositories, parent, owner, person).await;
    }
    let alpha = draft(&repositories, parent, creator, "Alpha", time(10)).await;
    assert_eq!(create(&repositories, &alpha).await, None);
    let sub_room = alpha.snapshot.catalog().id();
    // 另设两位这一间的管理员（设管理员的入口是 2d 的事，这里照它要写的样子直接写表）。
    for (index, manager) in [second, third].into_iter().enumerate() {
        let offset = i64::try_from(index).expect("很小");
        assert!(enter(&repositories, parent, sub_room, manager, time(20 + offset)).await);
        sqlx::query(
            r"INSERT INTO agent_room.private_room_sub_room_manager (
                   catalog_entry_id, principal_id, granted_by_principal_id, granted_at
               ) VALUES ($1, $2, $3, to_timestamp($4::double precision / 1000.0))",
        )
        .bind(sub_room.as_uuid())
        .bind(manager.as_uuid())
        .bind(owner.as_uuid())
        .bind(time(30 + offset).value())
        .execute(&database.runtime)
        .await
        .expect("记下子房间管理员");
    }
    let managers = || async { list(&repositories, parent, owner).await[0].managers.clone() };
    assert_eq!(managers().await, [creator, second, third]);

    // 存一次别的改动（有人进来）：都还在里面，一个不少。
    let bystander = seed_principal(&database.runtime, "sub-room-managers-bystander").await;
    admit(&repositories, parent, owner, bystander).await;
    assert!(enter(&repositories, parent, sub_room, bystander, time(40)).await);
    assert_eq!(managers().await, [creator, second, third]);

    // 建的人自己退出；房主移出一位；房主封禁一位。
    change(&repositories, sub_room, time(51), |room| {
        room.leave(creator).expect("能退出");
    })
    .await;
    assert_eq!(managers().await, [second, third]);
    change(&repositories, sub_room, time(52), |room| {
        room.remove_member(owner, second).expect("房主能移出");
    })
    .await;
    assert_eq!(managers().await, [third]);
    change(&repositories, sub_room, time(53), |room| {
        room.ban_member(owner, third).expect("房主能封禁");
    })
    .await;
    assert!(managers().await.is_empty());

    // 退出过的人再进来是普通成员：记录没了，不会自己回来。
    assert!(enter(&repositories, parent, sub_room, creator, time(60)).await);
    assert!(managers().await.is_empty());

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 名字和用途一起改_撞了别的子房间不写_归档了的改不了_它的名字让出来() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-room-revise-owner").await;
    let parent = create_room(&repositories, owner).await;
    let elsewhere = create_room(&repositories, owner).await;
    let alpha = draft(&repositories, parent, owner, "Alpha", time(3)).await;
    let beta = draft(&repositories, parent, owner, "Beta", time(5)).await;
    for sub_room in [&alpha, &beta] {
        assert_eq!(create(&repositories, sub_room).await, None);
    }
    let (alpha, beta) = (alpha.snapshot.catalog().id(), beta.snapshot.catalog().id());
    let revision = |catalog_id, name: &str, description: &str| SubRoomRevision {
        catalog_id,
        parent_catalog_id: parent,
        name: name.to_owned(),
        name_key: sub_room_name_key(name),
        description: description.to_owned(),
        at: time(50),
    };

    assert_eq!(
        revise(&repositories, &revision(alpha, "Launch", "Ship it")).await,
        None
    );
    let stored = load(&repositories, alpha).await;
    assert_eq!(stored.catalog().name(), "Launch");
    assert_eq!(stored.catalog().description(), "Ship it");
    assert_eq!(stored.parent_catalog_id(), Some(parent));
    assert_eq!(name_key(&repositories, alpha).await, "launch");
    // 成员和版本没动：改名不是成员的事。
    assert_eq!(stored.room().version(), AggregateVersion::INITIAL);

    // 撞了别的子房间（不分大小写）：什么都不写。
    assert_eq!(
        revise(&repositories, &revision(beta, "LAUNCH", "mine now")).await,
        Some(SubRoomRevisionBlock::NameTaken)
    );
    let untouched = load(&repositories, beta).await;
    assert_eq!(untouched.catalog().name(), "Beta");
    assert_ne!(untouched.catalog().description(), "mine now");
    assert_eq!(name_key(&repositories, beta).await, "beta");
    // 和自己不算重名：只换大小写、只改用途都行。
    assert_eq!(
        revise(&repositories, &revision(alpha, "LAUNCH", "")).await,
        None
    );
    assert_eq!(load(&repositories, alpha).await.catalog().name(), "LAUNCH");
    // 新建的也照改过的名字比重名。
    let clash = draft(&repositories, parent, owner, "launch", time(60)).await;
    assert_eq!(
        create(&repositories, &clash).await,
        Some(SubRoomCreationBlock::NameTaken)
    );
    let freed = draft(&repositories, parent, owner, "alpha", time(60)).await;
    assert_eq!(
        create(&repositories, &freed).await,
        None,
        "原来的名字让出来了"
    );

    // 不挂在这间房下面的、没有的：改不了。
    let unavailable = Some(SubRoomRevisionBlock::Unavailable);
    let misplaced = SubRoomRevision {
        parent_catalog_id: elsewhere,
        ..revision(alpha, "Moved", "")
    };
    assert_eq!(revise(&repositories, &misplaced).await, unavailable);
    let missing = RoomCatalogId::from_uuid(Uuid::now_v7());
    assert_eq!(
        revise(&repositories, &revision(missing, "Ghost", "")).await,
        unavailable
    );
    assert_eq!(
        revise(&repositories, &revision(parent, "Parent", "")).await,
        unavailable,
        "父房间自己不是子房间"
    );

    // 归档了的改不了（归档是 2e 的事，这里照它要写的样子直接写表）；它的名字让出来。
    archive(&repositories, beta, owner).await;
    sqlx::query(
        r"UPDATE agent_room.private_room_parent
             SET archived_at = to_timestamp($2::double precision / 1000.0),
                 archived_by_principal_id = $3,
                 archived_reason = 'manual'
           WHERE catalog_entry_id = $1",
    )
    .bind(beta.as_uuid())
    .bind(time(10).value())
    .bind(owner.as_uuid())
    .execute(&database.runtime)
    .await
    .expect("记下归档");
    assert_eq!(
        revise(&repositories, &revision(beta, "Gamma", "")).await,
        unavailable
    );
    assert_eq!(
        revise(&repositories, &revision(alpha, "Beta", "took the name")).await,
        None
    );
    assert_eq!(name_key(&repositories, alpha).await, "beta");

    database.close().await;
}

/// 照用例的做法把一个人记进没有门的子房间：读房间、记成已加入、交给存储。回存储说的记没记上。
async fn enter(
    repositories: &PostgresRepositories,
    parent: RoomCatalogId,
    sub_room: RoomCatalogId,
    principal: PrincipalId,
    at: UtcMillis,
) -> bool {
    let before = load(repositories, sub_room).await;
    let mut room = before.room().clone();
    room.admit(principal, ordinary_permissions())
        .expect("没有门能进");
    SubRoomStore::admit_to_sub_room(
        repositories,
        &SubRoomAdmission {
            room: &room,
            expected_version: before.room().version(),
            parent_catalog_id: parent,
            principal,
            at,
            role_changes: &[],
        },
    )
    .await
    .expect("写入不报错")
}

/// 直接改存着的一间房再存：退出、移出、封禁这些走的是私人房间自己的保存。
async fn change(
    repositories: &PostgresRepositories,
    catalog: RoomCatalogId,
    at: UtcMillis,
    apply: impl FnOnce(&mut PrivateRoom),
) {
    let before = load(repositories, catalog).await;
    let mut room = before.room().clone();
    apply(&mut room);
    PrivateRoomStore::save(repositories, &room, before.room().version(), at)
        .await
        .expect("存得进去");
}

async fn name_key(repositories: &PostgresRepositories, catalog: RoomCatalogId) -> String {
    link(repositories, catalog)
        .await
        .expect("有父子关系")
        .name_key
}

async fn revise(
    repositories: &PostgresRepositories,
    revision: &SubRoomRevision,
) -> Option<SubRoomRevisionBlock> {
    SubRoomStore::revise_sub_room(repositories, revision)
        .await
        .expect("写入不报错")
}
