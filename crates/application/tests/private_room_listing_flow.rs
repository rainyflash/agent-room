//! 公开房间的设置（`specs/public-rooms/design.md`）：只有房主能改；第一次公开先在 Matrix 上记下
//! 时间、生成分享码；每个账户最多 1 间公开房间；不加密的房间改回私密时先打开加密。

use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    persistence::RepositoryResult,
    ports::{
        Clock, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId,
        PortFuture, PrivateRoomListingMatrix, PrivateRoomListingRecord,
        PrivateRoomListingSaveOutcome, PrivateRoomListingStore, PrivateRoomSnapshot,
        PrivateRoomStore, RoomContentPolicy, RoomContentPolicyLookup, SecretDigest, SecretFactory,
        SecretGenerationFailure, SecretValue,
    },
    private_rooms::{
        InspectListing, ListingFailureKind, PrivateRoomListingDependencies,
        PrivateRoomListingService, PrivateRoomListingUseCases, UpdateListing,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId, RoomInstanceId},
    private_rooms::{PrivateRoom, PrivateRoomCapability, PrivateRoomPermissions},
    room_listing::{RoomJoinMode, RoomListingSettings},
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState,
    },
    time::UtcMillis,
    version::AggregateVersion,
};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

const NOW: i64 = 1_790_000_000_000;

#[tokio::test]
async fn 房主第一次公开_先在_matrix_上记下时间_有了分享码_再改设置不重写也不换码() {
    let fixture = Fixture::new(true);
    let before = fixture
        .service
        .inspect(inspect(speaker()))
        .await
        .expect("成员看得到设置");
    assert_eq!(before.settings, RoomListingSettings::UNLISTED);
    assert_eq!(before.share_code, None);
    assert!(before.end_to_end_encrypted);

    let listed = fixture
        .service
        .update(update(owner(), settings(true, RoomJoinMode::Open)))
        .await
        .expect("房主能公开");
    assert!(listed.settings.listed());
    assert_eq!(listed.first_listed_at, Some(time(NOW)));
    let code = listed.share_code.clone().expect("公开以后有分享码");
    assert_eq!(
        fixture.matrix.listed.lock().unwrap().as_slice(),
        [time(NOW)]
    );

    fixture.clock.advance(60_000);
    let changed = fixture
        .service
        .update(update(owner(), settings(true, RoomJoinMode::Request)))
        .await
        .expect("公开着改加入方式");
    assert_eq!(changed.settings.join_mode(), RoomJoinMode::Request);
    assert_eq!(changed.share_code, Some(code.clone()));
    assert_eq!(changed.first_listed_at, Some(time(NOW)));
    assert_eq!(
        fixture.matrix.listed.lock().unwrap().len(),
        1,
        "只在第一次公开时写"
    );

    let seen = fixture
        .service
        .inspect(inspect(speaker()))
        .await
        .expect("成员看得到分享码");
    assert_eq!(seen.share_code, Some(code));
    let stored = fixture.listings.record.lock().unwrap().clone().unwrap();
    assert_eq!(stored.version, AggregateVersion::new(2).unwrap());
    assert_eq!(stored.updated_at, time(NOW + 60_000));
}

#[tokio::test]
async fn 只有房主能改_管理员和普通成员都不能_不在房间里的看不到() {
    let fixture = Fixture::new(true);
    for actor in [manager(), speaker()] {
        assert_eq!(
            fixture
                .service
                .update(update(actor, settings(true, RoomJoinMode::Open)))
                .await
                .expect_err("不是房主")
                .kind(),
            ListingFailureKind::Forbidden
        );
    }
    for request in [
        fixture
            .service
            .inspect(inspect(stranger()))
            .await
            .map(|_| ()),
        fixture
            .service
            .update(update(stranger(), settings(true, RoomJoinMode::Open)))
            .await
            .map(|_| ()),
    ] {
        assert_eq!(
            request.expect_err("不在房间里").kind(),
            ListingFailureKind::NotFound
        );
    }
    assert!(fixture.matrix.listed.lock().unwrap().is_empty());
    assert!(fixture.listings.record.lock().unwrap().is_none());
}

