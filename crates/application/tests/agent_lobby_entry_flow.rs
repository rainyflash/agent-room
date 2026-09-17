use std::sync::{Arc, Mutex};

use agent_room_application::{
    agent_lobbies::{
        AgentLobbyEntryDependencies, AgentLobbyEntryFailureKind, AgentLobbyEntryService,
        AgentLobbyEntryUseCases, EnterAgentLobby,
    },
    devices::AuthenticatedDevice,
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        AgentLobbyAccessRecord, AgentLobbyAccessRepository, AgentRoomMembershipFactory, Clock,
        MatrixResult, MatrixRoomId, MatrixUserId, PortFuture, PrincipalAccount,
        PrivateMatrixMembership, PrivateMatrixSpeakingAssignment, PrivateRoomMatrixGateway,
        PrivateRoomSnapshot, PrivateRoomStore, RoomAllocationMode, RoomAllocationStore,
        RoomMembershipGateway, RoomReservationClaim, RoomReservationOutcome,
    },
    rooms::{
        LobbyJoinPolicy, LobbyProvisioningOperation, LobbyProvisioningOutcome,
        LobbyProvisioningRequest, LobbyProvisioningResult, RoomReservationIdentifierFactory,
    },
};
use agent_room_domain::{
    identity::Principal,
    ids::{
        AgentId, AgentInstanceId, DeviceId, PrincipalId, RoomCatalogId, RoomInstanceId,
        RoomReservationId,
    },
    private_rooms::{PrivateRoom, PrivateRoomCapability, PrivateRoomPermissions},
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState, RoomReservation, RoomReservationFields, RoomReservationState,
    },
    time::{DurationMillis, UtcMillis},
    version::AggregateVersion,
};
use uuid::Uuid;

const NOW: i64 = 1_700_000_000_000;

struct 固定访问仓储(Option<AgentLobbyAccessRecord>);

impl AgentLobbyAccessRepository for 固定访问仓储 {
    fn find_public_lobby_room<'a>(
        &'a self,
        catalog_id: RoomCatalogId,
        matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<RoomInstanceId>>> {
        Box::pin(async move {
            Ok(
                (catalog_id == room().catalog_id() && matrix_room_id == room().matrix_room_id())
                    .then_some(room_instance_id()),
            )
        })
    }
    fn find_lobby_access(
        &self,
        _agent_instance_id: AgentInstanceId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentLobbyAccessRecord>>> {
        let value = self.0.clone();
        Box::pin(async move { Ok(value) })
    }
}

#[derive(Default)]
struct 记录成员能力 {
    joins: Mutex<Vec<String>>,
}

impl RoomMembershipGateway for 记录成员能力 {
    fn join<'a>(&'a self, room_id: &'a MatrixRoomReference) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.joins
                .lock()
                .expect("成员记录锁可用")
                .push(room_id.as_str().to_owned());
            Ok(())
        })
    }

    fn leave<'a>(&'a self, _room_id: &'a MatrixRoomReference) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move { Ok(()) })
    }
}

struct 记录成员工厂 {
    users: Mutex<Vec<MatrixUserId>>,
    membership: Arc<记录成员能力>,
}

impl AgentRoomMembershipFactory for 记录成员工厂 {
    fn bind(&self, matrix_user_id: &MatrixUserId) -> MatrixResult<Arc<dyn RoomMembershipGateway>> {
        self.users
            .lock()
            .expect("用户记录锁可用")
            .push(matrix_user_id.clone());
        Ok(self.membership.clone())
    }
}

struct 固定分配仓储 {
    reservation: RoomReservation,
    room: RoomInstance,
    expected_mode: RoomAllocationMode,
}

impl RoomAllocationStore for 固定分配仓储 {
    fn reserve<'a>(
        &'a self,
        claim: &'a RoomReservationClaim,
    ) -> PortFuture<'a, RepositoryResult<RoomReservationOutcome>> {
        Box::pin(async move {
            assert_eq!(claim.agent_id, agent_id());
            assert_eq!(claim.agent_instance_id, instance_id());
            assert_eq!(claim.mode, self.expected_mode);
            Ok(RoomReservationOutcome::ExistingAssignment {
                reservation: self.reservation.clone(),
                room: self.room.clone(),
            })
        })
    }

    fn transition(
        &self,
        _reservation_id: RoomReservationId,
        _expected: RoomReservationState,
        _target: RoomReservationState,
        _changed_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<RoomReservation>> {
        Box::pin(async move {
            Err(RepositoryError::new(
                "test.unexpected_transition",
                RepositoryErrorKind::Conflict,
            ))
        })
    }

    fn expire_pending(
        &self,
        _now: UtcMillis,
        _limit: u16,
    ) -> PortFuture<'_, RepositoryResult<u16>> {
        Box::pin(async move { Ok(0) })
    }
}

