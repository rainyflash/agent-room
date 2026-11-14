//! 子房间的存储（`specs/redesign/sub-rooms.md` 的 2a）：房间、父子关系和子房间管理员一起存；重名、
//! 上限、限速在锁着父房间的事务里数；只有一层，数据库自己也拦着；一间房里子房间的三项设置按版本存。

use agent_room_application::ports::{
    NewSubRoom, SubRoomCreationBlock, SubRoomCreationCheck, SubRoomEntry, SubRoomLink,
    SubRoomParentFacts, SubRoomPolicyRecord, SubRoomStore,
};
use agent_room_domain::{
    private_rooms::PrivateRoomMember,
    room_listing::RoomJoinMode,
    sub_rooms::{
        MAXIMUM_OPEN_SUB_ROOMS, MAXIMUM_SUB_ROOMS_PER_CREATOR_PER_HOUR, SubRoomCreators,
        SubRoomPolicy, new_sub_room, sub_room_name_key,
    },
    version::AggregateVersion,
};

use super::{
    listing::{archive, create_room, version},
    *,
};

#[path = "sub_room_membership.rs"]
mod membership;

const HOUR_MS: i64 = 3_600_000;

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 子房间连同父子关系和管理员一起存_房间列表里没有它_重名和重发各有各的说法() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-rooms-owner").await;
    let member = seed_principal(&database.runtime, "sub-rooms-member").await;
    let parent = create_room(&repositories, owner).await;
    admit(&repositories, parent, owner, member).await;

    let design = draft(&repositories, parent, member, "Design", time(10)).await;
    let sub_room = design.snapshot.catalog().id();
    assert_eq!(attempt(&repositories, &design).await, None);

    // 房间本身照旧是一间私人房间，多记着它挂在谁下面。
    let stored = load(&repositories, sub_room).await;
    assert_eq!(stored.parent_catalog_id(), Some(parent));
    assert_eq!(stored.catalog().name(), "Design");
    assert_eq!(stored.room().owner_principal_id(), owner);
    assert!(
        stored
            .room()
            .member(member)
            .is_some_and(PrivateRoomMember::has_joined)
    );
    let by_matrix =
        PrivateRoomStore::find_by_matrix_room(&repositories, stored.instance().matrix_room_id())
            .await
            .expect("可读")
            .expect("有房间");
    assert_eq!(by_matrix.parent_catalog_id(), Some(parent));
    assert_eq!(load(&repositories, parent).await.parent_catalog_id(), None);
    assert_eq!(
        link(&repositories, sub_room).await,
        Some(SubRoomLink {
            catalog_id: sub_room,
            parent_catalog_id: parent,
            name_key: "design".to_owned(),
            access: RoomJoinMode::Open,
            hidden: false,
            created_by: member,
            created_by_agent: None,
            created_at: time(10),
        })
    );
    assert_eq!(link(&repositories, parent).await, None);
    assert_eq!(
        list(&repositories, parent, owner).await[0].managers,
        [member],
        "建的人自动是这一间的子房间管理员"
    );

    // 房间列表只列父房间：子房间由父房间的子房间名单给出。
    for principal in [owner, member] {
        let listed: Vec<_> = PrivateRoomStore::list_for_principal(&repositories, principal)
            .await
            .expect("可读")
            .iter()
            .map(|room| room.catalog().id())
            .collect();
        assert_eq!(listed, [parent]);
    }

    // 同一个请求重发：先问不拦（不算它自己），存的时候撞上房间号，用例按重试处理。
    assert_eq!(blocked(&repositories, &design).await, None);
    let retried = SubRoomStore::create_sub_room(&repositories, &design)
        .await
        .expect_err("房间号已经有了");
    assert_eq!(retried.kind(), RepositoryErrorKind::Conflict);

    // 另一间叫同一个名字的（不分大小写）：什么都不写。
    let again = draft(&repositories, parent, owner, "DESIGN", time(20)).await;
    assert_eq!(
        attempt(&repositories, &again).await,
        Some(SubRoomCreationBlock::NameTaken)
    );
    let unwritten = PrivateRoomStore::find_by_catalog(&repositories, again.snapshot.catalog().id())
        .await
        .expect("可读");
    assert!(unwritten.is_none());
    assert_eq!(list(&repositories, parent, owner).await.len(), 1);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 只有一层_父房间归档了或者不在了就建不了_数据库自己也拦着() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-rooms-level-owner").await;
    let parent = create_room(&repositories, owner).await;
    let first = draft(&repositories, parent, owner, "First", time(10)).await;
    let sub_room = first.snapshot.catalog().id();
    assert_eq!(attempt(&repositories, &first).await, None);
    let unavailable = Some(SubRoomCreationBlock::ParentUnavailable);

    // 子房间下面不能再开。
    let nested = draft(&repositories, sub_room, owner, "Nested", time(20)).await;
    assert_eq!(attempt(&repositories, &nested).await, unavailable);

    // 归档了的房间下面不能开。
    let closed = create_room(&repositories, owner).await;
    archive(&repositories, closed, owner).await;
    let late = draft(&repositories, closed, owner, "Late", time(20)).await;
    assert_eq!(attempt(&repositories, &late).await, unavailable);

    // 没有这间房。
    let nowhere = SubRoomCreationCheck {
        parent_catalog_id: RoomCatalogId::from_uuid(Uuid::now_v7()),
        ..late.check()
    };
    let asked = SubRoomStore::sub_room_creation_block(&repositories, &nowhere)
        .await
        .expect("可查");
    assert_eq!(asked, unavailable);

    // 绕过存储直接写表：表上的触发器和约束拦着。
    let other = create_room(&repositories, owner).await;
    for (below, above, hidden, reason) in [
        (other, sub_room, false, "子房间下面再挂一间"),
        (parent, other, false, "已经有子房间的房间挂到别的房间下面"),
        (other, other, false, "挂在自己下面"),
        (other, parent, true, "没有门的不能隐藏"),
    ] {
        let refused = sqlx::query(
            r"INSERT INTO agent_room.private_room_parent (
                   catalog_entry_id, parent_catalog_entry_id, name_key, access, hidden,
                   created_by_principal_id, parent_version_applied, created_at
               ) VALUES ($1, $2, 'direct', 'open', $3, $4, 0, statement_timestamp())",
        )
        .bind(below.as_uuid())
        .bind(above.as_uuid())
        .bind(hidden)
        .bind(owner.as_uuid())
        .execute(&database.runtime)
        .await
        .expect_err(reason);
        let refused = refused.as_database_error().expect(reason);
        assert_eq!(refused.code().as_deref(), Some("23514"), "{reason}");
    }
    assert_eq!(link(&repositories, other).await, None);
    assert_eq!(link(&repositories, parent).await, None);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 同一个人一小时最多建十间_换一间房建也算_他的_agent_和别人另数() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-rooms-rate-owner").await;
    let member = seed_principal(&database.runtime, "sub-rooms-rate-member").await;
    let agent = seed_agent(&database.runtime, owner).await;
    let parent = create_room(&repositories, owner).await;
    let elsewhere = create_room(&repositories, owner).await;
    admit(&repositories, parent, owner, member).await;

    for index in 0..i64::from(MAXIMUM_SUB_ROOMS_PER_CREATOR_PER_HOUR) {
        let name = format!("Room {index}");
        let sub_room = draft(&repositories, parent, owner, &name, time(HOUR_MS + index)).await;
        assert_eq!(attempt(&repositories, &sub_room).await, None, "{name}");
    }
    let limited = Some(SubRoomCreationBlock::CreatorLimitReached);
    let at = time(HOUR_MS + 10);
    let eleventh = draft(&repositories, parent, owner, "One more", at).await;
    assert_eq!(attempt(&repositories, &eleventh).await, limited);
    let moved = draft(&repositories, elsewhere, owner, "One more", at).await;
    assert_eq!(
        attempt(&repositories, &moved).await,
        limited,
        "换一间房建也算"
    );

    // 他的 Agent 建的另数，别人建的另数。
    let by_agent = NewSubRoom {
        created_by_agent: Some(agent),
        ..draft(&repositories, parent, owner, "By agent", at).await
    };
    assert_eq!(attempt(&repositories, &by_agent).await, None);
    let recorded = link(&repositories, by_agent.snapshot.catalog().id())
        .await
        .expect("有父子关系");
    assert_eq!(recorded.created_by_agent, Some(agent));
    let by_member = draft(&repositories, parent, member, "By member", at).await;
    assert_eq!(attempt(&repositories, &by_member).await, None);

    // 最早那一间满一个小时的那一刻才让出一个位置。
    let early = draft(
        &repositories,
        parent,
        owner,
        "Too early",
        time(2 * HOUR_MS - 1),
    )
    .await;
    assert_eq!(attempt(&repositories, &early).await, limited);
    let on_time = draft(&repositories, parent, owner, "On time", time(2 * HOUR_MS)).await;
    assert_eq!(attempt(&repositories, &on_time).await, None);

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 一间房最多一百间没归档的子房间_归档了的把位置和名字让出来() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-rooms-limit-owner").await;
    let parent = create_room(&repositories, owner).await;

    // 一个小时建一间，不碰限速。
    let limit = i64::from(MAXIMUM_OPEN_SUB_ROOMS);
    let mut first = None;
    for index in 0..limit {
        let name = format!("Room {index}");
        let sub_room = draft(&repositories, parent, owner, &name, time(index * HOUR_MS)).await;
        assert_eq!(create(&repositories, &sub_room).await, None, "{name}");
        first.get_or_insert(sub_room.snapshot.catalog().id());
    }
    let full = Some(SubRoomCreationBlock::RoomLimitReached);
    let at = time(limit * HOUR_MS);
    let extra = draft(&repositories, parent, owner, "One more", at).await;
    assert_eq!(attempt(&repositories, &extra).await, full);
    // 满了的时候叫已经有的名字，照旧先说重名。
    let reused = draft(&repositories, parent, owner, "room 0", at).await;
    assert_eq!(
        attempt(&repositories, &reused).await,
        Some(SubRoomCreationBlock::NameTaken)
    );

    // 归档最早那一间（归档是 2e 的事，这里照它要写的样子直接写表）：位置和名字都让出来。
    let first = first.expect("建过");
    archive(&repositories, first, owner).await;
    sqlx::query(
        r"UPDATE agent_room.private_room_parent
             SET archived_at = to_timestamp($2::double precision / 1000.0),
                 archived_by_principal_id = $3,
                 archived_reason = 'manual'
           WHERE catalog_entry_id = $1",
    )
    .bind(first.as_uuid())
    .bind(time(10).value())
    .bind(owner.as_uuid())
    .execute(&database.runtime)
    .await
    .expect("记下归档");
    assert_eq!(attempt(&repositories, &reused).await, None);
    assert_eq!(attempt(&repositories, &extra).await, full, "又满了");

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 名单按建的先后_带上谁在管_看的人自己在不在_人数_最近说话和归档() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-rooms-list-owner").await;
    let member = seed_principal(&database.runtime, "sub-rooms-list-member").await;
    let bystander = seed_principal(&database.runtime, "sub-rooms-list-bystander").await;
    let parent = create_room(&repositories, owner).await;
    admit(&repositories, parent, owner, member).await;
    admit(&repositories, parent, owner, bystander).await;
    assert!(list(&repositories, parent, owner).await.is_empty());

    let alpha = draft(&repositories, parent, owner, "Alpha", time(3)).await;
    let beta = draft(&repositories, parent, member, "Beta", time(5)).await;
    // 父房间不加密的话子房间也不加密；这里只看存下来的是交给它的那个值。
    let gamma = NewSubRoom {
        end_to_end_encrypted: false,
        ..draft(&repositories, parent, owner, "Gamma", time(7)).await
    };
    for sub_room in [&alpha, &beta, &gamma] {
        assert_eq!(create(&repositories, sub_room).await, None);
    }
    let id = |sub_room: &NewSubRoom| sub_room.snapshot.catalog().id();
    assert!(encrypted(&database.runtime, id(&alpha)).await);
    assert!(!encrypted(&database.runtime, id(&gamma)).await);
    archive(&repositories, id(&gamma), owner).await;
    sqlx::query(
        r"UPDATE agent_room.private_room_parent
             SET last_message_at = to_timestamp($2::double precision / 1000.0)
           WHERE catalog_entry_id = $1",
    )
    .bind(id(&beta).as_uuid())
    .bind(time(9_000).value())
    .execute(&database.runtime)
    .await
    .expect("写入最近说话");

    // 成员看：只在自己建的那一间里。
    let expected = [
        entry(&alpha),
        SubRoomEntry {
            viewer_status: Some(PrivateRoomMembershipStatus::Joined),
            member_count: 2,
            last_message_at: Some(time(9_000)),
            ..entry(&beta)
        },
        SubRoomEntry {
            archived: true,
            ..entry(&gamma)
        },
    ];
    assert_eq!(list(&repositories, parent, member).await, expected);

    // 房主每一间都在；只在父房间里的人哪一间都不在。
    let joined = Some(PrivateRoomMembershipStatus::Joined);
    let owner_sees = list(&repositories, parent, owner).await;
    assert!(owner_sees.iter().all(|seen| seen.viewer_status == joined));
    let bystander_sees = list(&repositories, parent, bystander).await;
    assert!(
        bystander_sees
            .iter()
            .all(|seen| seen.viewer_status.is_none())
    );
    assert_eq!(bystander_sees.len(), 3);

    // 子房间下面没有子房间。
    assert!(list(&repositories, id(&alpha), owner).await.is_empty());

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 三项设置没定过就是默认值_按版本存_别人刚改过就不写() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-rooms-policy-owner").await;
    let parent = create_room(&repositories, owner).await;
    let nowhere = RoomCatalogId::from_uuid(Uuid::now_v7());
    let facts = |policy| {
        Some(SubRoomParentFacts {
            end_to_end_encrypted: true,
            policy,
        })
    };

    assert_eq!(policy(&repositories, parent).await, None);
    assert_eq!(
        parent_facts(&repositories, parent).await,
        facts(SubRoomPolicy::DEFAULT)
    );
    assert_eq!(parent_facts(&repositories, nowhere).await, None);

    let strict = SubRoomPolicy {
        creators: SubRoomCreators::Admins,
        agents_may_create: false,
        auto_archive: false,
    };
    let first = SubRoomPolicyRecord {
        catalog_id: parent,
        policy: strict,
        updated_by: owner,
        updated_at: time(5),
        version: version(1),
    };
    assert!(save_policy(&repositories, &first, None).await);
    assert_eq!(policy(&repositories, parent).await, Some(first));
    assert_eq!(parent_facts(&repositories, parent).await, facts(strict));

    // 两头都以为还没有记录：后到的不写。
    let late = SubRoomPolicyRecord {
        policy: SubRoomPolicy::DEFAULT,
        updated_at: time(6),
        ..first
    };
    assert!(!save_policy(&repositories, &late, None).await);
    assert_eq!(policy(&repositories, parent).await, Some(first));

    // 照读到的版本改；拿旧版本再改的不写。
    let second = SubRoomPolicyRecord {
        policy: SubRoomPolicy {
            auto_archive: true,
            ..strict
        },
        updated_at: time(7),
        version: version(2),
        ..first
    };
    assert!(save_policy(&repositories, &second, Some(version(1))).await);
    assert_eq!(policy(&repositories, parent).await, Some(second));
    let stale = SubRoomPolicyRecord {
        updated_at: time(8),
        ..late
    };
    assert!(!save_policy(&repositories, &stale, Some(version(1))).await);
    assert_eq!(policy(&repositories, parent).await, Some(second));

    // 没有这间房：外键拦着。
    let orphan = SubRoomPolicyRecord {
        catalog_id: nowhere,
        ..first
    };
    let refused = SubRoomStore::save_sub_room_policy(&repositories, &orphan, None)
        .await
        .expect_err("没有这间房");
    assert_eq!(refused.kind(), RepositoryErrorKind::Constraint);

    database.close().await;
}

