use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        Clock, IdentifierFactory, MatrixResult, MatrixRoomId, MatrixUserId, PortFuture,
        PrivateMatrixMembership, PrivateMatrixRoomCreation, PrivateMatrixSpeakingAssignment,
        PrivateRoomMatrixGateway, PrivateRoomMatrixProvisioner, PrivateRoomPrincipalDirectory,
        PrivateRoomSnapshot, PrivateRoomStore,
    },
    private_rooms::{
        ArchivePrivateRoom, ChangePrivateRoomPermissions, CreatePrivateRoom,
        GovernPrivateRoomMember, ListPrivateRooms, PrivateRoomDependencies, PrivateRoomFailureKind,
        PrivateRoomInvitation, PrivateRoomMembershipAction, PrivateRoomService,
        PrivateRoomUseCases, TransferPrivateRoomOwnership,
    },
};
use agent_room_domain::{
    ids::{
        AdapterBindingId, AgentCardSnapshotId, AgentId, AgentInstanceId, AutomationGrantId,
        ContentId, DeviceAccessTokenId, DeviceId, DeviceRefreshTokenId, DeviceTokenFamilyId,
        HandoffId, LoginAttemptId, OutboxEventId, PrincipalId, RoomCatalogId, RoomInstanceId,
        RoomReservationId, WebSessionId,
    },
    private_rooms::{PrivateRoomCapability, PrivateRoomMembershipStatus, PrivateRoomPermissions},
    rooms::MatrixRoomReference,
    time::UtcMillis,
    version::AggregateVersion,
};
use uuid::Uuid;

#[tokio::test]
async fn 创建完成前先合并初始发言硬边界再发布真实房间() {
    let fixture = Fixture::new();
    let request = fixture.creation_request(speaker_permissions());

    let snapshot = fixture.service.create(request).await.expect("创建应成功");

    assert_eq!(
        fixture.events(),
        vec!["matrix.create", "matrix.speak.batch", "store.create"]
    );
    assert_eq!(snapshot.catalog().name(), "Architecture room");
    assert_eq!(
        snapshot
            .room()
            .member(fixture.member)
            .expect("受邀成员存在")
            .status(),
        PrivateRoomMembershipStatus::Invited
    );
    let creation = fixture.matrix.creation().expect("应记录真实建房请求");
    assert_eq!(creation.request().invite().len(), 3);
    assert_eq!(
        fixture.matrix.speaking_allowed("@owner:matrix.test"),
        Some(true)
    );
    assert_eq!(
        fixture.matrix.speaking_allowed("@member:matrix.test"),
        Some(true)
    );
    assert_eq!(
        fixture
            .matrix
            .speaking_allowed("@content-authority:matrix.test"),
        Some(false)
    );
}

#[tokio::test]
async fn 列表只返回当前主体受邀或已加入的权威房间() {
    let fixture = Fixture::new();
    fixture
        .service
        .create(fixture.creation_request(speaker_permissions()))
        .await
        .expect("创建应成功");

    let invited = fixture
        .service
        .list(ListPrivateRooms {
            actor: actor(fixture.member, "@member:matrix.test"),
        })
        .await
        .expect("受邀成员可列出房间");
    assert_eq!(invited.len(), 1);

    fixture
        .matrix
        .set_membership(&Fixture::member_matrix(), PrivateMatrixMembership::Left);
    fixture
        .service
        .decline(fixture.member_action())
        .await
        .expect("拒绝邀请应成功");
    let declined = fixture
        .service
        .list(ListPrivateRooms {
            actor: actor(fixture.member, "@member:matrix.test"),
        })
        .await
        .expect("拒绝后的列表应可读");
    assert!(declined.is_empty());
}

#[tokio::test]
async fn 接受邀请必须先由真实_matrix_客户端完成加入() {
    let fixture = Fixture::new();
    fixture
        .service
        .create(fixture.creation_request(speaker_permissions()))
        .await
        .expect("创建应成功");
    fixture.clear_events();

    let rejected = fixture
        .service
        .accept(fixture.member_action())
        .await
        .expect_err("只有 Matrix 邀请态时不能伪装已加入");
    assert_eq!(rejected.kind(), PrivateRoomFailureKind::Conflict);
    assert_eq!(fixture.events(), vec!["matrix.membership"]);
    assert_eq!(
        fixture.store.member_status(fixture.member),
        PrivateRoomMembershipStatus::Invited
    );

    fixture
        .matrix
        .set_membership(&Fixture::member_matrix(), PrivateMatrixMembership::Joined);
    fixture.clear_events();
    let accepted = fixture
        .service
        .accept(fixture.member_action())
        .await
        .expect("Matrix 已加入后可接受");
    assert_eq!(fixture.events(), vec!["matrix.membership", "store.save"]);
    assert_eq!(
        accepted
            .room()
            .member(fixture.member)
            .expect("成员存在")
            .status(),
        PrivateRoomMembershipStatus::Joined
    );
}

