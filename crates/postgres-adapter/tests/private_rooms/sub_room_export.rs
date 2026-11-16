//! 下载我的数据里的子房间（`specs/redesign/sub-rooms.md` 第 2l 步）：自己建的、管的、归档的子房间各一行，
//! 写着是哪一样；自己建的分类带名字；和自己无关的不在里面。

use agent_room_application::ports::{
    AccountDeletionRepository, NewSubRoomCategory, SubRoomArchiveOutcome, SubRoomCategoryName,
    SubRoomCategoryOutcome, SubRoomCategoryStore,
};
use agent_room_domain::{ids::SubRoomCategoryId, sub_room_categories::sub_room_category_name_key};
use serde_json::{Value, json};

use super::{archive::archive_now, follow::id, *};

async fn exported(repositories: &PostgresRepositories, principal: PrincipalId) -> Value {
    AccountDeletionRepository::export(repositories, principal, time(90))
        .await
        .expect("导出不报错")
        .expect("账户在用")
        .data
}

/// 每一行里不看时间的那几项：时间的写法跟着数据库，对不上也不算错。
fn sub_rooms(data: &Value) -> Vec<Value> {
    data["subRooms"]
        .as_array()
        .expect("子房间是一列")
        .iter()
        .map(|row| {
            json!({
                "roomId": row["roomId"],
                "parentRoomId": row["parentRoomId"],
                "access": row["access"],
                "createdByMe": row["createdByMe"],
                "managedByMe": row["managedByMe"],
                "archivedByMe": row["archivedByMe"],
                "archived": !row["archivedAt"].is_null(),
                "archivedReason": row["archivedReason"],
            })
        })
        .collect()
}

fn uuid_text(catalog: RoomCatalogId) -> String {
    catalog.as_uuid().to_string()
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 下载的数据里有自己建的_管的_归档的子房间和建的分类_别人的不在() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-room-export-owner").await;
    let member = seed_principal(&database.runtime, "sub-room-export-member").await;
    let bystander = seed_principal(&database.runtime, "sub-room-export-bystander").await;
    let parent = create_room(&repositories, owner).await;
    admit(&repositories, parent, owner, member).await;
    admit(&repositories, parent, owner, bystander).await;
    // 成员建的那间他自动在管；房主建的那间由房主归档。
    let theirs = draft(&repositories, parent, member, "Specs", time(5)).await;
    let mine = draft(&repositories, parent, owner, "Releases", time(6)).await;
    for sub_room in [&theirs, &mine] {
        assert_eq!(create(&repositories, sub_room).await, None);
    }
    assert_eq!(
        archive_now(&repositories, parent, id(&mine), owner, time(10)).await,
        SubRoomArchiveOutcome::Done
    );
    let category = SubRoomCategoryId::from_uuid(Uuid::now_v7());
    let saved = SubRoomCategoryStore::create_sub_room_category(
        &repositories,
        &NewSubRoomCategory {
            id: category,
            parent_catalog_id: parent,
            name: SubRoomCategoryName {
                name: "Ops".to_owned(),
                name_key: sub_room_category_name_key("Ops"),
            },
            created_by: member,
            created_at: time(7),
        },
    )
    .await
    .expect("写入不报错");
    assert!(matches!(saved, SubRoomCategoryOutcome::Saved(_)));

    let row = |sub_room: &NewSubRoom, created, managed, archived_by_me, reason: Value| {
        json!({
            "roomId": uuid_text(id(sub_room)),
            "parentRoomId": uuid_text(parent),
            "access": "open",
            "createdByMe": created,
            "managedByMe": managed,
            "archivedByMe": archived_by_me,
            "archived": !reason.is_null(),
            "archivedReason": reason,
        })
    };
    let member_data = exported(&repositories, member).await;
    assert_eq!(
        sub_rooms(&member_data),
        [row(&theirs, true, true, false, Value::Null)]
    );
    let categories = member_data["subRoomCategories"]
        .as_array()
        .expect("分类是一列");
    assert_eq!(categories.len(), 1);
    assert_eq!(categories[0]["id"], json!(category.as_uuid().to_string()));
    assert_eq!(categories[0]["roomId"], json!(uuid_text(parent)));
    assert_eq!(categories[0]["name"], "Ops");

    // 房主管着每一间，可那是房主的身份，不是子房间管理员的记录：只列他建的、他归档的。
    let owner_data = exported(&repositories, owner).await;
    assert_eq!(
        sub_rooms(&owner_data),
        [row(&mine, true, false, true, json!("manual"))]
    );
    assert_eq!(owner_data["subRoomCategories"], json!([]));

    let bystander_data = exported(&repositories, bystander).await;
    assert_eq!(bystander_data["subRooms"], json!([]));
    assert_eq!(bystander_data["subRoomCategories"], json!([]));

    database.close().await;
}
