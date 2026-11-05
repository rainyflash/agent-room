//! 进公开房间（`specs/public-rooms/design.md`）：直接进的以房主的名义邀请、拿房主定的默认权限；
//! 要申请的、只能邀请的不邀请；不公开的、归档的、平台拿掉的和没有的一样回答；封禁的进不来；已经在
//! 里面的照实说。
//! 申请和审批在 `public_room_join_flow/requests.rs`。

use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    persistence::RepositoryResult,
    ports::{
        Clock, JoinRequestCreateOutcome, JoinRequestRecord, JoinRequestSweep, PendingJoinRequest,
        PortFuture, PrivateRoomJoinRequestStore, PrivateRoomListingRecord,
        PrivateRoomListingSaveOutcome, PrivateRoomListingStore, PrivateRoomSnapshot,
        PrivateRoomStore, PublicRoomPlatformHold, PublicRoomPlatformHoldStore,
    },
    private_rooms::{
        InviteByLink, JoinPublicRoom, PrivateRoomFailure, PrivateRoomFailureKind,
        PrivateRoomFailureStage, PrivateRoomLinkInvitations, PrivateRoomResult,
        PublicJoinFailureKind, PublicJoinMembership, PublicRoomJoinDependencies,
        PublicRoomJoinService, PublicRoomJoinUseCases,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId, RoomInstanceId},
    private_rooms::{
        PrivateRoom, PrivateRoomCapability, PrivateRoomLifecycleStatus, PrivateRoomPermissions,
    },
    room_join_requests::JoinRequestStatus,
    room_listing::{RoomJoinMode, RoomListingSettings, RoomShareCode},
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

#[path = "public_room_join_flow/requests.rs"]
mod requests;

#[tokio::test]
async fn 直接进的房间_以房主的名义邀请_给房主定的默认权限() {
    let fixture = Fixture::new(
        PrivateRoom::create(catalog_id(), owner()),
        Some(listing(true, RoomJoinMode::Open)),
    );
    let membership = fixture
        .service
        .join(join(stranger()))
        .await
        .expect("进得来");
    assert_eq!(membership, PublicJoinMembership::Invited);
    assert_eq!(
        fixture.invitations.calls(),
        [InviteByLink {
            catalog_id: catalog_id(),
            link_creator: owner(),
            target_principal_id: stranger(),
            permissions: speak_permissions(),
        }]
    );
}

#[tokio::test]
async fn 已经在里面的照实说_不再邀请_改回私密以后也一样() {
    let mut room = PrivateRoom::create(catalog_id(), owner());
    room.invite(owner(), stranger(), speak_permissions())
        .unwrap();
    room.invite(owner(), speaker(), speak_permissions())
        .unwrap();
    room.accept_invitation(speaker()).unwrap();
    let fixture = Fixture::new(room, Some(listing(false, RoomJoinMode::Open)));
    assert_eq!(
        fixture.service.join(join(stranger())).await.unwrap(),
        PublicJoinMembership::Invited
    );
    assert_eq!(
        fixture.service.join(join(speaker())).await.unwrap(),
        PublicJoinMembership::Joined
    );
    assert!(fixture.invitations.calls().is_empty());
}

#[tokio::test]
async fn 不公开的_归档的_没有的_都说没有这个房间() {
    let private = Fixture::new(PrivateRoom::create(catalog_id(), owner()), None);
    let unlisted = Fixture::new(
        PrivateRoom::create(catalog_id(), owner()),
        Some(listing(false, RoomJoinMode::Open)),
    );
    let mut archived_room = PrivateRoom::create(catalog_id(), owner());
    archived_room.archive(owner()).unwrap();
    let archived = Fixture::new(archived_room, Some(listing(true, RoomJoinMode::Open)));
    for fixture in [&private, &unlisted, &archived] {
        let failure = fixture.service.join(join(stranger())).await.unwrap_err();
        assert_eq!(failure.kind(), PublicJoinFailureKind::NotFound);
    }
    let missing = private
        .service
        .join(JoinPublicRoom {
            actor: principal(stranger()),
            catalog_id: RoomCatalogId::from_uuid(Uuid::from_u128(99)),
        })
        .await
        .unwrap_err();
    assert_eq!(missing.kind(), PublicJoinFailureKind::NotFound);
    for fixture in [&private, &unlisted, &archived] {
        assert!(fixture.invitations.calls().is_empty());
    }
}