#[tokio::test]
async fn 移除先收紧_matrix_再保存且数据库失败不会重新放开访问() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    fixture.store.fail_next_save.store(true, Ordering::SeqCst);
    fixture.clear_events();

    let failure = fixture
        .service
        .remove(GovernPrivateRoomMember {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
        })
        .await
        .expect_err("模拟持久化失败");

    assert_eq!(
        failure.kind(),
        PrivateRoomFailureKind::DependencyUnavailable
    );
    assert_eq!(fixture.events(), vec!["matrix.kick", "store.save"]);
    assert_eq!(
        fixture.matrix.membership_of(&Fixture::member_matrix()),
        Some(PrivateMatrixMembership::Left)
    );
    assert_eq!(
        fixture.store.member_status(fixture.member),
        PrivateRoomMembershipStatus::Joined
    );

    fixture.clear_events();
    let removed = fixture
        .service
        .remove(GovernPrivateRoomMember {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
        })
        .await
        .expect("重试应收敛产品状态");
    assert_eq!(fixture.events(), vec!["matrix.kick", "store.save"]);
    assert_eq!(
        removed
            .room()
            .member(fixture.member)
            .expect("历史成员存在")
            .status(),
        PrivateRoomMembershipStatus::Removed
    );
}

#[tokio::test]
async fn 权限降级先写_matrix_而升级先写产品事实() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    fixture.clear_events();

    fixture
        .service
        .update_permissions(ChangePrivateRoomPermissions {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
            permissions: viewer_permissions(),
        })
        .await
        .expect("降级应成功");
    assert_eq!(fixture.events(), vec!["matrix.speak.off", "store.save"]);

    fixture.clear_events();
    fixture
        .service
        .update_permissions(ChangePrivateRoomPermissions {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
            target_principal_id: fixture.member,
            permissions: speaker_permissions(),
        })
        .await
        .expect("升级应成功");
    assert_eq!(fixture.events(), vec!["store.save", "matrix.speak.on"]);
}

#[tokio::test]
async fn 房主转移支持响应丢失后的幂等协议对账() {
    let fixture = Fixture::joined(viewer_permissions()).await;
    let request = TransferPrivateRoomOwnership {
        actor: fixture.owner_actor(),
        catalog_id: fixture.catalog,
        target_principal_id: fixture.member,
        former_owner_permissions: viewer_permissions(),
    };
    fixture.clear_events();

    let transferred = fixture
        .service
        .transfer_ownership(request.clone())
        .await
        .expect("转移应成功");
    assert_eq!(
        fixture.events(),
        vec!["matrix.speak.off", "store.save", "matrix.speak.on"]
    );
    assert_eq!(transferred.room().owner_principal_id(), fixture.member);

    fixture.clear_events();
    let reconciled = fixture
        .service
        .transfer_ownership(request)
        .await
        .expect("响应丢失后的同请求应完成对账");
    assert_eq!(
        fixture.events(),
        vec!["matrix.speak.on", "matrix.speak.off"]
    );
    assert_eq!(reconciled.room().owner_principal_id(), fixture.member);
}

#[tokio::test]
async fn 归档先锁死_matrix_消息再原子归档产品状态() {
    let fixture = Fixture::joined(speaker_permissions()).await;
    fixture.clear_events();

    let archived = fixture
        .service
        .archive(ArchivePrivateRoom {
            actor: fixture.owner_actor(),
            catalog_id: fixture.catalog,
        })
        .await
        .expect("归档应成功");

    assert_eq!(fixture.events(), vec!["matrix.archive", "store.save"]);
    assert_eq!(archived.catalog().status().as_str(), "archived");
    assert_eq!(archived.instance().state().as_str(), "archived");
}

struct Fixture {
    service: PrivateRoomService,
    store: Arc<TestStore>,
    matrix: Arc<TestMatrix>,
    events: Arc<Mutex<Vec<&'static str>>>,
    catalog: RoomCatalogId,
    owner: PrincipalId,
    member: PrincipalId,
}