#[tokio::test]
async fn 房主名下已经有一间公开房间_就不能再公开一间_存的时候撞上也一样() {
    let fixture = Fixture::new(true);
    *fixture.listings.owner_listed_elsewhere.lock().unwrap() = true;
    assert_eq!(
        fixture
            .service
            .update(update(owner(), settings(true, RoomJoinMode::Open)))
            .await
            .expect_err("超了限额")
            .kind(),
        ListingFailureKind::LimitReached
    );
    assert!(
        fixture.matrix.listed.lock().unwrap().is_empty(),
        "先查限额，不白写状态"
    );

    // 两个请求同时公开两间：先查时还没有，存的时候事务里撞上。
    *fixture.listings.owner_listed_elsewhere.lock().unwrap() = false;
    *fixture.listings.limit_on_save.lock().unwrap() = true;
    assert_eq!(
        fixture
            .service
            .update(update(owner(), settings(true, RoomJoinMode::Open)))
            .await
            .expect_err("事务里撞上限额")
            .kind(),
        ListingFailureKind::LimitReached
    );
    assert!(fixture.listings.record.lock().unwrap().is_none());
}

#[tokio::test]
async fn 分享码撞上别的房间就换一个() {
    let fixture = Fixture::new(true);
    *fixture.listings.taken_codes.lock().unwrap() = 2;
    let listed = fixture
        .service
        .update(update(owner(), settings(true, RoomJoinMode::Open)))
        .await
        .expect("换了码再存");
    let stored = fixture.listings.record.lock().unwrap().clone().unwrap();
    assert_eq!(listed.share_code, Some(stored.share_code));
    assert_eq!(*fixture.listings.attempts.lock().unwrap(), 3);
}

#[tokio::test]
async fn 不加密的房间改回私密_先打开加密再改标记_matrix_写不进就什么都不改() {
    let fixture = Fixture::new(false);
    fixture
        .service
        .update(update(owner(), settings(true, RoomJoinMode::Open)))
        .await
        .expect("公开");

    *fixture.matrix.failing.lock().unwrap() = true;
    assert_eq!(
        fixture
            .service
            .update(update(owner(), settings(false, RoomJoinMode::Open)))
            .await
            .expect_err("Matrix 暂时写不进")
            .kind(),
        ListingFailureKind::DependencyUnavailable
    );
    assert!(!*fixture.encrypted.lock().unwrap());
    assert!(
        fixture
            .listings
            .record
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .settings
            .listed(),
        "还公开着"
    );

    *fixture.matrix.failing.lock().unwrap() = false;
    let unlisted = fixture
        .service
        .update(update(owner(), settings(false, RoomJoinMode::Open)))
        .await
        .expect("再点一次就成了");
    assert!(!unlisted.settings.listed());
    assert!(unlisted.end_to_end_encrypted);
    assert!(*fixture.encrypted.lock().unwrap());
    assert_eq!(*fixture.matrix.encryption.lock().unwrap(), 1);
    assert!(unlisted.share_code.is_some(), "改回私密分享码留着");

    // 已经加密了，再公开、再改回私密都不再动加密。
    fixture
        .service
        .update(update(owner(), settings(true, RoomJoinMode::Open)))
        .await
        .expect("再公开");
    fixture
        .service
        .update(update(owner(), settings(false, RoomJoinMode::Open)))
        .await
        .expect("再改回私密");
    assert_eq!(*fixture.matrix.encryption.lock().unwrap(), 1);
    assert_eq!(fixture.matrix.listed.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn 从没公开过的房间改成私密什么都不写_登录过期的一律不行() {
    let fixture = Fixture::new(true);
    let view = fixture
        .service
        .update(update(owner(), settings(false, RoomJoinMode::Request)))
        .await
        .expect("没什么可改的");
    assert_eq!(view.settings, RoomListingSettings::UNLISTED);
    assert!(fixture.listings.record.lock().unwrap().is_none());

    let mut expired = principal(owner());
    expired.expires_at = time(NOW);
    assert_eq!(
        fixture
            .service
            .inspect(InspectListing {
                actor: expired,
                catalog_id: catalog_id(),
            })
            .await
            .expect_err("登录过期")
            .kind(),
        ListingFailureKind::Forbidden
    );
}

struct Fixture {
    service: PrivateRoomListingService,
    listings: Arc<MemoryListings>,
    matrix: Arc<RecordingMatrix>,
    encrypted: Arc<Mutex<bool>>,
    clock: Arc<TestClock>,
}

impl Fixture {
    fn new(encrypted: bool) -> Self {
        let encrypted = Arc::new(Mutex::new(encrypted));
        let listings = Arc::new(MemoryListings {
            encrypted: encrypted.clone(),
            ..MemoryListings::default()
        });
        let matrix = Arc::new(RecordingMatrix::default());
        let clock = Arc::new(TestClock(Mutex::new(NOW)));
        let service = PrivateRoomListingService::new(PrivateRoomListingDependencies {
            rooms: Arc::new(MemoryRooms(room())),
            listings: listings.clone(),
            policies: Arc::new(Policies(encrypted.clone())),
            matrix: matrix.clone(),
            secrets: Arc::new(CountingSecrets(Mutex::new(0))),
            clock: clock.clone(),
        });
        Self {
            service,
            listings,
            matrix,
            encrypted,
            clock,
        }
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

fn inspect(actor: PrincipalId) -> InspectListing {
    InspectListing {
        actor: principal(actor),
        catalog_id: catalog_id(),
    }
}

fn update(actor: PrincipalId, settings: RoomListingSettings) -> UpdateListing {
    UpdateListing {
        actor: principal(actor),
        catalog_id: catalog_id(),
        settings,
    }
}

fn room() -> PrivateRoom {
    let mut room = PrivateRoom::create(catalog_id(), owner());
    for (member, permissions) in [
        (manager(), PrivateRoomPermissions::ALL),
        (speaker(), speak_permissions()),
    ] {
        room.invite(owner(), member, permissions).unwrap();
        room.accept_invitation(member).unwrap();
    }
    room
}

fn speak_permissions() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([
        PrivateRoomCapability::View,
        PrivateRoomCapability::Speak,
    ])
    .unwrap()
}

fn snapshot(room: PrivateRoom) -> PrivateRoomSnapshot {
    let catalog = RoomCatalog::new(
        catalog_id(),
        RoomCatalogFields {
            kind: RoomCatalogKind::PrivateRoom,
            slug: None,
            name: "读书会".to_owned(),
            description: String::new(),
            language: None,
            matrix_space_id: None,
            owner_principal_id: Some(owner()),
            visibility: RoomCatalogVisibility::Private,
            retention_days: None,
            status: RoomCatalogStatus::Active,
        },
    )
    .unwrap();
    let instance = RoomInstance::restore(
        RoomInstanceId::from_uuid(Uuid::from_u128(20)),
        RoomInstanceFields {
            catalog_id: catalog_id(),
            matrix_room_id: MatrixRoomReference::new("!club:matrix.test".to_owned()).unwrap(),
            region: None,
            capacity: RoomCapacity::standard(),
            projected_member_count: 0,
            allocated_slots: 0,
            activity_score_millis: 0,
            state: RoomInstanceState::Active,
        },
    )
    .unwrap();
    PrivateRoomSnapshot::new(catalog, instance, room).unwrap()
}

fn principal(principal_id: PrincipalId) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id,
        matrix_user_id: "@someone:matrix.test".to_owned(),
        display_name: "Someone".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(NOW - 1_000),
        expires_at: UtcMillis::new(i64::MAX).unwrap(),
        recently_authenticated: false,
    }
}

fn person(sequence: u128) -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(sequence))
}

