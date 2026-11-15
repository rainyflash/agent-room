//! 子房间分类的存储（`specs/redesign/sub-rooms.md` 第 2e-1 步）：建的排在最后、重发不多建、重名不分
//! 大小写、一间房最多 20 个；改名挪位置以后先后从 0 起连着；删掉一个，里面的子房间回到“未分类”；
//! 建和删写审计，只记号不记名字；父房间归档了、或者它是一间子房间时管不了。建子房间时放进分类、
//! 之后挪（2e-1b），只认这间房自己的分类。

use agent_room_application::ports::{
    NewSubRoomCategory, SubRoomCategory, SubRoomCategoryName, SubRoomCategoryOutcome,
    SubRoomCategoryRemoval, SubRoomCategoryRevision, SubRoomCategoryStore, SubRoomRevision,
    SubRoomRevisionBlock,
};
use agent_room_domain::{
    ids::SubRoomCategoryId,
    sub_room_categories::{MAXIMUM_SUB_ROOM_CATEGORIES, sub_room_category_name_key},
};

use serde_json::{Value, json};

use super::{follow::id, *};

fn name(value: &str) -> SubRoomCategoryName {
    SubRoomCategoryName {
        name: value.to_owned(),
        name_key: sub_room_category_name_key(value),
    }
}

fn new_category(
    parent: RoomCatalogId,
    owner: PrincipalId,
    category: SubRoomCategoryId,
    value: &str,
) -> NewSubRoomCategory {
    NewSubRoomCategory {
        id: category,
        parent_catalog_id: parent,
        name: name(value),
        created_by: owner,
        created_at: time(20),
    }
}

fn fresh_id() -> SubRoomCategoryId {
    SubRoomCategoryId::from_uuid(Uuid::now_v7())
}

async fn create_category(
    repositories: &PostgresRepositories,
    category: &NewSubRoomCategory,
) -> SubRoomCategoryOutcome {
    SubRoomCategoryStore::create_sub_room_category(repositories, category)
        .await
        .expect("写入不报错")
}

async fn revise_category(
    repositories: &PostgresRepositories,
    revision: &SubRoomCategoryRevision,
) -> SubRoomCategoryOutcome {
    SubRoomCategoryStore::revise_sub_room_category(repositories, revision)
        .await
        .expect("写入不报错")
}

async fn delete_category(
    repositories: &PostgresRepositories,
    removal: &SubRoomCategoryRemoval,
) -> SubRoomCategoryOutcome {
    SubRoomCategoryStore::delete_sub_room_category(repositories, removal)
        .await
        .expect("写入不报错")
}

fn saved(outcome: SubRoomCategoryOutcome) -> Vec<(String, u8)> {
    match outcome {
        SubRoomCategoryOutcome::Saved(categories) => categories
            .into_iter()
            .map(|category: SubRoomCategory| (category.name, category.position))
            .collect(),
        other => panic!("应该存好了，结果是 {other:?}"),
    }
}

fn named(pairs: &[(&str, u8)]) -> Vec<(String, u8)> {
    pairs
        .iter()
        .map(|(name, position)| ((*name).to_owned(), *position))
        .collect()
}