impl Fixture {
    fn new() -> Self {
        let events = Arc::new(Mutex::new(Vec::new()));
        let store = Arc::new(TestStore::new(events.clone()));
        let matrix = Arc::new(TestMatrix::new(events.clone()));
        let owner = principal_id(1);
        let member = principal_id(2);
        let principals = Arc::new(TestPrincipalDirectory::new(BTreeMap::from([
            (
                owner,
                MatrixUserId::new("@owner:matrix.test").expect("用户有效"),
            ),
            (
                member,
                MatrixUserId::new("@member:matrix.test").expect("用户有效"),
            ),
        ])));
        let runtime = Arc::new(TestRuntime);
        let service = PrivateRoomService::new(PrivateRoomDependencies {
            store: store.clone(),
            matrix_provisioner: matrix.clone(),
            matrix: matrix.clone(),
            principals,
            trusted_matrix_readers: vec![
                MatrixUserId::new("@content-authority:matrix.test").expect("服务身份有效"),
            ],
            identifiers: runtime.clone(),
            clock: runtime,
        });
        Self {
            service,
            store,
            matrix,
            events,
            catalog: catalog_id(10),
            owner,
            member,
        }
    }

    async fn joined(permissions: PrivateRoomPermissions) -> Self {
        let fixture = Self::new();
        fixture
            .service
            .create(fixture.creation_request(permissions))
            .await
            .expect("创建应成功");
        fixture
            .matrix
            .set_membership(&Self::member_matrix(), PrivateMatrixMembership::Joined);
        fixture
            .service
            .accept(fixture.member_action())
            .await
            .expect("接受应成功");
        fixture
    }

    fn creation_request(&self, permissions: PrivateRoomPermissions) -> CreatePrivateRoom {
        CreatePrivateRoom {
            actor: self.owner_actor(),
            catalog_id: self.catalog,
            name: "Architecture room".to_owned(),
            description: "Private design review".to_owned(),
            retention_days: Some(30),
            invitations: vec![PrivateRoomInvitation {
                principal_id: self.member,
                permissions,
            }],
        }
    }

    fn member_action(&self) -> PrivateRoomMembershipAction {
        PrivateRoomMembershipAction {
            actor: actor(self.member, "@member:matrix.test"),
            catalog_id: self.catalog,
        }
    }

    fn owner_actor(&self) -> AuthenticatedPrincipal {
        actor(self.owner, "@owner:matrix.test")
    }

    fn member_matrix() -> MatrixUserId {
        MatrixUserId::new("@member:matrix.test").expect("用户有效")
    }

    fn clear_events(&self) {
        self.events.lock().expect("事件锁正常").clear();
    }

    fn events(&self) -> Vec<&'static str> {
        self.events.lock().expect("事件锁正常").clone()
    }
}

struct TestStore {
    snapshot: Mutex<Option<PrivateRoomSnapshot>>,
    events: Arc<Mutex<Vec<&'static str>>>,
    fail_next_save: AtomicBool,
}

impl TestStore {
    fn new(events: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            snapshot: Mutex::new(None),
            events,
            fail_next_save: AtomicBool::new(false),
        }
    }

    fn member_status(&self, principal_id: PrincipalId) -> PrivateRoomMembershipStatus {
        self.snapshot
            .lock()
            .expect("快照锁正常")
            .as_ref()
            .and_then(|snapshot| snapshot.room().member(principal_id))
            .expect("成员存在")
            .status()
    }
}

