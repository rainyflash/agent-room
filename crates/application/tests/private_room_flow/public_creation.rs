//! 新建时就选公开（`specs/public-rooms/design.md` 第 3a-2 步）：房间不加密、换一个别名，建好先写下
//! 公开的时间，再把房间和设置一起存；每个账户只能有一间公开房间，转让房主也算；只给人自己建。

use agent_room_application::ports::{
    MatrixRoomEncryption, PrivateRoomListingMatrix, PrivateRoomListingRecord,
    PrivateRoomListingSaveOutcome, PrivateRoomListingStore, SecretDigest, SecretFactory,
    SecretGenerationFailure, SecretValue,
};
use agent_room_domain::room_listing::{RoomJoinMode, RoomListingSettings, RoomShareCode};
use sha2::{Digest as _, Sha256};

use super::*;

#[tokio::test]
async fn 新建时就选公开_不加密_写下公开的时间_房间和设置一起存() {
    let fixture = Fixture::new();
    let snapshot = fixture
        .service
        .create(public_request(&fixture))
        .await
        .expect("建房成功");

    assert_eq!(
        fixture.events(),
        vec![
            "matrix.create",
            "matrix.speak.batch",
            "matrix.listed",
            "store.create_listed"
        ],
        "Matrix 房间建好、写下公开时间以后才存"
    );
    let creation = fixture.matrix.creation().expect("建了 Matrix 房间");
    assert_eq!(
        creation.request().encryption(),
        MatrixRoomEncryption::Unencrypted
    );
    assert!(
        creation
            .alias()
            .as_str()
            .starts_with("agent-room-public-room-"),
        "和私密建房用不同的别名"
    );
    let record = fixture.listings.record().expect("存了公开设置");
    assert_eq!(record.catalog_id, snapshot.catalog().id());
    assert!(record.settings.listed());
    assert_eq!(record.settings.join_mode(), RoomJoinMode::Request);
    assert_eq!(record.first_listed_at, UtcMillis::new(1_000).unwrap());
    assert_eq!(record.updated_by, fixture.owner);
    assert_eq!(record.version, AggregateVersion::INITIAL.next().unwrap());
    assert_eq!(
        *fixture.listings.listed_at.lock().unwrap(),
        Some(UtcMillis::new(1_000).unwrap())
    );

    // 同一个请求再来一次：认出是同一间，不再建。
    fixture.clear_events();
    let again = fixture
        .service
        .create(public_request(&fixture))
        .await
        .expect("重试认得出来");
    assert_eq!(again.catalog().id(), snapshot.catalog().id());
    assert!(fixture.events().is_empty());
    // 同一个房间号按私密再建：是另一个请求撞了号。
    let conflict = fixture
        .service
        .create(fixture.creation_request(speaker_permissions()))
        .await
        .expect_err("公开不公开对不上");
    assert_eq!(conflict.kind(), PrivateRoomFailureKind::Conflict);
}

#[tokio::test]
async fn 私密建房照旧加密_用原来的别名_不写公开设置() {
    let fixture = Fixture::new();
    fixture
        .service
        .create(fixture.creation_request(speaker_permissions()))
        .await
        .expect("建房成功");
    let creation = fixture.matrix.creation().expect("建了 Matrix 房间");
    assert_eq!(
        creation.request().encryption(),
        MatrixRoomEncryption::EndToEnd
    );
    assert!(creation.alias().as_str().starts_with("agent-room-private-"));
    assert!(fixture.listings.record().is_none());

    let conflict = fixture
        .service
        .create(public_request(&fixture))
        .await
        .expect_err("同一个房间号按公开再建");
    assert_eq!(conflict.kind(), PrivateRoomFailureKind::Conflict);
}

#[tokio::test]
async fn 名下已经有公开房间_建之前就拦下_存的时候撞上也拦下() {
    let fixture = Fixture::new();
    *fixture.listings.owner_listed_elsewhere.lock().unwrap() = true;
    let failure = fixture
        .service
        .create(public_request(&fixture))
        .await
        .expect_err("超了限额");
    assert_eq!(
        failure.kind(),
        PrivateRoomFailureKind::PublicRoomLimitReached
    );
    assert!(fixture.events().is_empty(), "没去建 Matrix 房间");

    let fixture = Fixture::new();
    *fixture.listings.limit_on_save.lock().unwrap() = true;
    let failure = fixture
        .service
        .create(public_request(&fixture))
        .await
        .expect_err("同时建的另一间先存上了");
    assert_eq!(
        failure.kind(),
        PrivateRoomFailureKind::PublicRoomLimitReached
    );
    assert!(fixture.store.snapshot.lock().unwrap().is_none());
}

fn transfer_request(fixture: &Fixture) -> TransferPrivateRoomOwnership {
    TransferPrivateRoomOwnership {
        actor: fixture.owner_actor(),
        catalog_id: fixture.catalog,
        target_principal_id: fixture.member,
        former_owner_permissions: viewer_permissions(),
    }
}