struct 禁止供给;

impl LobbyProvisioningOperation for 禁止供给 {
    fn provision(
        &self,
        _request: LobbyProvisioningRequest,
    ) -> PortFuture<'_, LobbyProvisioningResult<LobbyProvisioningOutcome>> {
        Box::pin(async move { panic!("已有分配时不得触发供给") })
    }
}

struct 固定运行时;

impl Clock for 固定运行时 {
    fn now(&self) -> UtcMillis {
        time(NOW)
    }
}

impl RoomReservationIdentifierFactory for 固定运行时 {
    fn room_reservation_id(&self) -> RoomReservationId {
        RoomReservationId::from_uuid(Uuid::from_u128(900))
    }
}

#[tokio::test]
async fn 只有实例所属设备能以权威_matrix_身份进入大厅() {
    let membership = Arc::new(记录成员能力::default());
    let factory = Arc::new(记录成员工厂 {
        users: Mutex::new(Vec::new()),
        membership: membership.clone(),
    });
    let service = service(access(true, device_id()), factory.clone());

    let outcome = service
        .enter(request(device_id()))
        .await
        .expect("实例所属设备应能进入大厅");

    assert!(matches!(
        outcome,
        agent_room_application::rooms::EnterLobbyOutcome::Joined { .. }
    ));
    assert_eq!(
        factory.users.lock().expect("用户记录锁可用").as_slice(),
        &[matrix_user_id()]
    );
    assert_eq!(
        membership.joins.lock().expect("成员记录锁可用").as_slice(),
        &["!lobby:matrix.test"]
    );
}

#[tokio::test]
async fn 其他设备不能借用_agent_实例进入大厅() {
    let membership = Arc::new(记录成员能力::default());
    let factory = Arc::new(记录成员工厂 {
        users: Mutex::new(Vec::new()),
        membership,
    });
    let service = service(access(true, device_id()), factory.clone());

    let failure = service
        .enter(request(DeviceId::from_uuid(Uuid::from_u128(999))))
        .await
        .expect_err("其他设备必须被拒绝");

    assert_eq!(failure.kind(), AgentLobbyEntryFailureKind::Unauthorized);
    assert!(factory.users.lock().expect("用户记录锁可用").is_empty());
}

#[tokio::test]
async fn 已失效实例在_matrix_调用前被拒绝() {
    let membership = Arc::new(记录成员能力::default());
    let factory = Arc::new(记录成员工厂 {
        users: Mutex::new(Vec::new()),
        membership,
    });
    let service = service(access(false, device_id()), factory.clone());

    let failure = service
        .enter(request(device_id()))
        .await
        .expect_err("失效实例必须被拒绝");

    assert_eq!(failure.kind(), AgentLobbyEntryFailureKind::Unauthorized);
    assert!(factory.users.lock().expect("用户记录锁可用").is_empty());
}

/// 只回放一个私人房间快照；入场只按 Matrix 房间定位，其余方法不应被调用。
struct 固定私人房间仓储(Option<PrivateRoomSnapshot>);

impl PrivateRoomStore for 固定私人房间仓储 {
    fn create<'a>(
        &'a self,
        _snapshot: &'a PrivateRoomSnapshot,
        _created_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("入场不得创建私人房间") })
    }
    fn find_by_catalog(
        &self,
        _catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("入场按 Matrix 房间定位") })
    }
    fn list_for_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<PrivateRoomSnapshot>>> {
        Box::pin(async { unreachable!("入场不列举房间") })
    }
    fn find_by_matrix_room<'a>(
        &'a self,
        _matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<PrivateRoomSnapshot>>> {
        let value = self.0.clone();
        Box::pin(async move { Ok(value) })
    }
    fn save<'a>(
        &'a self,
        _room: &'a PrivateRoom,
        _expected_version: AggregateVersion,
        _changed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { unreachable!("入场不得修改成员事实") })
    }
}

/// 记录受邀的 Matrix 身份，并可模拟 Agent 已经在房间内的重连情形。
struct 记录私人Matrix {
    已加入: bool,
    邀请: Mutex<Vec<String>>,
}

impl 记录私人Matrix {
    fn new(已加入: bool) -> Self {
        Self {
            已加入,
            邀请: Mutex::new(Vec::new()),
        }
    }
    fn 邀请记录(&self) -> Vec<String> {
        self.邀请.lock().expect("锁可用").clone()
    }
}