#[tokio::test]
async fn 平台拿掉的房间进不来_放回去以后照房主的设置来() {
    let open = Fixture::new(
        PrivateRoom::create(catalog_id(), owner()),
        Some(listing(true, RoomJoinMode::Open)),
    );
    open.holds.hold();
    let failure = open.service.join(join(stranger())).await.unwrap_err();
    assert_eq!(failure.kind(), PublicJoinFailureKind::NotFound);
    assert!(open.invitations.calls().is_empty());

    open.holds.release();
    assert_eq!(
        open.service.join(join(stranger())).await.unwrap(),
        PublicJoinMembership::Invited,
        "放回去以后照房主的设置来"
    );
}

#[tokio::test]
async fn 要申请的_只能邀请的_封禁的_登录过期的都不邀请() {
    let request = Fixture::new(
        PrivateRoom::create(catalog_id(), owner()),
        Some(listing(true, RoomJoinMode::Request)),
    );
    let invite_only = Fixture::new(
        PrivateRoom::create(catalog_id(), owner()),
        Some(listing(true, RoomJoinMode::InviteOnly)),
    );
    let mut banned_room = PrivateRoom::create(catalog_id(), owner());
    banned_room
        .invite(owner(), stranger(), speak_permissions())
        .unwrap();
    banned_room.ban_member(owner(), stranger()).unwrap();
    let banned = Fixture::new(banned_room, Some(listing(true, RoomJoinMode::Open)));

    for (fixture, kind) in [
        (&request, PublicJoinFailureKind::ApprovalRequired),
        (&invite_only, PublicJoinFailureKind::InviteOnly),
        (&banned, PublicJoinFailureKind::Banned),
    ] {
        let failure = fixture.service.join(join(stranger())).await.unwrap_err();
        assert_eq!(failure.kind(), kind);
        assert!(fixture.invitations.calls().is_empty());
    }

    let open = Fixture::new(
        PrivateRoom::create(catalog_id(), owner()),
        Some(listing(true, RoomJoinMode::Open)),
    );
    let mut expired = principal(stranger());
    expired.expires_at = time(NOW);
    let failure = open
        .service
        .join(JoinPublicRoom {
            actor: expired,
            catalog_id: catalog_id(),
        })
        .await
        .unwrap_err();
    assert_eq!(failure.kind(), PublicJoinFailureKind::Forbidden);
    assert!(open.invitations.calls().is_empty());
}

#[tokio::test]
async fn 邀请那一步没成_按原因回答() {
    for (kind, expected) in [
        (
            PrivateRoomFailureKind::Conflict,
            PublicJoinFailureKind::Conflict,
        ),
        (
            PrivateRoomFailureKind::Forbidden,
            PublicJoinFailureKind::Conflict,
        ),
        (
            PrivateRoomFailureKind::DependencyUnavailable,
            PublicJoinFailureKind::DependencyUnavailable,
        ),
        (
            PrivateRoomFailureKind::Internal,
            PublicJoinFailureKind::Internal,
        ),
    ] {
        let fixture = Fixture::new(
            PrivateRoom::create(catalog_id(), owner()),
            Some(listing(true, RoomJoinMode::Open)),
        );
        *fixture.invitations.failure.lock().unwrap() = Some(kind);
        let failure = fixture.service.join(join(stranger())).await.unwrap_err();
        assert_eq!(failure.kind(), expected, "{kind:?}");
    }
}

struct Fixture {
    service: PublicRoomJoinService,
    invitations: Arc<RecordingInvitations>,
    requests: Arc<MemoryRequests>,
    holds: Arc<MemoryHolds>,
}

impl Fixture {
    fn new(room: PrivateRoom, listing: Option<PrivateRoomListingRecord>) -> Self {
        let invitations = Arc::new(RecordingInvitations::default());
        let requests = Arc::new(MemoryRequests::default());
        let holds = Arc::new(MemoryHolds::default());
        let service = PublicRoomJoinService::new(PublicRoomJoinDependencies {
            rooms: Arc::new(MemoryRooms(room)),
            listings: Arc::new(MemoryListings(listing)),
            requests: requests.clone(),
            invitations: invitations.clone(),
            holds: holds.clone(),
            clock: Arc::new(TestClock),
        });
        Self {
            service,
            invitations,
            requests,
            holds,
        }
    }
}

fn join(actor: PrincipalId) -> JoinPublicRoom {
    JoinPublicRoom {
        actor: principal(actor),
        catalog_id: catalog_id(),
    }
}