/// 把 [`Fixture::joined`] 建好的房间记成公开着的。
fn list_room(fixture: &Fixture) {
    *fixture.listings.record.lock().unwrap() = Some(PrivateRoomListingRecord {
        catalog_id: fixture.catalog,
        settings: listed_settings(),
        share_code: RoomShareCode::parse("Trn001").unwrap(),
        first_listed_at: UtcMillis::new(500).unwrap(),
        updated_by: fixture.owner,
        updated_at: UtcMillis::new(500).unwrap(),
        version: AggregateVersion::INITIAL.next().unwrap(),
    });
}

#[tokio::test]
async fn 公开房间转给已有公开房间的人_什么都不动就拦下() {
    let fixture = Fixture::joined(viewer_permissions()).await;
    list_room(&fixture);
    *fixture.listings.owner_listed_elsewhere.lock().unwrap() = true;
    fixture.clear_events();

    let failure = fixture
        .service
        .transfer_ownership(transfer_request(&fixture))
        .await
        .expect_err("对方已经有一间公开房间");

    assert_eq!(
        failure.kind(),
        PrivateRoomFailureKind::PublicRoomLimitReached
    );
    assert!(fixture.events().is_empty(), "没去改 Matrix 上的发言权");
    let stored = fixture.store.snapshot.lock().unwrap().clone().unwrap();
    assert_eq!(stored.room().owner_principal_id(), fixture.owner);
}

#[tokio::test]
async fn 事务里才撞上限额_房主不变_压下去的发言权还回去() {
    let fixture = Fixture::joined(viewer_permissions()).await;
    list_room(&fixture);
    *fixture.listings.limit_on_save.lock().unwrap() = true;
    fixture.clear_events();

    let failure = fixture
        .service
        .transfer_ownership(transfer_request(&fixture))
        .await
        .expect_err("对方同时公开了别的房间");

    assert_eq!(
        failure.kind(),
        PrivateRoomFailureKind::PublicRoomLimitReached
    );
    assert_eq!(
        fixture.events(),
        vec!["matrix.speak.off", "matrix.speak.on"]
    );
    let stored = fixture.store.snapshot.lock().unwrap().clone().unwrap();
    assert_eq!(stored.room().owner_principal_id(), fixture.owner);
}

#[tokio::test]
async fn 公开房间转给名下没有公开房间的人_照常转() {
    let fixture = Fixture::joined(viewer_permissions()).await;
    list_room(&fixture);

    let transferred = fixture
        .service
        .transfer_ownership(transfer_request(&fixture))
        .await
        .expect("这间自己不算第二间");

    assert_eq!(transferred.room().owner_principal_id(), fixture.member);
}

#[tokio::test]
async fn 私密房间照常转给已有公开房间的人() {
    let fixture = Fixture::joined(viewer_permissions()).await;
    *fixture.listings.owner_listed_elsewhere.lock().unwrap() = true;

    let transferred = fixture
        .service
        .transfer_ownership(transfer_request(&fixture))
        .await
        .expect("私密的不占名额");

    assert_eq!(transferred.room().owner_principal_id(), fixture.member);
}

#[tokio::test]
async fn 分享码撞了换一个再存() {
    let fixture = Fixture::new();
    *fixture.listings.taken_codes.lock().unwrap() = 2;
    fixture
        .service
        .create(public_request(&fixture))
        .await
        .expect("换了码就存上");
    assert_eq!(
        fixture.events(),
        vec![
            "matrix.create",
            "matrix.speak.batch",
            "matrix.listed",
            "store.create_listed",
            "store.create_listed",
            "store.create_listed"
        ]
    );
    let codes = fixture.listings.tried_codes.lock().unwrap().clone();
    assert_eq!(codes.len(), 3);
    assert_ne!(codes[0], codes[1]);
    assert_ne!(codes[1], codes[2]);
    assert_eq!(
        fixture.listings.record().unwrap().share_code.as_str(),
        codes[2]
    );
}

#[tokio::test]
async fn 只给人自己建_设置也得是公开的() {
    let fixture = Fixture::new();
    let agent = agent_id(7);
    fixture.agents.own(fixture.owner, agent);
    let mut by_agent = fixture.agent_creation_request(
        PrivateRoomActor::Device(device(fixture.owner, "@owner:matrix.test")),
        agent,
    );
    by_agent.listing = Some(listed_settings());
    let mut unlisted = public_request(&fixture);
    unlisted.listing = Some(RoomListingSettings::UNLISTED);

    for request in [by_agent, unlisted] {
        let failure = fixture
            .service
            .create(request)
            .await
            .expect_err("不让新建时就公开");
        assert_eq!(failure.kind(), PrivateRoomFailureKind::InvalidRequest);
        assert_eq!(failure.rejected(), Some("listing"));
    }
    assert!(fixture.events().is_empty());
}

fn public_request(fixture: &Fixture) -> CreatePrivateRoom {
    CreatePrivateRoom {
        listing: Some(listed_settings()),
        ..fixture.creation_request(speaker_permissions())
    }
}