impl PrivateRoomStore for TestStore {
    fn create<'a>(
        &'a self,
        snapshot: &'a PrivateRoomSnapshot,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            self.events.lock().expect("事件锁正常").push("store.create");
            let mut stored = self.snapshot.lock().expect("快照锁正常");
            if stored.is_some() {
                return Err(repository_error(
                    "test.create",
                    RepositoryErrorKind::Conflict,
                ));
            }
            *stored = Some(snapshot.clone());
            Ok(())
        })
    }

    fn find_by_catalog(
        &self,
        catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async move {
            Ok(self
                .snapshot
                .lock()
                .expect("快照锁正常")
                .as_ref()
                .filter(|snapshot| snapshot.catalog().id() == catalog_id)
                .cloned())
        })
    }

    fn list_for_principal(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomSnapshot>>> {
        Box::pin(async move {
            Ok(self
                .snapshot
                .lock()
                .expect("快照锁正常")
                .as_ref()
                .filter(|snapshot| {
                    snapshot.room().member(principal_id).is_some_and(|member| {
                        matches!(
                            member.status(),
                            PrivateRoomMembershipStatus::Invited
                                | PrivateRoomMembershipStatus::Joined
                        )
                    })
                })
                .cloned()
                .into_iter()
                .collect())
        })
    }

    fn find_by_matrix_room<'a>(
        &'a self,
        matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async move {
            Ok(self
                .snapshot
                .lock()
                .expect("快照锁正常")
                .as_ref()
                .filter(|snapshot| snapshot.instance().matrix_room_id() == matrix_room_id)
                .cloned())
        })
    }

    fn save<'a>(
        &'a self,
        room: &'a agent_room_domain::private_rooms::PrivateRoom,
        expected_version: AggregateVersion,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            self.events.lock().expect("事件锁正常").push("store.save");
            if self.fail_next_save.swap(false, Ordering::SeqCst) {
                return Err(repository_error(
                    "test.save",
                    RepositoryErrorKind::Unavailable,
                ));
            }
            let mut stored = self.snapshot.lock().expect("快照锁正常");
            let current = stored
                .as_ref()
                .ok_or_else(|| repository_error("test.save", RepositoryErrorKind::NotFound))?;
            if current.room().version() != expected_version {
                return Err(repository_error("test.save", RepositoryErrorKind::Conflict));
            }
            *stored = Some(
                current
                    .clone()
                    .replacing_room(room.clone())
                    .expect("测试快照应一致"),
            );
            Ok(())
        })
    }
}

struct TestMatrix {
    events: Arc<Mutex<Vec<&'static str>>>,
    creation: Mutex<Option<PrivateMatrixRoomCreation>>,
    memberships: Mutex<BTreeMap<String, PrivateMatrixMembership>>,
    speaking: Mutex<BTreeMap<String, bool>>,
}

impl TestMatrix {
    fn new(events: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            events,
            creation: Mutex::new(None),
            memberships: Mutex::new(BTreeMap::new()),
            speaking: Mutex::new(BTreeMap::new()),
        }
    }

    fn creation(&self) -> Option<PrivateMatrixRoomCreation> {
        self.creation.lock().expect("建房锁正常").clone()
    }

    fn set_membership(&self, user_id: &MatrixUserId, membership: PrivateMatrixMembership) {
        self.memberships
            .lock()
            .expect("成员锁正常")
            .insert(user_id.as_str().to_owned(), membership);
    }

    fn membership_of(&self, user_id: &MatrixUserId) -> Option<PrivateMatrixMembership> {
        self.memberships
            .lock()
            .expect("成员锁正常")
            .get(user_id.as_str())
            .copied()
    }

    fn record(&self, event: &'static str) {
        self.events.lock().expect("事件锁正常").push(event);
    }

    fn speaking_allowed(&self, user_id: &str) -> Option<bool> {
        self.speaking
            .lock()
            .expect("发言锁正常")
            .get(user_id)
            .copied()
    }
}

impl PrivateRoomMatrixProvisioner for TestMatrix {
    fn create<'a>(
        &'a self,
        creation: &'a PrivateMatrixRoomCreation,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        Box::pin(async move {
            self.record("matrix.create");
            creation.request().invite().iter().for_each(|user_id| {
                self.set_membership(user_id, PrivateMatrixMembership::Invited);
            });
            *self.creation.lock().expect("建房锁正常") = Some(creation.clone());
            Ok(MatrixRoomId::new("!private:matrix.test").expect("房间有效"))
        })
    }
}

impl PrivateRoomMatrixGateway for TestMatrix {
    fn membership<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<Option<PrivateMatrixMembership>>> {
        Box::pin(async move {
            self.record("matrix.membership");
            Ok(self.membership_of(user_id))
        })
    }

    fn invite<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.record("matrix.invite");
            self.set_membership(user_id, PrivateMatrixMembership::Invited);
            Ok(())
        })
    }

    fn kick<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.record("matrix.kick");
            self.set_membership(user_id, PrivateMatrixMembership::Left);
            Ok(())
        })
    }

    fn ban<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.record("matrix.ban");
            self.set_membership(user_id, PrivateMatrixMembership::Banned);
            Ok(())
        })
    }

    fn set_speaking<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
        allowed: bool,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.record(if allowed {
                "matrix.speak.on"
            } else {
                "matrix.speak.off"
            });
            self.speaking
                .lock()
                .expect("发言锁正常")
                .insert(user_id.as_str().to_owned(), allowed);
            Ok(())
        })
    }

    fn set_speaking_batch<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        assignments: &'a [PrivateMatrixSpeakingAssignment],
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.record("matrix.speak.batch");
            let mut speaking = self.speaking.lock().expect("发言锁正常");
            for assignment in assignments {
                speaking.insert(
                    assignment.user_id().as_str().to_owned(),
                    assignment.allowed(),
                );
            }
            Ok(())
        })
    }

    fn archive<'a>(&'a self, _room_id: &'a MatrixRoomId) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.record("matrix.archive");
            Ok(())
        })
    }
}