/// 这个分类的审计：动作和房间号。
async fn audits(pool: &PgPool, category: SubRoomCategoryId) -> Vec<(String, Value)> {
    sqlx::query_as(
        r"SELECT action, metadata
            FROM agent_room.audit_event
           WHERE target_kind = 'sub_room_category' AND target_reference = $1
           ORDER BY occurred_at, id",
    )
    .bind(category.to_string())
    .fetch_all(pool)
    .await
    .expect("读审计")
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 建的排在最后_重发不多建_撞号和重名各有说法_一间房最多二十个_审计只记号() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-room-categories-owner").await;
    let parent = create_room(&repositories, owner).await;
    let elsewhere = create_room(&repositories, owner).await;

    let design = new_category(parent, owner, fresh_id(), "设计");
    let release = new_category(parent, owner, fresh_id(), "Release");
    saved(create_category(&repositories, &design).await);
    assert_eq!(
        saved(create_category(&repositories, &release).await),
        named(&[("设计", 0), ("Release", 1)])
    );
    // 同一个请求重发：什么都不写，回现在的样子。
    assert_eq!(
        saved(create_category(&repositories, &release).await),
        named(&[("设计", 0), ("Release", 1)])
    );
    assert_eq!(
        audits(&database.runtime, release.id).await.len(),
        1,
        "重发不多记"
    );
    let (action, metadata) = &audits(&database.runtime, design.id).await[0];
    assert_eq!(action, "sub_room.category.created");
    assert_eq!(
        metadata,
        &json!({ "roomCatalogId": parent.to_string() }),
        "审计里没有名字"
    );

    for taken in [
        new_category(parent, owner, release.id, "测试"),
        new_category(elsewhere, owner, release.id, "Release"),
    ] {
        assert_eq!(
            create_category(&repositories, &taken).await,
            SubRoomCategoryOutcome::IdTaken
        );
    }
    assert_eq!(
        create_category(
            &repositories,
            &new_category(parent, owner, fresh_id(), "release")
        )
        .await,
        SubRoomCategoryOutcome::NameTaken
    );
    // 别的房间里同名不算。
    saved(
        create_category(
            &repositories,
            &new_category(elsewhere, owner, fresh_id(), "Release"),
        )
        .await,
    );

    for number in 2..MAXIMUM_SUB_ROOM_CATEGORIES {
        let category = new_category(parent, owner, fresh_id(), &format!("分类 {number}"));
        saved(create_category(&repositories, &category).await);
    }
    assert_eq!(
        create_category(
            &repositories,
            &new_category(parent, owner, fresh_id(), "第二十一个")
        )
        .await,
        SubRoomCategoryOutcome::LimitReached
    );
    let all = SubRoomCategoryStore::sub_room_categories(&repositories, parent)
        .await
        .expect("可读");
    assert_eq!(all.len(), usize::from(MAXIMUM_SUB_ROOM_CATEGORIES));
    assert!(
        all.iter()
            .enumerate()
            .all(|(index, category)| usize::from(category.position) == index),
        "先后从 0 起连着"
    );
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 改名挪位置_别的跟着让_超过最后的放到最后() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-room-categories-arrange").await;
    let parent = create_room(&repositories, owner).await;
    let ids = three_categories(&repositories, parent, owner).await;
    let revision = |category, value, position| revision(parent, category, value, position);
    assert_eq!(
        saved(revise_category(&repositories, &revision(ids[2], None, Some(0))).await),
        named(&[("测试", 0), ("设计", 1), ("Release", 2)])
    );
    assert_eq!(
        saved(revise_category(&repositories, &revision(ids[0], Some("Design"), Some(200))).await),
        named(&[("测试", 0), ("Release", 1), ("Design", 2)]),
        "超过最后的放到最后"
    );
    assert_eq!(
        saved(revise_category(&repositories, &revision(ids[0], Some("design"), None)).await),
        named(&[("测试", 0), ("Release", 1), ("design", 2)]),
        "改成只差大小写的自己不算重名"
    );
    assert_eq!(
        revise_category(&repositories, &revision(ids[0], Some("RELEASE"), None)).await,
        SubRoomCategoryOutcome::NameTaken
    );
    assert_eq!(
        revise_category(&repositories, &revision(fresh_id(), Some("别的"), None)).await,
        SubRoomCategoryOutcome::Missing
    );
}

/// 设计、Release、测试三个分类，回它们的号。
async fn three_categories(
    repositories: &PostgresRepositories,
    parent: RoomCatalogId,
    owner: PrincipalId,
) -> Vec<SubRoomCategoryId> {
    let ids: Vec<SubRoomCategoryId> = (0..3).map(|_| fresh_id()).collect();
    for (category, value) in ids.iter().zip(["设计", "Release", "测试"]) {
        saved(create_category(repositories, &new_category(parent, owner, *category, value)).await);
    }
    ids
}

