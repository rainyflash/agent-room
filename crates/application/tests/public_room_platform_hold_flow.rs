//! 平台把公开房间从目录里拿掉（`specs/public-rooms/design.md` 的 3b-4b）：只有平台治理的人，要最近
//! 登录过；拿掉时作废在等的申请；原因去掉首尾空白，太长、带控制字符的不收；没有的、归档的房间说没有；
//! 放回去本来就没拿掉的也算成。

use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        Clock, JoinRequestCreateOutcome, JoinRequestRecord, JoinRequestSweep, ModerationAuthority,
        ModerationRoomContext, PendingJoinRequest, PortFuture, PrivateRoomJoinRequestStore,
        PrivateRoomSnapshot, PrivateRoomStore, PublicRoomPlatformHold, PublicRoomPlatformHoldStore,
    },
    private_rooms::{
        HoldPublicRoom, MAX_PLATFORM_HOLD_REASON_CHARS, PlatformHoldFailureKind,
        PublicRoomPlatformHoldDependencies, PublicRoomPlatformHoldService,
        PublicRoomPlatformHoldUseCases, ReleasePublicRoom,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId, RoomInstanceId},
    moderation::{ModerationRole, ModerationTarget},
    private_rooms::PrivateRoom,
    room_join_requests::JoinRequestStatus,
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

#[tokio::test]
async fn 平台治理的人拿掉房间_作废在等的申请_原因去掉首尾空白() {
    let fixture = Fixture::new(PrivateRoom::create(catalog_id(), owner()));

    let hold = fixture
        .service
        .hold(hold_as(moderator(), "  冒充官方客服\n骗人转账  "))
        .await
        .expect("平台治理的人能拿掉");

    let expected = PublicRoomPlatformHold {
        catalog_id: catalog_id(),
        held_by: moderator(),
        reason: "冒充官方客服\n骗人转账".to_owned(),
        held_at: time(NOW),
    };
    assert_eq!(hold, expected);
    assert_eq!(*fixture.holds.0.lock().unwrap(), Some(expected));
    assert_eq!(
        *fixture.voids.0.lock().unwrap(),
        [(catalog_id(), time(NOW))]
    );

    fixture
        .service
        .release(release_as(moderator()))
        .await
        .expect("放回去");
    assert_eq!(*fixture.holds.0.lock().unwrap(), None);
    fixture
        .service
        .release(release_as(moderator()))
        .await
        .expect("本来就没拿掉的也算放回去了");
}