/// 照用例的做法备好一间子房间：房主和建的人在里面，建的人不是房主就自动是这一间的子房间管理员。
async fn draft(
    repositories: &PostgresRepositories,
    parent: RoomCatalogId,
    creator: PrincipalId,
    name: &str,
    at: UtcMillis,
) -> NewSubRoom {
    let above = load(repositories, parent).await;
    let owner = above.room().owner_principal_id();
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    let room = new_sub_room(catalog_id, above.room(), creator).expect("建的人在父房间里");
    NewSubRoom {
        snapshot: snapshot_named(catalog_id, owner, room, name).under(parent),
        parent_catalog_id: parent,
        parent_version: above.room().version(),
        name_key: sub_room_name_key(name),
        access: RoomJoinMode::Open,
        hidden: false,
        created_by: creator,
        created_by_agent: None,
        managers: if creator == owner {
            Vec::new()
        } else {
            vec![creator]
        },
        end_to_end_encrypted: true,
        created_at: at,
    }
}

/// 刚建好、没人动过的子房间在名单里的样子：看的人不在里面，里面只有房主。
fn entry(sub_room: &NewSubRoom) -> SubRoomEntry {
    SubRoomEntry {
        catalog_id: sub_room.snapshot.catalog().id(),
        name: sub_room.snapshot.catalog().name().to_owned(),
        description: sub_room.snapshot.catalog().description().to_owned(),
        access: sub_room.access,
        hidden: sub_room.hidden,
        archived: false,
        created_by: sub_room.created_by,
        created_at: sub_room.created_at,
        managers: sub_room.managers.clone(),
        viewer_status: None,
        member_count: 1,
        agent_count: 0,
        last_message_at: None,
    }
}