impl PrivateRoomMatrixGateway for 记录私人Matrix {
    fn membership<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<Option<PrivateMatrixMembership>>> {
        let joined = self.已加入;
        Box::pin(async move { Ok(joined.then_some(PrivateMatrixMembership::Joined)) })
    }
    fn invite<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            self.邀请
                .lock()
                .expect("锁可用")
                .push(user_id.as_str().to_owned());
            Ok(())
        })
    }
    fn kick<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { unreachable!("入场不得移除成员") })
    }
    fn ban<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { unreachable!("入场不得封禁成员") })
    }
    fn set_speaking<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _user_id: &'a MatrixUserId,
        _allowed: bool,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { unreachable!("入场不得改写发言权限") })
    }
    fn set_speaking_batch<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _assignments: &'a [PrivateMatrixSpeakingAssignment],
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { unreachable!("入场不得改写发言权限") })
    }
    fn archive<'a>(&'a self, _room_id: &'a MatrixRoomId) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { unreachable!("入场不得归档房间") })
    }
}

fn 私人房间快照(
    成员: Option<(PrincipalId, PrivateRoomPermissions)>
) -> PrivateRoomSnapshot {
    let owner = PrincipalId::from_uuid(Uuid::from_u128(7_777));
    let mut room = PrivateRoom::create(private_catalog_id(), owner);
    if let Some((principal_id, permissions)) = 成员 {
        room.invite(owner, principal_id, permissions)
            .expect("邀请成功");
        room.accept_invitation(principal_id).expect("接受邀请成功");
    }
    let catalog = RoomCatalog::new(
        private_catalog_id(),
        RoomCatalogFields {
            kind: RoomCatalogKind::PrivateRoom,
            slug: None,
            name: "项目室".to_owned(),
            description: String::new(),
            language: None,
            matrix_space_id: None,
            owner_principal_id: Some(owner),
            visibility: RoomCatalogVisibility::Private,
            retention_days: Some(30),
            status: RoomCatalogStatus::Active,
        },
    )
    .expect("私人目录有效");
    let instance = RoomInstance::restore(
        private_room_instance_id(),
        RoomInstanceFields {
            catalog_id: private_catalog_id(),
            matrix_room_id: private_matrix_room(),
            region: None,
            capacity: RoomCapacity::standard(),
            projected_member_count: 0,
            allocated_slots: 0,
            activity_score_millis: 0,
            state: RoomInstanceState::Active,
        },
    )
    .expect("私人实例有效");
    PrivateRoomSnapshot::new(catalog, instance, room).expect("快照一致")
}

fn 可发言权限() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([
        PrivateRoomCapability::View,
        PrivateRoomCapability::Speak,
    ])
    .expect("权限有效")
}

fn 只读权限() -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities([PrivateRoomCapability::View]).expect("权限有效")
}

fn private_catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(5_150))
}

fn private_room_instance_id() -> RoomInstanceId {
    RoomInstanceId::from_uuid(Uuid::from_u128(5_151))
}

fn private_matrix_room() -> MatrixRoomReference {
    MatrixRoomReference::new("!project:matrix.test".to_owned()).expect("房间标识有效")
}

fn service(
    access: AgentLobbyAccessRecord,
    memberships: Arc<记录成员工厂>,
) -> AgentLobbyEntryService {
    service_with_mode(access, memberships, RoomAllocationMode::Automatic)
}

fn service_with_mode(
    access: AgentLobbyAccessRecord,
    memberships: Arc<记录成员工厂>,
    expected_mode: RoomAllocationMode,
) -> AgentLobbyEntryService {
    private_service(
        access,
        memberships,
        expected_mode,
        None,
        Arc::new(记录私人Matrix::new(false)),
        room(),
        reservation(),
    )
}

fn private_service(
    access: AgentLobbyAccessRecord,
    memberships: Arc<记录成员工厂>,
    expected_mode: RoomAllocationMode,
    snapshot: Option<PrivateRoomSnapshot>,
    private_matrix: Arc<记录私人Matrix>,
    room: RoomInstance,
    reservation: RoomReservation,
) -> AgentLobbyEntryService {
    let runtime = Arc::new(固定运行时);
    AgentLobbyEntryService::new(
        AgentLobbyEntryDependencies {
            access: Arc::new(固定访问仓储(Some(access))),
            allocations: Arc::new(固定分配仓储 {
                reservation,
                room,
                expected_mode,
            }),
            private_rooms: Arc::new(固定私人房间仓储(snapshot)),
            private_matrix,
            memberships,
            provisioning: Arc::new(禁止供给),
            identifiers: runtime.clone(),
            clock: runtime,
        },
        LobbyJoinPolicy::new(DurationMillis::new(60_000).expect("时限有效")).expect("策略有效"),
    )
}