fn listing(listed: bool, join_mode: RoomJoinMode) -> PrivateRoomListingRecord {
    PrivateRoomListingRecord {
        catalog_id: catalog_id(),
        settings: RoomListingSettings::new(listed, join_mode, speak_permissions(), true).unwrap(),
        share_code: RoomShareCode::parse("Join01").unwrap(),
        first_listed_at: time(NOW - 10_000),
        updated_by: owner(),
        updated_at: time(NOW - 10_000),
        version: AggregateVersion::INITIAL.next().unwrap(),
    }
}

fn speak_permissions() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([
        PrivateRoomCapability::View,
        PrivateRoomCapability::Speak,
    ])
    .unwrap()
}

fn snapshot(room: PrivateRoom) -> PrivateRoomSnapshot {
    let archived = room.status() == PrivateRoomLifecycleStatus::Archived;
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
        recently_authenticated: false,
    }
}

fn person(sequence: u128) -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(sequence))
}

fn owner() -> PrincipalId {
    person(1)
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

struct TestClock;

impl Clock for TestClock {
    fn now(&self) -> UtcMillis {
        time(NOW)
    }
}

#[derive(Default)]
struct RecordingInvitations {
    calls: Mutex<Vec<InviteByLink>>,
    failure: Mutex<Option<PrivateRoomFailureKind>>,
}

impl RecordingInvitations {
    fn calls(&self) -> Vec<InviteByLink> {
        self.calls.lock().unwrap().clone()
    }
}

impl PrivateRoomLinkInvitations for RecordingInvitations {
    fn invite_by_link(
        &self,
        request: InviteByLink,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        self.calls.lock().unwrap().push(request);
        let failure = *self.failure.lock().unwrap();
        Box::pin(async move {
            match failure {
                Some(kind) => Err(PrivateRoomFailure::new(
                    "test",
                    PrivateRoomFailureStage::Matrix,
                    kind,
                )),
                // 回答的快照进门这一步用不上。
                None => Ok(snapshot(PrivateRoom::create(catalog_id(), owner()))),
            }
        })
    }
}

struct MemoryRooms(PrivateRoom);

impl PrivateRoomStore for MemoryRooms {
    fn create<'a>(
        &'a self,
        _snapshot: &'a PrivateRoomSnapshot,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("进房间不建房间") })
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
        Box::pin(async { unreachable!("进房间不列房间") })
    }

    fn find_by_matrix_room<'a>(
        &'a self,
        _matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("进房间按目录找房间") })
    }

    fn save<'a>(
        &'a self,
        _room: &'a PrivateRoom,
        _expected_version: AggregateVersion,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("进房间经邀请那一步存") })
    }

    fn rename<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        _name: &'a str,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("进房间不改名") })
    }
}

/// 平台拿掉没有。进房间只问有没有，原因和是谁不看。
#[derive(Default)]
struct MemoryHolds(Mutex<bool>);

impl MemoryHolds {
    fn hold(&self) {
        *self.0.lock().unwrap() = true;
    }

    fn release(&self) {
        *self.0.lock().unwrap() = false;
    }
}

impl PublicRoomPlatformHoldStore for MemoryHolds {
    fn platform_hold(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PublicRoomPlatformHold>>> {
        let hold = (*self.0.lock().unwrap()).then(|| PublicRoomPlatformHold {
            catalog_id,
            held_by: person(9),
            reason: String::new(),
            held_at: time(NOW - 1_000),
        });
        Box::pin(async move { Ok(hold) })
    }

    fn place_platform_hold<'a>(
        &'a self,
        _hold: &'a PublicRoomPlatformHold,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("进房间不拿掉房间") })
    }

    fn release_platform_hold(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        Box::pin(async { unreachable!("进房间不放回房间") })
    }
}

struct MemoryListings(Option<PrivateRoomListingRecord>);

impl PrivateRoomListingStore for MemoryListings {
    fn listing(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomListingRecord>>> {
        let record = self
            .0
            .clone()
            .filter(|record| record.catalog_id == catalog_id);
        Box::pin(async move { Ok(record) })
    }

    fn listed_rooms_owned_by(
        &self,
        _owner: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<RoomCatalogId>>> {
        Box::pin(async { unreachable!("进房间不查限额") })
    }

    fn save_listing<'a>(
        &'a self,
        _record: &'a PrivateRoomListingRecord,
        _expected: Option<AggregateVersion>,
        _owner: PrincipalId,
    ) -> PortFuture<'a, RepositoryResult<PrivateRoomListingSaveOutcome>> {
        Box::pin(async { unreachable!("进房间不改设置") })
    }

