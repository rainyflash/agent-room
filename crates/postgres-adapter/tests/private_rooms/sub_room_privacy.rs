//! 子房间跟着父房间公开、改回私密的存储（`specs/redesign/sub-rooms.md` 第 2d-5 步）：读一间房下面
//! 每一间子房间在 Matrix 上是哪间房、有没有门、写没写过那一刻、加没加密，归档了的也在；记下那一刻
//! 只认第一次。

use agent_room_application::ports::{MatrixRoomId, PrivateRoomListingStore, SubRoomPrivacy};

use super::{follow::id, *};

fn matrix_room(snapshot: &PrivateRoomSnapshot) -> MatrixRoomId {
    MatrixRoomId::new(snapshot.instance().matrix_room_id().as_str().to_owned())
        .expect("Matrix 房间标识有效")
}

async fn privacy(
    repositories: &PostgresRepositories,
    parent: RoomCatalogId,
) -> Vec<SubRoomPrivacy> {
    PrivateRoomListingStore::sub_room_privacy(repositories, parent)
        .await
        .expect("可读")
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 读得出每一间子房间的门_那一刻和加密_归档了的也在_别的房间的不在() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-room-privacy-owner").await;
    let parent = create_room(&repositories, owner).await;
    let elsewhere = create_room(&repositories, owner).await;
    let plain = NewSubRoom {
        end_to_end_encrypted: false,
        ..draft(&repositories, parent, owner, "Plain", time(5)).await
    };
    let guarded = NewSubRoom {
        access: RoomJoinMode::Request,
        ..draft(&repositories, parent, owner, "Guarded", time(6)).await
    };
    let shelved = draft(&repositories, parent, owner, "Shelved", time(7)).await;
    let other = draft(&repositories, elsewhere, owner, "Other", time(8)).await;
    for sub_room in [&plain, &guarded, &shelved, &other] {
        assert_eq!(create(&repositories, sub_room).await, None);
    }
    // 归档是 2e 的事，这里照它要写的样子直接写表。
    archive(&repositories, id(&shelved), owner).await;
    sqlx::query(
        r"UPDATE agent_room.private_room_parent
             SET archived_at = to_timestamp($2::double precision / 1000.0),
                 archived_by_principal_id = $3,
                 archived_reason = 'manual'
           WHERE catalog_entry_id = $1",
    )
    .bind(id(&shelved).as_uuid())
    .bind(time(10).value())
    .bind(owner.as_uuid())
    .execute(&database.runtime)
    .await
    .expect("记下归档");

    let expected = |sub_room: &NewSubRoom, has_door, encrypted| SubRoomPrivacy {
        catalog_id: id(sub_room),
        matrix_room: matrix_room(&sub_room.snapshot),
        has_door,
        listing_marked: false,
        end_to_end_encrypted: encrypted,
    };
    assert_eq!(
        privacy(&repositories, parent).await,
        [
            expected(&plain, false, false),
            expected(&guarded, true, true),
            expected(&shelved, false, true),
        ],
        "按建的先后；归档了的也在"
    );
    assert_eq!(
        privacy(&repositories, elsewhere).await,
        [expected(&other, false, true)]
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 记下那一刻只认第一次_拆门以后读得出来_开了加密读得出来() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "sub-room-privacy-mark-owner").await;
    let parent = create_room(&repositories, owner).await;
    let plain = NewSubRoom {
        end_to_end_encrypted: false,
        ..draft(&repositories, parent, owner, "Plain", time(5)).await
    };
    assert_eq!(create(&repositories, &plain).await, None);
    let plain = id(&plain);

    PrivateRoomListingStore::mark_sub_room_listed(&repositories, plain, time(50))
        .await
        .expect("记得下");
    PrivateRoomListingStore::mark_sub_room_listed(&repositories, plain, time(60))
        .await
        .expect("再记不报错");
    assert_eq!(
        link(&repositories, plain)
            .await
            .expect("有父子关系")
            .door_opened_at,
        Some(time(50)),
        "只认第一次：往后挪的话，那一刻以后进来的人会变成“以前就在”"
    );

    PrivateRoomListingStore::mark_room_encrypted(&repositories, plain)
        .await
        .expect("改得了");
    let read = privacy(&repositories, parent).await;
    assert_eq!(
        read.iter()
            .map(|sub_room| (sub_room.listing_marked, sub_room.end_to_end_encrypted))
            .collect::<Vec<_>>(),
        [(true, true)]
    );

    database.close().await;
}
