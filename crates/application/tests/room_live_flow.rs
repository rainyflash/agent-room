//! 分享页放实况的开关（`specs/redesign/project-rooms.md`“分享页放实况”，3P-6a）：房间里的人都读得到
//! 开没开、差哪几样；只有房主和管理员能开关；有一样不满足就开不了，照实说是哪一样；打开前再问一遍
//! 聊天服务器加没加密；已经开着的再开不换那一刻。

use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    persistence::RepositoryResult,
    ports::{
        Clock, EnableRoomLiveViewOutcome, MatrixFailure, MatrixFailureKind, MatrixOperation,
        MatrixResult, MatrixRoomId, PortFuture, PrivateRoomSnapshot, PrivateRoomStore,
        RoomEncryptionState, RoomLiveViewFacts, RoomLiveViewRecord, RoomLiveViewStore,
    },
    room_live::{
        InspectLiveView, LiveViewCondition, LiveViewFailureKind, LiveViewState,
        RoomLiveViewDependencies, RoomLiveViewService, RoomLiveViewUseCases, UpdateLiveView,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId, RoomInstanceId},
    private_rooms::{PrivateRoom, PrivateRoomPermissions},
    room_listing::RoomJoinMode,
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState,
    },
    time::UtcMillis,
    version::AggregateVersion,
};
use uuid::Uuid;

const NOW: i64 = 1_790_000_000_000;
const EARLIER: i64 = NOW - 86_400_000;

#[tokio::test]
async fn 房主和管理员能打开_记下这一刻_再开不换() {
    let fixture = Fixture::new(room(), eligible());

    let opened = fixture
        .service
        .update(update(manager(), true))
        .await
        .expect("管理员能打开");
    assert_eq!(
        opened,
        LiveViewState {
            enabled: Some(record(manager(), NOW)),
            unmet: Vec::new(),
            can_change: true,
        }
    );
    assert!(opened.live());
    assert_eq!(fixture.encryption.asked(), 1);

    let again = fixture
        .service
        .update(update(owner(), true))
        .await
        .expect("再开一次照样成");
    assert_eq!(again.enabled, Some(record(manager(), NOW)));
    assert_eq!(fixture.live_views.enabled_calls(), 1);
    assert_eq!(fixture.encryption.asked(), 1, "开着的不再问聊天服务器");
}

#[tokio::test]
async fn 房主关掉以后再读是关着的() {
    let fixture = Fixture::new(room(), enabled(eligible()));

    let closed = fixture
        .service
        .update(update(owner(), false))
        .await
        .expect("房主能关");
    assert_eq!(closed.enabled, None);
    assert!(closed.unmet.is_empty());
    assert!(!closed.live());

    let read = fixture
        .service
        .inspect(inspect(member()))
        .await
        .expect("成员读得到");
    assert_eq!(read.enabled, None);
    assert!(!read.can_change);
}

#[tokio::test]
async fn 成员读得到开着_却不能开关_外人和登录过期的都不行() {
    let fixture = Fixture::new(room(), enabled(eligible()));

    let read = fixture
        .service
        .inspect(inspect(member()))
        .await
        .expect("成员读得到");
    assert!(read.live());
    assert!(!read.can_change);
    let owner_read = fixture.service.inspect(inspect(owner())).await.unwrap();
    assert!(owner_read.can_change);

    for enabled in [true, false] {
        let failure = fixture
            .service
            .update(update(member(), enabled))
            .await
            .expect_err("普通成员不能开关");
        assert_eq!(failure.kind(), LiveViewFailureKind::Forbidden);
    }
    let failure = fixture
        .service
        .inspect(inspect(stranger()))
        .await
        .expect_err("外人看不到");
    assert_eq!(failure.kind(), LiveViewFailureKind::NotFound);

    let mut expired = principal(owner());
    expired.expires_at = time(NOW);
    let failure = fixture
        .service
        .update(UpdateLiveView {
            actor: expired,
            catalog_id: catalog_id(),
            enabled: true,
        })
        .await
        .expect_err("登录过期");
    assert_eq!(failure.kind(), LiveViewFailureKind::Forbidden);
    assert_eq!(fixture.live_views.enabled_calls(), 0);
    assert!(fixture.live_views.facts().live_view.is_some(), "没关掉");
}