fn access(active: bool, device_id: DeviceId) -> AgentLobbyAccessRecord {
    AgentLobbyAccessRecord {
        agent_id: agent_id(),
        agent_instance_id: instance_id(),
        device_id,
        principal_id: operator_principal_id(),
        matrix_user_id: matrix_user_id(),
        active,
    }
}

fn operator_principal_id() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(4_242))
}

#[tokio::test]
async fn 精确邀请只进入指定公共房间且不可用时不分配默认房间() {
    for (room_id, allowed) in [
        ("!lobby:matrix.test", true),
        ("!missing:matrix.test", false),
    ] {
        let membership = Arc::new(记录成员能力::default());
        let factory = Arc::new(记录成员工厂 {
            users: Mutex::new(Vec::new()),
            membership: membership.clone(),
        });
        let service = service_with_mode(
            access(true, device_id()),
            factory.clone(),
            RoomAllocationMode::Manual(room_instance_id()),
        );
        let mut request = request(device_id());
        request.target_room = Some(MatrixRoomReference::new(room_id).unwrap());
        let result = service.enter(request).await;
        if allowed {
            assert!(result.is_ok());
            assert_eq!(*membership.joins.lock().unwrap(), [room_id]);
        } else {
            assert_eq!(
                result.unwrap_err().kind(),
                AgentLobbyEntryFailureKind::NotFound
            );
            assert!(factory.users.lock().unwrap().is_empty());
            assert!(membership.joins.lock().unwrap().is_empty());
        }
    }
}

/// 组装一次以私人房间为目标的入场请求。
fn private_request(
    snapshot: Option<PrivateRoomSnapshot>,
    private_matrix: &Arc<记录私人Matrix>,
) -> (AgentLobbyEntryService, EnterAgentLobby, Arc<记录成员能力>) {
    let membership = Arc::new(记录成员能力::default());
    let factory = Arc::new(记录成员工厂 {
        users: Mutex::new(Vec::new()),
        membership: membership.clone(),
    });
    let instance = RoomInstance::restore(
        private_room_instance_id(),
        RoomInstanceFields {
            catalog_id: private_catalog_id(),
            matrix_room_id: private_matrix_room(),
            region: None,
            capacity: RoomCapacity::standard(),
            projected_member_count: 0,
            allocated_slots: 0,
            activity_score_millis: 0,
            state: RoomInstanceState::Active,
        },
    )
    .expect("私人实例有效");
    let service = private_service(
        access(true, device_id()),
        factory,
        RoomAllocationMode::Manual(private_room_instance_id()),
        snapshot,
        private_matrix.clone(),
        instance,
        private_reservation(),
    );
    let mut request = request(device_id());
    request.catalog_id = private_catalog_id();
    request.target_room = Some(private_matrix_room());
    (service, request, membership)
}

fn private_reservation() -> RoomReservation {
    RoomReservation::restore(
        RoomReservationId::from_uuid(Uuid::from_u128(5_152)),
        RoomReservationFields {
            catalog_id: private_catalog_id(),
            room_instance_id: private_room_instance_id(),
            agent_instance_id: instance_id(),
            reserved_at: time(NOW - 1_000),
            expires_at: time(NOW + 59_000),
            state: RoomReservationState::Committed,
            finalized_at: Some(time(NOW - 500)),
        },
    )
    .expect("私人预约有效")
}

#[tokio::test]
async fn agent_随已加入且可发言的主体进入私人房间并被邀请到_matrix() {
    let matrix = Arc::new(记录私人Matrix::new(false));
    let snapshot = 私人房间快照(Some((operator_principal_id(), 可发言权限())));
    let (service, request, membership) = private_request(Some(snapshot), &matrix);

    service.enter(request).await.expect("入场成功");

    assert_eq!(matrix.邀请记录(), [matrix_user_id().as_str()]);
    assert_eq!(
        *membership.joins.lock().expect("锁可用"),
        [private_matrix_room().as_str()]
    );
}