#[tokio::test]
async fn 不是平台治理的人_房主自己_没最近登录过_登录过期的都不行() {
    let fixture = Fixture::new(PrivateRoom::create(catalog_id(), owner()));
    let stale = AuthenticatedPrincipal {
        recently_authenticated: false,
        ..principal(moderator())
    };
    let expired = AuthenticatedPrincipal {
        expires_at: time(NOW),
        ..principal(moderator())
    };
    for (actor, expected) in [
        (principal(owner()), PlatformHoldFailureKind::Forbidden),
        (principal(auditor()), PlatformHoldFailureKind::Forbidden),
        (stale, PlatformHoldFailureKind::ReauthenticationRequired),
        (expired, PlatformHoldFailureKind::Forbidden),
    ] {
        let hold = fixture
            .service
            .hold(HoldPublicRoom {
                actor: actor.clone(),
                catalog_id: catalog_id(),
                reason: String::new(),
            })
            .await
            .unwrap_err();
        assert_eq!(hold.kind(), expected, "{actor:?}");
        let release = fixture
            .service
            .release(ReleasePublicRoom {
                actor: actor.clone(),
                catalog_id: catalog_id(),
            })
            .await
            .unwrap_err();
        assert_eq!(release.kind(), expected, "{actor:?}");
    }
    assert_eq!(*fixture.holds.0.lock().unwrap(), None);
    assert!(fixture.voids.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 原因太长_带控制字符的不收_没有的_归档的房间说没有() {
    let fixture = Fixture::new(PrivateRoom::create(catalog_id(), owner()));
    let too_long = "长".repeat(MAX_PLATFORM_HOLD_REASON_CHARS + 1);
    for reason in [too_long.as_str(), "带\r回车", "带\u{7}响铃"] {
        let failure = fixture
            .service
            .hold(hold_as(moderator(), reason))
            .await
            .unwrap_err();
        assert_eq!(failure.kind(), PlatformHoldFailureKind::InvalidRequest);
    }
    let exactly = "长".repeat(MAX_PLATFORM_HOLD_REASON_CHARS);
    fixture
        .service
        .hold(hold_as(moderator(), &exactly))
        .await
        .expect("正好 500 个字收");

    let missing = fixture
        .service
        .hold(HoldPublicRoom {
            catalog_id: RoomCatalogId::from_uuid(Uuid::from_u128(99)),
            ..hold_as(moderator(), "")
        })
        .await
        .unwrap_err();
    assert_eq!(missing.kind(), PlatformHoldFailureKind::NotFound);

    let mut archived_room = PrivateRoom::create(catalog_id(), owner());
    archived_room.archive(owner()).unwrap();
    let archived = Fixture::new(archived_room);
    let failure = archived
        .service
        .hold(hold_as(moderator(), ""))
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), PlatformHoldFailureKind::NotFound);
    assert!(archived.voids.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 存不上就不作废申请_库连不上说暂时不行() {
    let fixture = Fixture::new(PrivateRoom::create(catalog_id(), owner()));
    *fixture.holds.1.lock().unwrap() = true;
    let failure = fixture
        .service
        .hold(hold_as(moderator(), ""))
        .await
        .unwrap_err();
    assert_eq!(
        failure.kind(),
        PlatformHoldFailureKind::DependencyUnavailable
    );
    assert!(fixture.voids.0.lock().unwrap().is_empty());
}

struct Fixture {
    service: PublicRoomPlatformHoldService,
    holds: Arc<MemoryHolds>,
    voids: Arc<RecordingVoids>,
}

impl Fixture {
    fn new(room: PrivateRoom) -> Self {
        let holds = Arc::new(MemoryHolds::default());
        let voids = Arc::new(RecordingVoids::default());
        let service = PublicRoomPlatformHoldService::new(PublicRoomPlatformHoldDependencies {
            rooms: Arc::new(MemoryRooms(room)),
            holds: holds.clone(),
            authority: Arc::new(Roles),
            requests: voids.clone(),
            clock: Arc::new(TestClock),
        });
        Self {
            service,
            holds,
            voids,
        }
    }
}

fn hold_as(actor: PrincipalId, reason: &str) -> HoldPublicRoom {
    HoldPublicRoom {
        actor: principal(actor),
        catalog_id: catalog_id(),
        reason: reason.to_owned(),
    }
}

fn release_as(actor: PrincipalId) -> ReleasePublicRoom {
    ReleasePublicRoom {
        actor: principal(actor),
        catalog_id: catalog_id(),
    }
}

fn snapshot(room: PrivateRoom) -> PrivateRoomSnapshot {
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
            projected_member_count: 0,
            allocated_slots: 0,
            activity_score_millis: 0,
            state: if archived {
                RoomInstanceState::Archived
            } else {
                RoomInstanceState::Active
            },
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
        expires_at: time(NOW + 60_000),
        recently_authenticated: true,
    }
}

fn owner() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(1))
}

fn moderator() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(7))
}

/// 只能看审计，不能动手。
fn auditor() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(8))
}

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(10))
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

/// `moderator()` 是平台治理的人，`auditor()` 只能看审计，别人什么都不是。
struct Roles;