#[tokio::test]
async fn 有一样不满足就开不了_说是哪一样() {
    let cases: [(RoomLiveViewFacts, LiveViewCondition); 4] = [
        (
            RoomLiveViewFacts {
                listing: None,
                ..eligible()
            },
            LiveViewCondition::NotListed,
        ),
        (
            RoomLiveViewFacts {
                listing: Some((true, RoomJoinMode::Request)),
                ..eligible()
            },
            LiveViewCondition::JoinMode,
        ),
        (
            RoomLiveViewFacts {
                platform_hold: true,
                ..eligible()
            },
            LiveViewCondition::PlatformHold,
        ),
        (
            RoomLiveViewFacts {
                end_to_end_encrypted: true,
                ..eligible()
            },
            LiveViewCondition::Encrypted,
        ),
    ];
    for (facts, condition) in cases {
        let fixture = Fixture::new(room(), facts);
        let failure = fixture
            .service
            .update(update(owner(), true))
            .await
            .expect_err("开不了");
        assert_eq!(
            failure.kind(),
            LiveViewFailureKind::NotEligible(condition),
            "{condition:?}"
        );
        assert_eq!(fixture.live_views.enabled_calls(), 0);
        assert_eq!(fixture.encryption.asked(), 0);
    }
}

#[tokio::test]
async fn 几样都不满足时读出全部_开的时候说排在最前的一样() {
    let fixture = Fixture::new(
        room(),
        RoomLiveViewFacts {
            listing: Some((false, RoomJoinMode::InviteOnly)),
            end_to_end_encrypted: true,
            ..eligible()
        },
    );

    let read = fixture.service.inspect(inspect(owner())).await.unwrap();
    assert_eq!(
        read.unmet,
        [
            LiveViewCondition::NotListed,
            LiveViewCondition::Encrypted,
            LiveViewCondition::JoinMode,
        ]
    );
    let failure = fixture
        .service
        .update(update(owner(), true))
        .await
        .expect_err("开不了");
    assert_eq!(
        failure.kind(),
        LiveViewFailureKind::NotEligible(LiveViewCondition::NotListed)
    );
}

#[tokio::test]
async fn 子房间和关了的房间开不了_关还是关得掉() {
    let fixture = Fixture::with_parent(room(), eligible(), Some(parent_catalog_id()));
    let failure = fixture
        .service
        .update(update(owner(), true))
        .await
        .expect_err("子房间不放");
    assert_eq!(
        failure.kind(),
        LiveViewFailureKind::NotEligible(LiveViewCondition::SubRoom)
    );

    let mut archived = room();
    archived.archive(owner()).unwrap();
    let fixture = Fixture::new(archived, enabled(eligible()));
    let read = fixture.service.inspect(inspect(owner())).await.unwrap();
    assert_eq!(read.unmet, [LiveViewCondition::Closed]);
    assert!(!read.live(), "关了的房间开着也不放");
    let failure = fixture
        .service
        .update(update(owner(), true))
        .await
        .expect_err("关了的房间开不了");
    assert_eq!(
        failure.kind(),
        LiveViewFailureKind::NotEligible(LiveViewCondition::Closed)
    );
    let closed = fixture
        .service
        .update(update(owner(), false))
        .await
        .expect("关了的房间照样关得掉");
    assert_eq!(closed.enabled, None);
}

#[tokio::test]
async fn 聊天服务器说加了密就开不了_问不到就过会儿再点() {
    let fixture = Fixture::new(room(), eligible());
    *fixture.encryption.answer.lock().unwrap() = Ok(true);
    let failure = fixture
        .service
        .update(update(owner(), true))
        .await
        .expect_err("房间状态里有加密");
    assert_eq!(
        failure.kind(),
        LiveViewFailureKind::NotEligible(LiveViewCondition::Encrypted)
    );

    *fixture.encryption.answer.lock().unwrap() = Err(MatrixFailureKind::Timeout);
    let failure = fixture
        .service
        .update(update(owner(), true))
        .await
        .expect_err("问不到");
    assert_eq!(failure.kind(), LiveViewFailureKind::DependencyUnavailable);
    assert_eq!(fixture.live_views.enabled_calls(), 0);
}

