//! 公开房间的设置（`specs/public-rooms/design.md`）：往返、乐观锁、每个账户 1 间、分享码不重复、
//! 改回私密时打开加密标记。

use agent_room_application::ports::{
    MatrixRoomId, PrivateRoomListingRecord, PrivateRoomListingSaveOutcome, PrivateRoomListingStore,
    RoomContentPolicyLookup,
};
use agent_room_domain::{
    room_listing::{RoomJoinMode, RoomListingSettings, RoomShareCode},
    version::AggregateVersion,
};

use super::*;

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 公开设置往返_改回私密分享码留着_版本对不上不覆盖() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "listing-owner").await;
    let room = create_room(&repositories, owner).await;
    assert_eq!(
        PrivateRoomListingStore::listing(&repositories, room)
            .await
            .expect("可读"),
        None,
        "没公开过的房间没有记录"
    );

    let listed = record(room, owner, true, "Lst001", 1);
    assert_eq!(save(&repositories, &listed, None, owner).await, Saved);
    assert_eq!(
        PrivateRoomListingStore::listing(&repositories, room)
            .await
            .expect("可读"),
        Some(listed.clone())
    );
    assert_eq!(
        PrivateRoomListingStore::listed_rooms_owned_by(&repositories, owner)
            .await
            .expect("可读"),
        [room]
    );

    let unlisted = PrivateRoomListingRecord {
        settings: settings(false, RoomJoinMode::Request),
        updated_at: time(5_000),
        version: version(2),
        ..listed.clone()
    };
    assert_eq!(
        save(&repositories, &unlisted, Some(version(1)), owner).await,
        Saved
    );
    let stale = PrivateRoomListingRecord {
        version: version(2),
        ..listed.clone()
    };
    assert_eq!(
        save(&repositories, &stale, Some(version(1)), owner).await,
        PrivateRoomListingSaveOutcome::Stale
    );
    assert_eq!(
        save(&repositories, &listed, None, owner).await,
        PrivateRoomListingSaveOutcome::Stale,
        "已经有记录的房间再当成第一次公开"
    );
    let stored = PrivateRoomListingStore::listing(&repositories, room)
        .await
        .expect("可读")
        .expect("有记录");
    assert_eq!(stored, unlisted);
    assert_eq!(stored.share_code, listed.share_code);
    assert!(
        PrivateRoomListingStore::listed_rooms_owned_by(&repositories, owner)
            .await
            .expect("可读")
            .is_empty()
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 每个账户最多一间公开房间_改回私密和归档的不算() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "listing-limit").await;
    let first = create_room(&repositories, owner).await;
    let second = create_room(&repositories, owner).await;

    let first_listed = record(first, owner, true, "Lim001", 1);
    assert_eq!(save(&repositories, &first_listed, None, owner).await, Saved);
    assert_eq!(
        save(
            &repositories,
            &record(second, owner, true, "Lim002", 1),
            None,
            owner
        )
        .await,
        PrivateRoomListingSaveOutcome::OwnerLimitReached
    );
    // 公开着的那间自己改设置不算第二间。
    let first_changed = PrivateRoomListingRecord {
        settings: settings(true, RoomJoinMode::InviteOnly),
        version: version(2),
        ..first_listed.clone()
    };
    assert_eq!(
        save(&repositories, &first_changed, Some(version(1)), owner).await,
        Saved
    );
    // 私密的房间记设置不受限额。
    assert_eq!(
        save(
            &repositories,
            &record(second, owner, false, "Lim002", 1),
            None,
            owner
        )
        .await,
        Saved
    );

    archive(&repositories, first, owner).await;
    assert_eq!(
        save(
            &repositories,
            &record(second, owner, true, "Lim002", 2),
            Some(version(1)),
            owner
        )
        .await,
        Saved,
        "归档的房间不再占名额"
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 分享码被别的房间占了就说出来_不改那间房() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let first_owner = seed_principal(&database.runtime, "listing-code-a").await;
    let second_owner = seed_principal(&database.runtime, "listing-code-b").await;
    let first = create_room(&repositories, first_owner).await;
    let second = create_room(&repositories, second_owner).await;

    let taken = record(first, first_owner, true, "Dup001", 1);
    assert_eq!(save(&repositories, &taken, None, first_owner).await, Saved);
    assert_eq!(
        save(
            &repositories,
            &record(second, second_owner, true, "Dup001", 1),
            None,
            second_owner
        )
        .await,
        PrivateRoomListingSaveOutcome::ShareCodeTaken
    );
    assert_eq!(
        save(
            &repositories,
            &record(second, second_owner, true, "dup001", 1),
            None,
            second_owner
        )
        .await,
        Saved,
        "大小写不同是两个码"
    );
    assert_eq!(
        PrivateRoomListingStore::listing(&repositories, first)
            .await
            .expect("可读"),
        Some(taken)
    );

    database.close().await;
}