impl ModerationAuthority for Roles {
    fn may_report<'a>(
        &'a self,
        _principal_id: PrincipalId,
        _target: &'a ModerationTarget,
        _room_catalog_id: Option<RoomCatalogId>,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        Box::pin(async { unreachable!("拿掉房间不举报") })
    }

    fn inspect_room<'a>(
        &'a self,
        _principal_id: PrincipalId,
        _room_catalog_id: RoomCatalogId,
        _target: &'a ModerationTarget,
    ) -> PortFuture<'a, RepositoryResult<Option<ModerationRoomContext>>> {
        Box::pin(async { unreachable!("拿掉房间只看平台的角色") })
    }

    fn platform_role(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<ModerationRole>> {
        let role = if principal_id == moderator() {
            ModerationRole::PlatformModerator
        } else if principal_id == auditor() {
            ModerationRole::AuditReader
        } else {
            ModerationRole::None
        };
        Box::pin(async move { Ok(role) })
    }
}

/// 拿掉的记在内存里；第二项设上时存不进去。
#[derive(Default)]
struct MemoryHolds(Mutex<Option<PublicRoomPlatformHold>>, Mutex<bool>);

impl PublicRoomPlatformHoldStore for MemoryHolds {
    fn platform_hold(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PublicRoomPlatformHold>>> {
        let hold = self.0.lock().unwrap().clone();
        Box::pin(async move { Ok(hold) })
    }

    fn place_platform_hold<'a>(
        &'a self,
        hold: &'a PublicRoomPlatformHold,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        let result = if *self.1.lock().unwrap() {
            Err(RepositoryError::new(
                "public_room.platform_hold.place",
                RepositoryErrorKind::Unavailable,
            ))
        } else {
            *self.0.lock().unwrap() = Some(hold.clone());
            Ok(())
        };
        Box::pin(async move { result })
    }

    fn release_platform_hold(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        let released = self.0.lock().unwrap().take().is_some();
        Box::pin(async move { Ok(released) })
    }
}

/// 只记下作废了哪间房的申请。
#[derive(Default)]
struct RecordingVoids(Mutex<Vec<(RoomCatalogId, UtcMillis)>>);

impl PrivateRoomJoinRequestStore for RecordingVoids {
    fn pending_join_request(
        &self,
        _catalog_id: RoomCatalogId,
        _requester: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<JoinRequestRecord>>> {
        Box::pin(async { unreachable!("拿掉房间不看申请") })
    }

    fn last_rejected_at(
        &self,
        _catalog_id: RoomCatalogId,
        _requester: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<UtcMillis>>> {
        Box::pin(async { unreachable!("拿掉房间不看申请") })
    }

    fn create_join_request<'a>(
        &'a self,
        _record: &'a JoinRequestRecord,
    ) -> PortFuture<'a, RepositoryResult<JoinRequestCreateOutcome>> {
        Box::pin(async { unreachable!("拿掉房间不记申请") })
    }

    fn pending_join_requests(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PendingJoinRequest>>> {
        Box::pin(async { unreachable!("拿掉房间不看申请") })
    }

    fn decide_join_request(
        &self,
        _catalog_id: RoomCatalogId,
        _requester: PrincipalId,
        _status: JoinRequestStatus,
        _decided_by: Option<PrincipalId>,
        _decided_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        Box::pin(async { unreachable!("拿掉房间不处理申请") })
    }

    fn void_pending_join_requests(
        &self,
        catalog_id: RoomCatalogId,
        voided_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<u64>> {
        self.0.lock().unwrap().push((catalog_id, voided_at));
        Box::pin(async { Ok(0) })
    }

    fn sweep_join_requests(
        &self,
        _now: UtcMillis,
        _decided_before: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<JoinRequestSweep>> {
        Box::pin(async { unreachable!("拿掉房间不清理") })
    }
}

struct MemoryRooms(PrivateRoom);

impl PrivateRoomStore for MemoryRooms {
    fn create<'a>(
        &'a self,
        _snapshot: &'a PrivateRoomSnapshot,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("拿掉房间不建房间") })
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
        Box::pin(async { unreachable!("拿掉房间不列房间") })
    }

    fn find_by_matrix_room<'a>(
        &'a self,
        _matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("拿掉房间按目录找房间") })
    }

    fn save<'a>(
        &'a self,
        _room: &'a PrivateRoom,
        _expected_version: AggregateVersion,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("拿掉房间不改成员") })
    }

    fn rename<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        _name: &'a str,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("拿掉房间不改名") })
    }
}
