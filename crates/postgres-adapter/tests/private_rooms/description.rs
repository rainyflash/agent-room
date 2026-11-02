//! 房主改房间简介（`specs/public-rooms/design.md`）：存进房间目录，读回来是新的；归档了的房间不改。

use agent_room_application::{
    persistence::RepositoryErrorKind, ports::PrivateRoomDescriptionStore,
};

use super::{
    listing::{archive, create_room},
    *,
};

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 简介存进目录_读回来是新的_归档了的不改() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "description-owner").await;
    let room = create_room(&repositories, owner).await;

    for description in ["每周读一本书\n周五晚上聊", ""] {
        PrivateRoomDescriptionStore::describe(&repositories, room, description, time(30))
            .await
            .expect("存上");
        let snapshot = PrivateRoomStore::find_by_catalog(&repositories, room)
            .await
            .expect("可读")
            .expect("有房间");
        assert_eq!(snapshot.catalog().description(), description);
    }

    archive(&repositories, room, owner).await;
    let failure = PrivateRoomDescriptionStore::describe(&repositories, room, "归档以后", time(40))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), RepositoryErrorKind::Conflict);
    database.close().await;
}