#[tokio::test]
async fn 非成员与只读成员的_agent_都进不了私人房间且不会被邀请() {
    for 成员 in [
        None,
        Some((operator_principal_id(), 只读权限())),
        Some((PrincipalId::from_uuid(Uuid::from_u128(8_888)), 可发言权限())),
    ] {
        let matrix = Arc::new(记录私人Matrix::new(false));
        let (service, request, membership) = private_request(Some(私人房间快照(成员)), &matrix);

        let failure = service.enter(request).await.expect_err("必须拒绝");

        assert_eq!(failure.kind(), AgentLobbyEntryFailureKind::Unauthorized);
        assert!(matrix.邀请记录().is_empty(), "被拒绝时不得发出 Matrix 邀请");
        assert!(membership.joins.lock().expect("锁可用").is_empty());
    }
}

#[tokio::test]
async fn 重连时不重复邀请已在房间的_agent() {
    let matrix = Arc::new(记录私人Matrix::new(true));
    let snapshot = 私人房间快照(Some((operator_principal_id(), 可发言权限())));
    let (service, request, _) = private_request(Some(snapshot), &matrix);

    service.enter(request).await.expect("入场成功");

    assert!(matrix.邀请记录().is_empty(), "已加入时不应再次邀请");
}

#[tokio::test]
async fn 不能用另一个房间的成员资格换取本房间入场() {
    let matrix = Arc::new(记录私人Matrix::new(false));
    // 快照本身允许该主体，但它属于另一个目录，请求的目录必须一致。
    let snapshot = 私人房间快照(Some((operator_principal_id(), 可发言权限())));
    let (service, mut request, _) = private_request(Some(snapshot), &matrix);
    request.catalog_id = catalog_id();

    let failure = service.enter(request).await.expect_err("必须拒绝");

    assert_eq!(failure.kind(), AgentLobbyEntryFailureKind::NotFound);
    assert!(matrix.邀请记录().is_empty());
}

#[tokio::test]
async fn 既非公共大厅也不存在私人房间时拒绝入场() {
    let matrix = Arc::new(记录私人Matrix::new(false));
    let (service, request, _) = private_request(None, &matrix);

    let failure = service.enter(request).await.expect_err("必须拒绝");

    assert_eq!(failure.kind(), AgentLobbyEntryFailureKind::NotFound);
    assert!(matrix.邀请记录().is_empty());
}

fn request(device_id: DeviceId) -> EnterAgentLobby {
    let principal_id = PrincipalId::from_uuid(Uuid::from_u128(5));
    EnterAgentLobby {
        actor: AuthenticatedDevice {
            account: PrincipalAccount {
                principal: Principal::new(principal_id),
                matrix_user_id: "@owner:matrix.test".to_owned(),
                display_name: "Owner".to_owned(),
                avatar_content_id: None,
                locale: "zh-CN".to_owned(),
            },
            device_id,
            access_token_expires_at: time(NOW + 60_000),
        },
        agent_id: agent_id(),
        agent_instance_id: instance_id(),
        catalog_id: catalog_id(),
        preferred_language: None,
        preferred_region: None,
        target_room: None,
    }
}

fn room() -> RoomInstance {
    RoomInstance::restore(
        room_instance_id(),
        RoomInstanceFields {
            catalog_id: catalog_id(),
            matrix_room_id: MatrixRoomReference::new("!lobby:matrix.test").expect("房间标识有效"),
            region: None,
            capacity: RoomCapacity::new(180, 250).expect("容量有效"),
            projected_member_count: 1,
            allocated_slots: 1,
            activity_score_millis: 0,
            state: RoomInstanceState::Active,
        },
    )
    .expect("房间有效")
}

fn reservation() -> RoomReservation {
    RoomReservation::restore(
        RoomReservationId::from_uuid(Uuid::from_u128(8)),
        RoomReservationFields {
            catalog_id: catalog_id(),
            room_instance_id: room_instance_id(),
            agent_instance_id: instance_id(),
            reserved_at: time(NOW - 1_000),
            expires_at: time(NOW + 59_000),
            state: RoomReservationState::Committed,
            finalized_at: Some(time(NOW - 500)),
        },
    )
    .expect("预约有效")
}

fn matrix_user_id() -> MatrixUserId {
    MatrixUserId::new("@_agent_00000000000000000000000000000001:matrix.test")
        .expect("Matrix 用户有效")
}

fn agent_id() -> AgentId {
    AgentId::from_uuid(Uuid::from_u128(1))
}

fn instance_id() -> AgentInstanceId {
    AgentInstanceId::from_uuid(Uuid::from_u128(2))
}

fn device_id() -> DeviceId {
    DeviceId::from_uuid(Uuid::from_u128(3))
}

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(6))
}

fn room_instance_id() -> RoomInstanceId {
    RoomInstanceId::from_uuid(Uuid::from_u128(7))
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