#[tokio::test]
async fn 开的那一下设置被改了_照实说是哪一样_没变就是冲突() {
    let fixture = Fixture::new(room(), eligible());
    *fixture.live_views.race.lock().unwrap() = Some(RoomLiveViewFacts {
        listing: Some((true, RoomJoinMode::InviteOnly)),
        ..eligible()
    });
    let failure = fixture
        .service
        .update(update(owner(), true))
        .await
        .expect_err("刚改成只能邀请");
    assert_eq!(
        failure.kind(),
        LiveViewFailureKind::NotEligible(LiveViewCondition::JoinMode)
    );

    let fixture = Fixture::new(room(), eligible());
    *fixture.live_views.race.lock().unwrap() = Some(eligible());
    let failure = fixture
        .service
        .update(update(owner(), true))
        .await
        .expect_err("库里说不行、再读又都满足");
    assert_eq!(failure.kind(), LiveViewFailureKind::Conflict);
}

struct Fixture {
    service: RoomLiveViewService,
    live_views: Arc<FakeLiveViews>,
    encryption: Arc<FakeEncryption>,
}

impl Fixture {
    fn new(room: PrivateRoom, facts: RoomLiveViewFacts) -> Self {
        Self::with_parent(room, facts, None)
    }

    fn with_parent(
        room: PrivateRoom,
        facts: RoomLiveViewFacts,
        parent: Option<RoomCatalogId>,
    ) -> Self {
        let live_views = Arc::new(FakeLiveViews {
            facts: Mutex::new(facts),
            race: Mutex::new(None),
            enabled_calls: Mutex::new(0),
        });
        let encryption = Arc::new(FakeEncryption {
            answer: Mutex::new(Ok(false)),
            asked: Mutex::new(0),
        });
        let service = RoomLiveViewService::new(RoomLiveViewDependencies {
            rooms: Arc::new(MemoryRooms { room, parent }),
            live_views: live_views.clone(),
            encryption: encryption.clone(),
            clock: Arc::new(TestClock),
        });
        Self {
            service,
            live_views,
            encryption,
        }
    }
}

/// 库里的几样；`race` 设上时，打开那一下先换成它、回“不满足”，像别的请求刚改了设置。
struct FakeLiveViews {
    facts: Mutex<RoomLiveViewFacts>,
    race: Mutex<Option<RoomLiveViewFacts>>,
    enabled_calls: Mutex<usize>,
}

impl FakeLiveViews {
    fn facts(&self) -> RoomLiveViewFacts {
        *self.facts.lock().unwrap()
    }

    fn enabled_calls(&self) -> usize {
        *self.enabled_calls.lock().unwrap()
    }
}