fn listed_settings() -> RoomListingSettings {
    RoomListingSettings::new(
        true,
        RoomJoinMode::Request,
        RoomListingSettings::DEFAULT_MEMBER_PERMISSIONS,
        true,
    )
    .unwrap()
}

/// 公开设置、Matrix 上公开的时间、分享码的随机数，都记在这一个假的里。建房时把房间写进
/// [`TestStore`]，和真的一样在一个事务里一起存。
pub(super) struct TestListings {
    store: Arc<TestStore>,
    events: Arc<Mutex<Vec<&'static str>>>,
    record: Mutex<Option<PrivateRoomListingRecord>>,
    listed_at: Mutex<Option<UtcMillis>>,
    owner_listed_elsewhere: Mutex<bool>,
    limit_on_save: Mutex<bool>,
    /// 接下来这么多次存都说分享码被占了。
    taken_codes: Mutex<u32>,
    tried_codes: Mutex<Vec<String>>,
    secrets: Mutex<u64>,
}

impl TestListings {
    pub(super) fn new(store: Arc<TestStore>, events: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            store,
            events,
            record: Mutex::new(None),
            listed_at: Mutex::new(None),
            owner_listed_elsewhere: Mutex::new(false),
            limit_on_save: Mutex::new(false),
            taken_codes: Mutex::new(0),
            tried_codes: Mutex::new(Vec::new()),
            secrets: Mutex::new(0),
        }
    }

    fn record(&self) -> Option<PrivateRoomListingRecord> {
        self.record.lock().unwrap().clone()
    }
}

impl PrivateRoomListingStore for TestListings {
    fn listing(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomListingRecord>>> {
        let record = self.record();
        Box::pin(async move { Ok(record) })
    }

    fn listed_rooms_owned_by(
        &self,
        _owner: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<RoomCatalogId>>> {
        let mut rooms = Vec::new();
        if *self.owner_listed_elsewhere.lock().unwrap() {
            rooms.push(catalog_id(99));
        }
        if let Some(record) = self.record().filter(|record| record.settings.listed()) {
            rooms.push(record.catalog_id);
        }
        Box::pin(async move { Ok(rooms) })
    }

    fn save_listing<'a>(
        &'a self,
        _record: &'a PrivateRoomListingRecord,
        _expected: Option<AggregateVersion>,
        _owner: PrincipalId,
    ) -> PortFuture<'a, RepositoryResult<PrivateRoomListingSaveOutcome>> {
        Box::pin(async { unreachable!("建房不改设置") })
    }

    fn create_listed_room<'a>(
        &'a self,
        snapshot: &'a PrivateRoomSnapshot,
        listing: &'a PrivateRoomListingRecord,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<PrivateRoomListingSaveOutcome>> {
        Box::pin(async move {
            self.events.lock().unwrap().push("store.create_listed");
            self.tried_codes
                .lock()
                .unwrap()
                .push(listing.share_code.as_str().to_owned());
            if *self.limit_on_save.lock().unwrap() {
                return Ok(PrivateRoomListingSaveOutcome::OwnerLimitReached);
            }
            let mut taken = self.taken_codes.lock().unwrap();
            if *taken > 0 {
                *taken -= 1;
                return Ok(PrivateRoomListingSaveOutcome::ShareCodeTaken);
            }
            let mut stored = self.store.snapshot.lock().unwrap();
            if stored.is_some() {
                return Ok(PrivateRoomListingSaveOutcome::Stale);
            }
            *stored = Some(snapshot.clone());
            *self.record.lock().unwrap() = Some(listing.clone());
            Ok(PrivateRoomListingSaveOutcome::Saved)
        })
    }

    fn save_transferred_room<'a>(
        &'a self,
        room: &'a agent_room_domain::private_rooms::PrivateRoom,
        expected: AggregateVersion,
        changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<PrivateRoomListingSaveOutcome>> {
        Box::pin(async move {
            if *self.limit_on_save.lock().unwrap() {
                return Ok(PrivateRoomListingSaveOutcome::OwnerLimitReached);
            }
            self.store.save(room, expected, changed_at).await?;
            Ok(PrivateRoomListingSaveOutcome::Saved)
        })
    }

    fn mark_room_encrypted(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        Box::pin(async { unreachable!("建房不改加密") })
    }
}

impl PrivateRoomListingMatrix for TestListings {
    fn mark_listed<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        first_listed_at: UtcMillis,
    ) -> PortFuture<'a, MatrixResult<()>> {
        self.events.lock().unwrap().push("matrix.listed");
        *self.listed_at.lock().unwrap() = Some(first_listed_at);
        Box::pin(async { Ok(()) })
    }

    fn enable_encryption<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { unreachable!("建房不打开加密") })
    }
}

impl SecretFactory for TestListings {
    fn generate(&self) -> Result<SecretValue, SecretGenerationFailure> {
        let mut counter = self.secrets.lock().unwrap();
        *counter += 1;
        Ok(SecretValue::new(format!("secret-{counter}")).unwrap())
    }

    fn digest(&self, value: &str) -> SecretDigest {
        SecretDigest::from_array(Sha256::digest(value.as_bytes()).into())
    }
}