/// 先问、再存：两头查的是同一段，说的要一样。
async fn attempt(
    repositories: &PostgresRepositories,
    sub_room: &NewSubRoom,
) -> Option<SubRoomCreationBlock> {
    let asked = blocked(repositories, sub_room).await;
    let stored = create(repositories, sub_room).await;
    assert_eq!(asked, stored, "先问的和存的时候说的不一样");
    stored
}

async fn blocked(
    repositories: &PostgresRepositories,
    sub_room: &NewSubRoom,
) -> Option<SubRoomCreationBlock> {
    SubRoomStore::sub_room_creation_block(repositories, &sub_room.check())
        .await
        .expect("可查")
}

async fn create(
    repositories: &PostgresRepositories,
    sub_room: &NewSubRoom,
) -> Option<SubRoomCreationBlock> {
    SubRoomStore::create_sub_room(repositories, sub_room)
        .await
        .expect("写入不报错")
}

/// 请一个人进房间，他接受。
async fn admit(
    repositories: &PostgresRepositories,
    catalog: RoomCatalogId,
    owner: PrincipalId,
    member: PrincipalId,
) {
    let before = load(repositories, catalog).await;
    let mut room = before.room().clone();
    room.invite(owner, member, ordinary_permissions())
        .expect("房主能邀请");
    PrivateRoomStore::save(repositories, &room, before.room().version(), time(1))
        .await
        .expect("邀请存得进去");
    let expected = room.version();
    room.accept_invitation(member).expect("受邀的人能进来");
    PrivateRoomStore::save(repositories, &room, expected, time(2))
        .await
        .expect("加入存得进去");
}