    fn create_listed_room<'a>(
        &'a self,
        _snapshot: &'a PrivateRoomSnapshot,
        _listing: &'a PrivateRoomListingRecord,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<PrivateRoomListingSaveOutcome>> {
        Box::pin(async { unreachable!("进房间不建房间") })
    }

    fn save_transferred_room<'a>(
        &'a self,
        _room: &'a PrivateRoom,
        _expected: AggregateVersion,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<PrivateRoomListingSaveOutcome>> {
        Box::pin(async { unreachable!("进房间不转让房主") })
    }

    fn mark_room_encrypted(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<()>> {
        Box::pin(async { unreachable!("进房间不改加密") })
    }
}

/// 申请记在内存里；申请人的昵称都是一个像邮箱的名字。`sweeps` 记下每次清理的参数。
#[derive(Default)]
struct MemoryRequests {
    records: Mutex<Vec<JoinRequestRecord>>,
    sweeps: Mutex<Vec<(UtcMillis, UtcMillis)>>,
}

impl MemoryRequests {
    fn records(&self) -> Vec<JoinRequestRecord> {
        self.records.lock().unwrap().clone()
    }

    fn seed(&self, record: JoinRequestRecord) {
        self.records.lock().unwrap().push(record);
    }

    fn pending(
        &self,
        catalog_id: RoomCatalogId,
        requester: PrincipalId,
    ) -> Option<JoinRequestRecord> {
        self.records
            .lock()
            .unwrap()
            .iter()
            .find(|record| {
                record.catalog_id == catalog_id
                    && record.requester == requester
                    && record.status == JoinRequestStatus::Pending
            })
            .cloned()
    }
}

impl PrivateRoomJoinRequestStore for MemoryRequests {
    fn pending_join_request(
        &self,
        catalog_id: RoomCatalogId,
        requester: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<JoinRequestRecord>>> {
        let found = self.pending(catalog_id, requester);
        Box::pin(async move { Ok(found) })
    }

    fn last_rejected_at(
        &self,
        catalog_id: RoomCatalogId,
        requester: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<UtcMillis>>> {
        let rejected_at = self
            .records
            .lock()
            .unwrap()
            .iter()
            .filter(|record| {
                record.catalog_id == catalog_id
                    && record.requester == requester
                    && record.status == JoinRequestStatus::Rejected
            })
            .filter_map(|record| record.decided_at)
            .max();
        Box::pin(async move { Ok(rejected_at) })
    }

    fn create_join_request<'a>(
        &'a self,
        record: &'a JoinRequestRecord,
    ) -> PortFuture<'a, RepositoryResult<JoinRequestCreateOutcome>> {
        let outcome = if let Some(existing) = self.pending(record.catalog_id, record.requester) {
            JoinRequestCreateOutcome::AlreadyPending(existing)
        } else {
            self.seed(record.clone());
            JoinRequestCreateOutcome::Created
        };
        Box::pin(async move { Ok(outcome) })
    }

    fn pending_join_requests(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PendingJoinRequest>>> {
        let pending = self
            .records()
            .into_iter()
            .filter(|record| {
                record.catalog_id == catalog_id && record.status == JoinRequestStatus::Pending
            })
            .map(|record| PendingJoinRequest {
                record,
                requester_display_name: "xiaolin@example.test".to_owned(),
            })
            .collect();
        Box::pin(async move { Ok(pending) })
    }

    fn decide_join_request(
        &self,
        catalog_id: RoomCatalogId,
        requester: PrincipalId,
        status: JoinRequestStatus,
        decided_by: Option<PrincipalId>,
        decided_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        let decided = self
            .records
            .lock()
            .unwrap()
            .iter_mut()
            .find(|record| {
                record.catalog_id == catalog_id
                    && record.requester == requester
                    && record.status == JoinRequestStatus::Pending
            })
            .map(|record| {
                record.status = status;
                record.decided_by = decided_by;
                record.decided_at = Some(decided_at.max(record.requested_at));
            })
            .is_some();
        Box::pin(async move { Ok(decided) })
    }

    fn void_pending_join_requests(
        &self,
        _catalog_id: RoomCatalogId,
        _voided_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<u64>> {
        Box::pin(async { unreachable!("改回私密时作废，进房间不作废") })
    }

    fn sweep_join_requests(
        &self,
        now: UtcMillis,
        decided_before: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<JoinRequestSweep>> {
        self.sweeps.lock().unwrap().push((now, decided_before));
        Box::pin(async { Ok(JoinRequestSweep::default()) })
    }
}