#[tokio::test]
#[ignore = "需要由 tools/database.py 提供隔离的真实 PostgreSQL"]
async fn 打开加密以后目录上的标记是加密的() {
    let database = TestDatabase::connect().await;
    let repositories = PostgresRepositories::new(database.runtime.clone());
    let owner = seed_principal(&database.runtime, "listing-encryption").await;
    let snapshot = snapshot(RoomCatalogId::from_uuid(Uuid::now_v7()), owner);
    PrivateRoomStore::create(&repositories, &snapshot, time(0))
        .await
        .expect("建房成功");
    // 新建时选了公开的房间不加密（第 3 步建房那一步写的）。
    sqlx::query(
        "UPDATE agent_room.room_catalog_entry SET end_to_end_encrypted = false WHERE id = $1",
    )
    .bind(snapshot.catalog().id().as_uuid())
    .execute(&database.migration)
    .await
    .expect("改成不加密");
    let matrix_room =
        MatrixRoomId::new(snapshot.instance().matrix_room_id().as_str().to_owned()).unwrap();
    let encrypted = || async {
        RoomContentPolicyLookup::content_policy(&repositories, &matrix_room)
            .await
            .expect("可读")
            .expect("是我们的房间")
            .end_to_end_encrypted
    };
    assert!(!encrypted().await);

    for _ in 0..2 {
        PrivateRoomListingStore::mark_room_encrypted(&repositories, snapshot.catalog().id())
            .await
            .expect("改标记成功，重复也行");
        assert!(encrypted().await);
    }

    database.close().await;
}

use PrivateRoomListingSaveOutcome::Saved;

async fn create_room(repositories: &PostgresRepositories, owner: PrincipalId) -> RoomCatalogId {
    let catalog_id = RoomCatalogId::from_uuid(Uuid::now_v7());
    PrivateRoomStore::create(repositories, &snapshot(catalog_id, owner), time(0))
        .await
        .expect("建房成功");
    catalog_id
}

async fn archive(
    repositories: &PostgresRepositories,
    catalog_id: RoomCatalogId,
    owner: PrincipalId,
) {
    let snapshot = PrivateRoomStore::find_by_catalog(repositories, catalog_id)
        .await
        .expect("可读")
        .expect("有房间");
    let mut room = snapshot.room().clone();
    room.archive(owner).expect("房主能归档");
    PrivateRoomStore::save(repositories, &room, snapshot.room().version(), time(10))
        .await
        .expect("归档成功");
}

async fn save(
    repositories: &PostgresRepositories,
    record: &PrivateRoomListingRecord,
    expected: Option<AggregateVersion>,
    owner: PrincipalId,
) -> PrivateRoomListingSaveOutcome {
    PrivateRoomListingStore::save_listing(repositories, record, expected, owner)
        .await
        .expect("写入不报错")
}

fn record(
    catalog_id: RoomCatalogId,
    owner: PrincipalId,
    listed: bool,
    share_code: &str,
    sequence: i64,
) -> PrivateRoomListingRecord {
    PrivateRoomListingRecord {
        catalog_id,
        settings: settings(listed, RoomJoinMode::Open),
        share_code: RoomShareCode::parse(share_code).unwrap(),
        first_listed_at: time(1_000),
        updated_by: owner,
        updated_at: time(1_000 + sequence),
        version: version(sequence),
    }
}

fn settings(listed: bool, join_mode: RoomJoinMode) -> RoomListingSettings {
    RoomListingSettings::new(
        listed,
        join_mode,
        RoomListingSettings::DEFAULT_MEMBER_PERMISSIONS,
        true,
    )
    .unwrap()
}

fn version(value: i64) -> AggregateVersion {
    AggregateVersion::new(value).unwrap()
}