fn owner() -> PrincipalId {
    person(1)
}

/// 有全部权限（管理员）的成员。
fn manager() -> PrincipalId {
    person(2)
}

fn speaker() -> PrincipalId {
    person(3)
}

fn stranger() -> PrincipalId {
    person(4)
}

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(10))
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}

struct TestClock(Mutex<i64>);

impl TestClock {
    fn advance(&self, millis: i64) {
        *self.0.lock().unwrap() += millis;
    }
}

impl Clock for TestClock {
    fn now(&self) -> UtcMillis {
        time(*self.0.lock().unwrap())
    }
}

struct CountingSecrets(Mutex<u64>);

impl SecretFactory for CountingSecrets {
    fn generate(&self) -> Result<SecretValue, SecretGenerationFailure> {
        let mut counter = self.0.lock().unwrap();
        *counter += 1;
        Ok(SecretValue::new(format!("secret-{counter}")).unwrap())
    }

    fn digest(&self, value: &str) -> SecretDigest {
        SecretDigest::from_array(Sha256::digest(value.as_bytes()).into())
    }
}

struct MemoryRooms(PrivateRoom);

impl PrivateRoomStore for MemoryRooms {
    fn create<'a>(
        &'a self,
        _snapshot: &'a PrivateRoomSnapshot,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("公开设置不建房间") })
    }

    fn find_by_catalog(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        let found = (catalog_id == self.0.catalog_id()).then(|| snapshot(self.0.clone()));
        Box::pin(async move { Ok(found) })
    }

    fn list_for_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("公开设置不列房间") })
    }

    fn find_by_matrix_room<'a>(
        &'a self,
        _matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("公开设置按目录找房间") })
    }

    fn save<'a>(
        &'a self,
        _room: &'a PrivateRoom,
        _expected_version: AggregateVersion,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("公开设置不改成员") })
    }

    fn rename<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        _name: &'a str,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("公开设置不改名") })
    }
}