fn revision(
    parent: RoomCatalogId,
    category: SubRoomCategoryId,
    value: Option<&str>,
    position: Option<u8>,
) -> SubRoomCategoryRevision {
    SubRoomCategoryRevision {
        id: category,
        parent_catalog_id: parent,
        name: value.map(name),
        position,
        at: time(30),
    }
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 删掉以后子房间回到未分类_后面的往前挪_归档的和子房间管不了() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-room-categories-delete").await;
    let parent = create_room(&repositories, owner).await;
    let ids = three_categories(&repositories, parent, owner).await;

    // 一间子房间放在“Release”里（建子房间时放进分类是 2e-1b 的事，这里直接写表）。
    let sub_room = draft(&repositories, parent, owner, "Ship", time(40)).await;
    assert_eq!(create(&repositories, &sub_room).await, None);
    sqlx::query(
        "UPDATE agent_room.private_room_parent SET category_id = $2 WHERE catalog_entry_id = $1",
    )
    .bind(id(&sub_room).as_uuid())
    .bind(ids[1].as_uuid())
    .execute(&database.runtime)
    .await
    .expect("放进分类");

    let removal = SubRoomCategoryRemoval {
        id: ids[1],
        parent_catalog_id: parent,
        removed_by: owner,
        at: time(50),
    };
    assert_eq!(
        saved(delete_category(&repositories, &removal).await),
        named(&[("设计", 0), ("测试", 1)]),
        "后面的往前挪"
    );
    let category: Option<Uuid> = sqlx::query_scalar(
        "SELECT category_id FROM agent_room.private_room_parent WHERE catalog_entry_id = $1",
    )
    .bind(id(&sub_room).as_uuid())
    .fetch_one(&database.runtime)
    .await
    .expect("读子房间");
    assert_eq!(category, None, "里面的子房间回到未分类");
    assert_eq!(
        saved(delete_category(&repositories, &removal).await),
        named(&[("设计", 0), ("测试", 1)]),
        "删过的再删也算成功"
    );
    assert_eq!(
        audits(&database.runtime, ids[1])
            .await
            .into_iter()
            .map(|(action, _)| action)
            .collect::<Vec<_>>(),
        ["sub_room.category.created", "sub_room.category.deleted"],
        "再删一次不多记"
    );

    // 子房间下面没有分类。
    assert_eq!(
        create_category(
            &repositories,
            &new_category(id(&sub_room), owner, fresh_id(), "设计")
        )
        .await,
        SubRoomCategoryOutcome::ParentUnavailable
    );
    archive(&repositories, parent, owner).await;
    assert_eq!(
        create_category(
            &repositories,
            &new_category(parent, owner, fresh_id(), "新的")
        )
        .await,
        SubRoomCategoryOutcome::ParentUnavailable
    );
    assert_eq!(
        revise_category(
            &repositories,
            &revision(parent, ids[0], Some("Design"), None)
        )
        .await,
        SubRoomCategoryOutcome::ParentUnavailable
    );
}

/// 只挪分类：名字和用途照现在的带上。
async fn placement(
    repositories: &PostgresRepositories,
    parent: RoomCatalogId,
    sub_room: RoomCatalogId,
    category: Option<SubRoomCategoryId>,
) -> Option<SubRoomRevisionBlock> {
    let current = load(repositories, sub_room).await;
    let name = current.catalog().name().to_owned();
    SubRoomStore::revise_sub_room(
        repositories,
        &SubRoomRevision {
            catalog_id: sub_room,
            parent_catalog_id: parent,
            name_key: sub_room_name_key(&name),
            name,
            description: current.catalog().description().to_owned(),
            door: None,
            category: Some(category),
            at: time(60),
        },
    )
    .await
    .expect("写入不报错")
}

async fn category_in_list(
    repositories: &PostgresRepositories,
    parent: RoomCatalogId,
    owner: PrincipalId,
    sub_room: RoomCatalogId,
) -> Option<SubRoomCategoryId> {
    list(repositories, parent, owner)
        .await
        .into_iter()
        .find(|entry| entry.catalog_id == sub_room)
        .expect("名单里有这一间")
        .category_id
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 建子房间时放进分类_能挪_能挪回未分类_别的房间的分类不收() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-room-categories-placement").await;
    let parent = create_room(&repositories, owner).await;
    let elsewhere = create_room(&repositories, owner).await;
    let ids = three_categories(&repositories, parent, owner).await;
    let foreign = fresh_id();
    saved(
        create_category(
            &repositories,
            &new_category(elsewhere, owner, foreign, "别处"),
        )
        .await,
    );

    let ship = NewSubRoom {
        category_id: Some(ids[1]),
        ..draft(&repositories, parent, owner, "Ship", time(40)).await
    };
    assert_eq!(attempt(&repositories, &ship).await, None);
    assert_eq!(
        category_in_list(&repositories, parent, owner, id(&ship)).await,
        Some(ids[1])
    );
    let stray = NewSubRoom {
        category_id: Some(foreign),
        ..draft(&repositories, parent, owner, "Stray", time(41)).await
    };
    assert_eq!(
        attempt(&repositories, &stray).await,
        Some(SubRoomCreationBlock::CategoryMissing),
        "先问和存的时候都不收别的房间的分类"
    );

    let missing = Some(SubRoomRevisionBlock::CategoryMissing);
    assert_eq!(
        placement(&repositories, parent, id(&ship), Some(foreign)).await,
        missing
    );
    assert_eq!(
        placement(&repositories, parent, id(&ship), Some(fresh_id())).await,
        missing
    );
    assert_eq!(
        placement(&repositories, parent, id(&ship), Some(ids[0])).await,
        None
    );
    assert_eq!(
        category_in_list(&repositories, parent, owner, id(&ship)).await,
        Some(ids[0])
    );
    assert_eq!(
        placement(&repositories, parent, id(&ship), None).await,
        None
    );
    assert_eq!(
        category_in_list(&repositories, parent, owner, id(&ship)).await,
        None,
        "挪回未分类"
    );
}