struct TestPrincipalDirectory {
    users: BTreeMap<PrincipalId, MatrixUserId>,
}

impl TestPrincipalDirectory {
    fn new(users: BTreeMap<PrincipalId, MatrixUserId>) -> Self {
        Self { users }
    }
}

impl PrivateRoomPrincipalDirectory for TestPrincipalDirectory {
    fn matrix_user_id(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Option<MatrixUserId>>> {
        Box::pin(async move { Ok(self.users.get(&principal_id).cloned()) })
    }
}

struct TestRuntime;

impl Clock for TestRuntime {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(1_000).expect("时间有效")
    }
}

impl IdentifierFactory for TestRuntime {
    fn principal_id(&self) -> PrincipalId {
        principal_id(101)
    }
    fn login_attempt_id(&self) -> LoginAttemptId {
        LoginAttemptId::from_uuid(uuid(102))
    }
    fn web_session_id(&self) -> WebSessionId {
        WebSessionId::from_uuid(uuid(103))
    }
    fn device_id(&self) -> DeviceId {
        DeviceId::from_uuid(uuid(104))
    }
    fn device_token_family_id(&self) -> DeviceTokenFamilyId {
        DeviceTokenFamilyId::from_uuid(uuid(105))
    }
    fn device_access_token_id(&self) -> DeviceAccessTokenId {
        DeviceAccessTokenId::from_uuid(uuid(106))
    }
    fn device_refresh_token_id(&self) -> DeviceRefreshTokenId {
        DeviceRefreshTokenId::from_uuid(uuid(107))
    }
    fn agent_id(&self) -> AgentId {
        AgentId::from_uuid(uuid(108))
    }
    fn agent_card_snapshot_id(&self) -> AgentCardSnapshotId {
        AgentCardSnapshotId::from_uuid(uuid(109))
    }
    fn adapter_binding_id(&self) -> AdapterBindingId {
        AdapterBindingId::from_uuid(uuid(110))
    }
    fn agent_instance_id(&self) -> AgentInstanceId {
        AgentInstanceId::from_uuid(uuid(111))
    }
    fn room_catalog_id(&self) -> RoomCatalogId {
        catalog_id(112)
    }
    fn room_instance_id(&self) -> RoomInstanceId {
        RoomInstanceId::from_uuid(uuid(113))
    }
    fn room_reservation_id(&self) -> RoomReservationId {
        RoomReservationId::from_uuid(uuid(114))
    }
    fn content_id(&self) -> ContentId {
        ContentId::from_uuid(uuid(115))
    }
    fn handoff_id(&self) -> HandoffId {
        HandoffId::from_uuid(uuid(116))
    }
    fn automation_grant_id(&self) -> AutomationGrantId {
        AutomationGrantId::from_uuid(uuid(117))
    }
    fn outbox_event_id(&self) -> OutboxEventId {
        OutboxEventId::from_uuid(uuid(118))
    }
}

fn actor(principal_id: PrincipalId, matrix_user_id: &str) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id,
        matrix_user_id: matrix_user_id.to_owned(),
        display_name: "Test user".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: UtcMillis::new(100).expect("时间有效"),
        expires_at: UtcMillis::new(2_000).expect("时间有效"),
        recently_authenticated: true,
    }
}

fn viewer_permissions() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([PrivateRoomCapability::View]).expect("查看权限有效")
}

fn speaker_permissions() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([
        PrivateRoomCapability::View,
        PrivateRoomCapability::Speak,
    ])
    .expect("发言权限有效")
}

fn principal_id(value: u128) -> PrincipalId {
    PrincipalId::from_uuid(uuid(value))
}

fn catalog_id(value: u128) -> RoomCatalogId {
    RoomCatalogId::from_uuid(uuid(value))
}

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn repository_error(operation: &'static str, kind: RepositoryErrorKind) -> RepositoryError {
    RepositoryError::new(operation, kind)
}