async fn load(repositories: &PostgresRepositories, catalog: RoomCatalogId) -> PrivateRoomSnapshot {
    PrivateRoomStore::find_by_catalog(repositories, catalog)
        .await
        .expect("可读")
        .expect("有房间")
}

async fn link(repositories: &PostgresRepositories, catalog: RoomCatalogId) -> Option<SubRoomLink> {
    SubRoomStore::sub_room_link(repositories, catalog)
        .await
        .expect("可读")
}

async fn list(
    repositories: &PostgresRepositories,
    parent: RoomCatalogId,
    viewer: PrincipalId,
) -> Vec<SubRoomEntry> {
    SubRoomStore::list_sub_rooms(repositories, parent, viewer)
        .await
        .expect("可读")
}

async fn policy(
    repositories: &PostgresRepositories,
    parent: RoomCatalogId,
) -> Option<SubRoomPolicyRecord> {
    SubRoomStore::sub_room_policy(repositories, parent)
        .await
        .expect("可读")
}

async fn parent_facts(
    repositories: &PostgresRepositories,
    parent: RoomCatalogId,
) -> Option<SubRoomParentFacts> {
    SubRoomStore::sub_room_parent_facts(repositories, parent)
        .await
        .expect("可读")
}

async fn save_policy(
    repositories: &PostgresRepositories,
    record: &SubRoomPolicyRecord,
    expected: Option<AggregateVersion>,
) -> bool {
    SubRoomStore::save_sub_room_policy(repositories, record, expected)
        .await
        .expect("写入不报错")
}

async fn encrypted(pool: &PgPool, catalog: RoomCatalogId) -> bool {
    sqlx::query_scalar(
        "SELECT end_to_end_encrypted FROM agent_room.room_catalog_entry WHERE id = $1",
    )
    .bind(catalog.as_uuid())
    .fetch_one(pool)
    .await
    .expect("目录可读")
}