#[derive(Default)]
struct MemoryListings {
    record: Mutex<Option<PrivateRoomListingRecord>>,
    /// 房主名下还有一间别的公开房间。
    owner_listed_elsewhere: Mutex<bool>,
    /// 存的时候事务里才发现超了限额。
    limit_on_save: Mutex<bool>,
    /// 接下来这么多次存都说分享码被占了。
    taken_codes: Mutex<u32>,
    attempts: Mutex<u32>,
    encrypted: Arc<Mutex<bool>>,
}

impl PrivateRoomListingStore for MemoryListings {
    fn listing(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomListingRecord>>> {
        let record = self.record.lock().unwrap().clone();
        Box::pin(async move { Ok(record) })
    }

    fn listed_rooms_owned_by(
        &self,
        _owner: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<RoomCatalogId>>> {
        let mut rooms = Vec::new();
        if *self.owner_listed_elsewhere.lock().unwrap() {
            rooms.push(RoomCatalogId::from_uuid(Uuid::from_u128(11)));
        }
        if self
            .record
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|record| record.settings.listed())
        {
            rooms.push(catalog_id());
        }
        Box::pin(async move { Ok(rooms) })
    }

    fn save_listing<'a>(
        &'a self,
        record: &'a PrivateRoomListingRecord,
        expected: Option<AggregateVersion>,
        _owner: PrincipalId,
    ) -> PortFuture<'a, RepositoryResult<PrivateRoomListingSaveOutcome>> {
        *self.attempts.lock().unwrap() += 1;
        let outcome = {
            let mut stored = self.record.lock().unwrap();
            let mut taken = self.taken_codes.lock().unwrap();
            if stored.as_ref().map(|stored| stored.version) != expected {
                PrivateRoomListingSaveOutcome::Stale
            } else if *self.limit_on_save.lock().unwrap() && record.settings.listed() {
                PrivateRoomListingSaveOutcome::OwnerLimitReached
            } else if *taken > 0 {
                *taken -= 1;
                PrivateRoomListingSaveOutcome::ShareCodeTaken
            } else {
                *stored = Some(record.clone());
                PrivateRoomListingSaveOutcome::Saved
            }
        };
        Box::pin(async move { Ok(outcome) })
    }

    fn create_listed_room<'a>(
        &'a self,
        _snapshot: &'a PrivateRoomSnapshot,
        _listing: &'a PrivateRoomListingRecord,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<PrivateRoomListingSaveOutcome>> {
        Box::pin(async { unreachable!("改设置不建房") })
    }

    fn mark_room_encrypted(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        *self.encrypted.lock().unwrap() = true;
        Box::pin(async { Ok(()) })
    }
}

struct Policies(Arc<Mutex<bool>>);

impl RoomContentPolicyLookup for Policies {
    fn content_policy<'a>(
        &'a self,
        _matrix_room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, RepositoryResult<Option<RoomContentPolicy>>> {
        let end_to_end_encrypted = *self.0.lock().unwrap();
        Box::pin(async move {
            Ok(Some(RoomContentPolicy {
                retention_days: None,
                end_to_end_encrypted,
            }))
        })
    }
}

#[derive(Default)]
struct RecordingMatrix {
    listed: Mutex<Vec<UtcMillis>>,
    encryption: Mutex<u32>,
    failing: Mutex<bool>,
}

impl RecordingMatrix {
    fn fail(&self, operation: MatrixOperation) -> MatrixResult<()> {
        if *self.failing.lock().unwrap() {
            Err(MatrixFailure::new(
                operation,
                MatrixFailureKind::DependencyUnavailable,
            ))
        } else {
            Ok(())
        }
    }
}

impl PrivateRoomListingMatrix for RecordingMatrix {
    fn mark_listed<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        first_listed_at: UtcMillis,
    ) -> PortFuture<'a, MatrixResult<()>> {
        let result = self.fail(MatrixOperation::SendStateEvent);
        if result.is_ok() {
            self.listed.lock().unwrap().push(first_listed_at);
        }
        Box::pin(async move { result })
    }

    fn enable_encryption<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        let result = self.fail(MatrixOperation::SendStateEvent);
        if result.is_ok() {
            *self.encryption.lock().unwrap() += 1;
        }
        Box::pin(async move { result })
    }
}