impl RoomLiveViewStore for FakeLiveViews {
    fn live_view_facts(
        &self,
        catalog: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<RoomLiveViewFacts>>> {
        let facts = (catalog == catalog_id()).then(|| self.facts());
        Box::pin(async move { Ok(facts) })
    }

    fn enable_live_view(
        &self,
        catalog: RoomCatalogId,
        enabled_by: PrincipalId,
        at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<EnableRoomLiveViewOutcome>> {
        assert_eq!(catalog, catalog_id());
        *self.enabled_calls.lock().unwrap() += 1;
        let outcome = if let Some(raced) = self.race.lock().unwrap().take() {
            *self.facts.lock().unwrap() = raced;
            EnableRoomLiveViewOutcome::NotEligible
        } else {
            let mut facts = self.facts.lock().unwrap();
            if let Some(existing) = facts.live_view {
                EnableRoomLiveViewOutcome::AlreadyEnabled(existing)
            } else {
                let created = RoomLiveViewRecord {
                    enabled_by,
                    enabled_at: at,
                };
                facts.live_view = Some(created);
                EnableRoomLiveViewOutcome::Enabled(created)
            }
        };
        Box::pin(async move { Ok(outcome) })
    }

    fn disable_live_view(&self, catalog: RoomCatalogId) -> PortFuture<'_, RepositoryResult<()>> {
        assert_eq!(catalog, catalog_id());
        self.facts.lock().unwrap().live_view = None;
        Box::pin(async { Ok(()) })
    }
}

struct FakeEncryption {
    answer: Mutex<Result<bool, MatrixFailureKind>>,
    asked: Mutex<usize>,
}

impl FakeEncryption {
    fn asked(&self) -> usize {
        *self.asked.lock().unwrap()
    }
}

impl RoomEncryptionState for FakeEncryption {
    fn room_encrypted<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<bool>> {
        assert_eq!(room_id.as_str(), "!club:matrix.test");
        *self.asked.lock().unwrap() += 1;
        let answer = (*self.answer.lock().unwrap())
            .map_err(|kind| MatrixFailure::new(MatrixOperation::ReadRoomState, kind));
        Box::pin(async move { answer })
    }
}

fn eligible() -> RoomLiveViewFacts {
    RoomLiveViewFacts {
        listing: Some((true, RoomJoinMode::Open)),
        end_to_end_encrypted: false,
        platform_hold: false,
        live_view: None,
    }
}

fn enabled(facts: RoomLiveViewFacts) -> RoomLiveViewFacts {
    RoomLiveViewFacts {
        live_view: Some(record(owner(), EARLIER)),
        ..facts
    }
}

fn record(enabled_by: PrincipalId, at: i64) -> RoomLiveViewRecord {
    RoomLiveViewRecord {
        enabled_by,
        enabled_at: time(at),
    }
}

fn update(actor: PrincipalId, enabled: bool) -> UpdateLiveView {
    UpdateLiveView {
        actor: principal(actor),
        catalog_id: catalog_id(),
        enabled,
    }
}

fn inspect(actor: PrincipalId) -> InspectLiveView {
    InspectLiveView {
        actor: principal(actor),
        catalog_id: catalog_id(),
    }
}

fn room() -> PrivateRoom {
    let mut room = PrivateRoom::create(catalog_id(), owner());
    room.invite(owner(), manager(), PrivateRoomPermissions::ALL)
        .unwrap();
    room.accept_invitation(manager()).unwrap();
    room.invite(owner(), member(), PrivateRoomPermissions::INVITED_BY_AGENT)
        .unwrap();
    room.accept_invitation(member()).unwrap();
    room
}

fn snapshot(room: PrivateRoom, parent: Option<RoomCatalogId>) -> PrivateRoomSnapshot {
    let archived = room.status().as_str() == "archived";
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
            status: if archived {
                RoomCatalogStatus::Archived
            } else {
                RoomCatalogStatus::Active
            },
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
            allocated_slots: 0,
            state: if archived {
                RoomInstanceState::Archived
            } else {
                RoomInstanceState::Active
            },
        },
    )
    .unwrap();
    let snapshot = PrivateRoomSnapshot::new(catalog, instance, room).unwrap();
    match parent {
        Some(parent) => snapshot.under(parent),
        None => snapshot,
    }
}

fn principal(principal_id: PrincipalId) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id,
        matrix_user_id: format!("@user-{}:matrix.test", principal_id.as_uuid()),
        display_name: "Someone".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(NOW - 1_000),
        expires_at: time(NOW + 60_000),
        recently_authenticated: false,
    }
}

fn owner() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(1))
}

fn manager() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(2))
}

fn member() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(3))
}

fn stranger() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(4))
}

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(10))
}

fn parent_catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(11))
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}

struct TestClock;

impl Clock for TestClock {
    fn now(&self) -> UtcMillis {
        time(NOW)
    }
}

struct MemoryRooms {
    room: PrivateRoom,
    parent: Option<RoomCatalogId>,
}

impl PrivateRoomStore for MemoryRooms {
    fn create<'a>(
        &'a self,
        _snapshot: &'a PrivateRoomSnapshot,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("实况不建房间") })
    }

    fn find_by_catalog(
        &self,
        catalog: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        let found = (catalog == catalog_id()).then(|| snapshot(self.room.clone(), self.parent));
        Box::pin(async move { Ok(found) })
    }

    fn list_for_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("实况不列房间") })
    }

    fn find_by_matrix_room<'a>(
        &'a self,
        _matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("实况按目录找房间") })
    }

    fn save<'a>(
        &'a self,
        _room: &'a PrivateRoom,
        _expected_version: AggregateVersion,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("实况不改成员") })
    }

    fn rename<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        _name: &'a str,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("实况不改名") })
    }
}
