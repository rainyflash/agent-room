use std::sync::{Arc, Mutex};

use agent_room_application::{
    agent_lobbies::AgentLobbyEntryFailureKind,
    agent_room_placements::{
        AgentRoomActor, AgentRoomPlacementDependencies, AgentRoomPlacementService,
        AgentRoomPlacementUseCases, LeaveAgentRoom, LeaveAllAgentRooms, ListAgentRooms,
    },
    authentication::AuthenticatedPrincipal,
    devices::AuthenticatedDevice,
    persistence::RepositoryResult,
    ports::{
        AgentLobbyAccessRecord, AgentLobbyAccessRepository, AgentMembershipRepository,
        AgentRoomKind, AgentRoomMembershipFactory, AgentRoomPlacement, AgentRoomPlacementStore,
        Clock, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId,
        MatrixUserId, PortFuture, PrincipalAccount, PrivateMatrixMembership,
        PrivateMatrixSpeakingAssignment, PrivateRoomMatrixGateway, RoomAllocationStore,
        RoomMembershipGateway, RoomReservationClaim, RoomReservationOutcome,
    },
};
use agent_room_domain::{
    agents::{AgentMember, AgentMemberships, AgentRole},
    identity::Principal,
    ids::{
        AgentId, AgentInstanceId, DeviceId, PrincipalId, RoomCatalogId, RoomInstanceId,
        RoomReservationId,
    },
    rooms::{
        MatrixRoomReference, RoomCatalogStatus, RoomReservation, RoomReservationFields,
        RoomReservationState,
    },
    time::UtcMillis,
};
use uuid::Uuid;

const NOW: i64 = 1_700_000_000_000;

/// 各个假依赖按调用顺序记下做了什么，测试按顺序核对。
#[derive(Default)]
struct 调用记录(Mutex<Vec<String>>);

impl 调用记录 {
    fn 记(&self, entry: impl Into<String>) {
        self.0.lock().expect("记录锁可用").push(entry.into());
    }

    fn 全部(&self) -> Vec<String> {
        self.0.lock().expect("记录锁可用").clone()
    }
}

struct 固定访问(Option<AgentLobbyAccessRecord>);

impl AgentLobbyAccessRepository for 固定访问 {
    fn find_public_lobby_room<'a>(
        &'a self,
        _catalog_id: RoomCatalogId,
        _matrix_room_id: &'a MatrixRoomReference,
    ) -> PortFuture<'a, RepositoryResult<Option<RoomInstanceId>>> {
        Box::pin(async { unreachable!("离开不解析大厅") })
    }

    fn find_lobby_access(
        &self,
        _agent_instance_id: AgentInstanceId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentLobbyAccessRecord>>> {
        let value = self.0.clone();
        Box::pin(async move { Ok(value) })
    }
}

struct 固定成员(Option<AgentMemberships>);

impl AgentMembershipRepository for 固定成员 {
    fn find_memberships(
        &self,
        _agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<AgentMemberships>>> {
        let value = self.0.clone();
        Box::pin(async move { Ok(value) })
    }
}

struct 固定位置 {
    placements: Vec<AgentRoomPlacement>,
    log: Arc<调用记录>,
}

impl AgentRoomPlacementStore for 固定位置 {
    fn placements(
        &self,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Vec<AgentRoomPlacement>>> {
        assert_eq!(agent_id, agent());
        let value = self.placements.clone();
        Box::pin(async move { Ok(value) })
    }

    fn matrix_user(
        &self,
        _agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<MatrixUserId>>> {
        Box::pin(async { Ok(Some(matrix_user())) })
    }

    fn forget_agent_member(
        &self,
        catalog_id: RoomCatalogId,
        _agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        self.log
            .记(format!("forget {}", catalog_id.as_uuid().as_u128()));
        Box::pin(async { Ok(true) })
    }
}

struct 记录分配(Arc<调用记录>);

impl RoomAllocationStore for 记录分配 {
    fn reserve<'a>(
        &'a self,
        _claim: &'a RoomReservationClaim,
    ) -> PortFuture<'a, RepositoryResult<RoomReservationOutcome>> {
        Box::pin(async { unreachable!("离开不占位置") })
    }

    fn transition(
        &self,
        reservation_id: RoomReservationId,
        expected: RoomReservationState,
        target: RoomReservationState,
        changed_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<RoomReservation>> {
        assert_eq!(expected, RoomReservationState::Committed);
        assert_eq!(target, RoomReservationState::Released);
        self.0
            .记(format!("release {}", reservation_id.as_uuid().as_u128()));
        Box::pin(async move { Ok(released(reservation_id, changed_at)) })
    }

    fn expire_pending(
        &self,
        _now: UtcMillis,
        _limit: u16,
    ) -> PortFuture<'_, RepositoryResult<u16>> {
        Box::pin(async { unreachable!("离开不清理过期预约") })
    }
}

struct 记录私人Matrix(Arc<调用记录>);

impl PrivateRoomMatrixGateway for 记录私人Matrix {
    fn membership<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<Option<PrivateMatrixMembership>>> {
        Box::pin(async { unreachable!("离开不查成员") })
    }

    fn invite<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { unreachable!("离开不邀请") })
    }

    fn kick<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { unreachable!("离开不移出") })
    }

    fn ban<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { unreachable!("离开不封禁") })
    }

    fn unban<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { unreachable!("离开不解封") })
    }

    fn set_speaking<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        user_id: &'a MatrixUserId,
        allowed: bool,
    ) -> PortFuture<'a, MatrixResult<()>> {
        assert_eq!(user_id, &matrix_user());
        self.0
            .记(format!("speaking {} {allowed}", room_id.as_str()));
        Box::pin(async { Ok(()) })
    }

    fn set_speaking_batch<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _assignments: &'a [PrivateMatrixSpeakingAssignment],
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { unreachable!("离开只改它一个人的发言") })
    }

    fn archive<'a>(&'a self, _room_id: &'a MatrixRoomId) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { unreachable!("离开不归档") })
    }

    fn set_name<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _name: &'a str,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { unreachable!("离开不改名") })
    }
}

struct 记录离开 {
    log: Arc<调用记录>,
    failure: Option<MatrixFailureKind>,
}

impl RoomMembershipGateway for 记录离开 {
    fn join<'a>(&'a self, _room_id: &'a MatrixRoomReference) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async { unreachable!("离开不加入") })
    }

    fn leave<'a>(&'a self, room_id: &'a MatrixRoomReference) -> PortFuture<'a, MatrixResult<()>> {
        self.log.记(format!("leave {}", room_id.as_str()));
        let result = self.failure.map_or(Ok(()), |kind| {
            Err(MatrixFailure::new(MatrixOperation::Leave, kind))
        });
        Box::pin(async move { result })
    }
}

struct 离开工厂(Arc<记录离开>);

impl AgentRoomMembershipFactory for 离开工厂 {
    fn bind(&self, matrix_user_id: &MatrixUserId) -> MatrixResult<Arc<dyn RoomMembershipGateway>> {
        assert_eq!(matrix_user_id, &matrix_user());
        Ok(self.0.clone())
    }
}

struct 固定时钟;

impl Clock for 固定时钟 {
    fn now(&self) -> UtcMillis {
        time(NOW)
    }
}

struct 场景 {
    access: Option<AgentLobbyAccessRecord>,
    members: Option<AgentMemberships>,
    placements: Vec<AgentRoomPlacement>,
    leave_failure: Option<MatrixFailureKind>,
}

impl 场景 {
    fn new(placements: Vec<AgentRoomPlacement>) -> Self {
        Self {
            access: Some(access(true, device())),
            members: Some(memberships(AgentRole::Owner)),
            placements,
            leave_failure: None,
        }
    }

    fn service(self) -> (AgentRoomPlacementService, Arc<调用记录>) {
        let log = Arc::new(调用记录::default());
        let service = AgentRoomPlacementService::new(AgentRoomPlacementDependencies {
            access: Arc::new(固定访问(self.access)),
            agent_memberships: Arc::new(固定成员(self.members)),
            placements: Arc::new(固定位置 {
                placements: self.placements,
                log: log.clone(),
            }),
            allocations: Arc::new(记录分配(log.clone())),
            private_matrix: Arc::new(记录私人Matrix(log.clone())),
            memberships: Arc::new(离开工厂(Arc::new(记录离开 {
                log: log.clone(),
                failure: self.leave_failure,
            }))),
            clock: Arc::new(固定时钟),
        });
        (service, log)
    }
}

#[tokio::test]
async fn 主人看到它在哪些房间同一个房间只算一次() {
    let (service, _) = 场景::new(vec![
        placement(1, 10, AgentRoomKind::PublicLobby, RoomCatalogStatus::Active),
        placement(2, 10, AgentRoomKind::PublicLobby, RoomCatalogStatus::Active),
        placement(
            3,
            20,
            AgentRoomKind::PrivateRoom,
            RoomCatalogStatus::Archived,
        ),
    ])
    .service();

    let rooms = service
        .rooms(ListAgentRooms {
            actor: owner(),
            agent_id: agent(),
        })
        .await
        .expect("主人能看");

    assert_eq!(rooms.len(), 2);
    assert_eq!(rooms[0].catalog_id, catalog(10));
    assert_eq!(rooms[0].kind, AgentRoomKind::PublicLobby);
    assert_eq!(rooms[1].catalog_id, catalog(20));
    assert_eq!(rooms[1].status, RoomCatalogStatus::Archived);
}

#[tokio::test]
async fn 不是它的成员当作没有这个_agent() {
    let mut scene = 场景::new(vec![placement(
        1,
        10,
        AgentRoomKind::PublicLobby,
        RoomCatalogStatus::Active,
    )]);
    scene.members = Some(
        AgentMemberships::restore(
            agent(),
            vec![member(
                PrincipalId::from_uuid(Uuid::from_u128(77)),
                AgentRole::Owner,
            )],
        )
        .expect("成员有效"),
    );
    let (service, log) = scene.service();

    let listed = service
        .rooms(ListAgentRooms {
            actor: owner(),
            agent_id: agent(),
        })
        .await
        .expect_err("别人的 Agent 看不到");
    let left = service
        .leave(owner_leave(10))
        .await
        .expect_err("别人的 Agent 不能让它离开");

    assert_eq!(listed.kind(), AgentLobbyEntryFailureKind::NotFound);
    assert_eq!(left.kind(), AgentLobbyEntryFailureKind::NotFound);
    assert!(log.全部().is_empty());
}

#[tokio::test]
async fn 只能查看的成员看得到但不能让它离开() {
    let mut scene = 场景::new(vec![placement(
        1,
        10,
        AgentRoomKind::PublicLobby,
        RoomCatalogStatus::Active,
    )]);
    scene.members = Some(memberships(AgentRole::Viewer));
    let (service, log) = scene.service();

    service
        .rooms(ListAgentRooms {
            actor: owner(),
            agent_id: agent(),
        })
        .await
        .expect("查看的成员能看");
    let failure = service
        .leave(owner_leave(10))
        .await
        .expect_err("查看的成员不能让它离开");

    assert_eq!(failure.kind(), AgentLobbyEntryFailureKind::Unauthorized);
    assert!(log.全部().is_empty());
}

#[tokio::test]
async fn 私人房间里先收回发言再离开再删成员记录最后放掉每个位置() {
    let (service, log) = 场景::new(vec![
        placement(1, 20, AgentRoomKind::PrivateRoom, RoomCatalogStatus::Active),
        placement(2, 10, AgentRoomKind::PublicLobby, RoomCatalogStatus::Active),
        placement(3, 20, AgentRoomKind::PrivateRoom, RoomCatalogStatus::Active),
    ])
    .service();

    let left = service.leave(device_leave(20)).await.expect("能离开");

    assert!(left.left);
    assert_eq!(
        log.全部(),
        vec![
            format!("speaking {} false", matrix_room(20).as_str()),
            format!("leave {}", matrix_room(20).as_str()),
            "forget 20".to_owned(),
            "release 1".to_owned(),
            "release 3".to_owned(),
        ]
    );
}

#[tokio::test]
async fn 公共大厅不动发言级别也没有成员记录() {
    let (service, log) = 场景::new(vec![placement(
        1,
        10,
        AgentRoomKind::PublicLobby,
        RoomCatalogStatus::Active,
    )])
    .service();

    service.leave(owner_leave(10)).await.expect("能离开");

    assert_eq!(
        log.全部(),
        vec![
            format!("leave {}", matrix_room(10).as_str()),
            "release 1".to_owned(),
        ]
    );
}

#[tokio::test]
async fn 归档了的房间不写发言级别免得取消归档() {
    let (service, log) = 场景::new(vec![placement(
        1,
        20,
        AgentRoomKind::PrivateRoom,
        RoomCatalogStatus::Archived,
    )])
    .service();

    service.leave(owner_leave(20)).await.expect("能离开");

    assert_eq!(
        log.全部(),
        vec![
            format!("leave {}", matrix_room(20).as_str()),
            "forget 20".to_owned(),
            "release 1".to_owned(),
        ]
    );
}

#[tokio::test]
async fn 不在这个房间时什么都不动也算成功() {
    let (service, log) = 场景::new(vec![placement(
        1,
        10,
        AgentRoomKind::PublicLobby,
        RoomCatalogStatus::Active,
    )])
    .service();

    let left = service
        .leave(owner_leave(30))
        .await
        .expect("重复离开不报错");

    assert!(!left.left);
    assert!(log.全部().is_empty());
}

#[tokio::test]
async fn matrix_上已经不在房间里时照样放掉位置() {
    for kind in [MatrixFailureKind::Forbidden, MatrixFailureKind::NotFound] {
        let mut scene = 场景::new(vec![placement(
            1,
            10,
            AgentRoomKind::PublicLobby,
            RoomCatalogStatus::Active,
        )]);
        scene.leave_failure = Some(kind);
        let (service, log) = scene.service();

        service.leave(owner_leave(10)).await.expect("当作已经离开");

        assert_eq!(log.全部().last().map(String::as_str), Some("release 1"));
    }
}

#[tokio::test]
async fn matrix_暂时不通时不放位置() {
    let mut scene = 场景::new(vec![placement(
        1,
        10,
        AgentRoomKind::PublicLobby,
        RoomCatalogStatus::Active,
    )]);
    scene.leave_failure = Some(MatrixFailureKind::DependencyUnavailable);
    let (service, log) = scene.service();

    let failure = service.leave(owner_leave(10)).await.expect_err("没离开");

    assert_eq!(
        failure.kind(),
        AgentLobbyEntryFailureKind::DependencyUnavailable
    );
    assert_eq!(
        log.全部(),
        vec![format!("leave {}", matrix_room(10).as_str())]
    );
}

#[tokio::test]
async fn 设备和连接对不上时不能替它离开() {
    for access in [
        None,
        Some(access(false, device())),
        Some(access(true, DeviceId::from_uuid(Uuid::from_u128(99)))),
    ] {
        let mut scene = 场景::new(vec![placement(
            1,
            10,
            AgentRoomKind::PublicLobby,
            RoomCatalogStatus::Active,
        )]);
        let expected = if access.is_some() {
            AgentLobbyEntryFailureKind::Unauthorized
        } else {
            AgentLobbyEntryFailureKind::NotFound
        };
        scene.access = access;
        let (service, log) = scene.service();

        let failure = service.leave(device_leave(10)).await.expect_err("不能离开");

        assert_eq!(failure.kind(), expected);
        assert!(log.全部().is_empty());
    }
}

#[tokio::test]
async fn 登录过期时不能看也不能离开() {
    let (service, log) = 场景::new(vec![placement(
        1,
        10,
        AgentRoomKind::PublicLobby,
        RoomCatalogStatus::Active,
    )])
    .service();
    let mut expired = owner();
    expired.expires_at = time(NOW);

    let listed = service
        .rooms(ListAgentRooms {
            actor: expired.clone(),
            agent_id: agent(),
        })
        .await
        .expect_err("过期了");
    let left = service
        .leave(LeaveAgentRoom {
            actor: AgentRoomActor::Owner(expired),
            agent_id: agent(),
            catalog_id: catalog(10),
        })
        .await
        .expect_err("过期了");

    assert_eq!(listed.kind(), AgentLobbyEntryFailureKind::Unauthorized);
    assert_eq!(left.kind(), AgentLobbyEntryFailureKind::Unauthorized);
    assert!(log.全部().is_empty());
}

#[tokio::test]
async fn 删除之前离开所有房间_每个房间照离开一个房间那样做() {
    let (service, log) = 场景::new(vec![
        placement(1, 20, AgentRoomKind::PrivateRoom, RoomCatalogStatus::Active),
        placement(2, 10, AgentRoomKind::PublicLobby, RoomCatalogStatus::Active),
        placement(3, 20, AgentRoomKind::PrivateRoom, RoomCatalogStatus::Active),
    ])
    .service();

    let left = service
        .leave_all(LeaveAllAgentRooms {
            actor: owner(),
            agent_id: agent(),
        })
        .await
        .expect("主人能让它离开所有房间");

    assert_eq!(left, 2);
    assert_eq!(
        log.全部(),
        vec![
            format!("speaking {} false", matrix_room(20).as_str()),
            format!("leave {}", matrix_room(20).as_str()),
            "forget 20".to_owned(),
            "release 1".to_owned(),
            "release 3".to_owned(),
            format!("leave {}", matrix_room(10).as_str()),
            "release 2".to_owned(),
        ]
    );
}

#[tokio::test]
async fn 删除之前离开所有房间只有主人能做_不在房间里时什么都不动() {
    for role in [AgentRole::Operator, AgentRole::Viewer] {
        let mut scene = 场景::new(vec![placement(
            1,
            10,
            AgentRoomKind::PublicLobby,
            RoomCatalogStatus::Active,
        )]);
        scene.members = Some(memberships(role));
        let (service, log) = scene.service();

        let failure = service
            .leave_all(LeaveAllAgentRooms {
                actor: owner(),
                agent_id: agent(),
            })
            .await
            .expect_err("不是主人");

        assert_eq!(failure.kind(), AgentLobbyEntryFailureKind::Unauthorized);
        assert!(log.全部().is_empty());
    }

    let (service, log) = 场景::new(Vec::new()).service();
    let left = service
        .leave_all(LeaveAllAgentRooms {
            actor: owner(),
            agent_id: agent(),
        })
        .await
        .expect("没在房间里");
    assert_eq!(left, 0);
    assert!(log.全部().is_empty());
}

fn owner_leave(catalog_id: u128) -> LeaveAgentRoom {
    LeaveAgentRoom {
        actor: AgentRoomActor::Owner(owner()),
        agent_id: agent(),
        catalog_id: catalog(catalog_id),
    }
}

fn device_leave(catalog_id: u128) -> LeaveAgentRoom {
    LeaveAgentRoom {
        actor: AgentRoomActor::Device {
            device: AuthenticatedDevice {
                account: PrincipalAccount {
                    principal: Principal::new(owner_id()),
                    matrix_user_id: "@owner:matrix.test".to_owned(),
                    display_name: "Owner".to_owned(),
                    avatar_content_id: None,
                    locale: "zh-CN".to_owned(),
                },
                device_id: device(),
                access_token_expires_at: time(NOW + 60_000),
            },
            agent_instance_id: instance(),
        },
        agent_id: agent(),
        catalog_id: catalog(catalog_id),
    }
}

fn owner() -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: owner_id(),
        matrix_user_id: "@owner:matrix.test".to_owned(),
        display_name: "Owner".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(NOW - 1_000),
        expires_at: time(NOW + 60_000),
        recently_authenticated: false,
    }
}

fn memberships(role: AgentRole) -> AgentMemberships {
    let other_owner = PrincipalId::from_uuid(Uuid::from_u128(77));
    let mut members = vec![member(owner_id(), role)];
    if role != AgentRole::Owner {
        members.push(member(other_owner, AgentRole::Owner));
    }
    AgentMemberships::restore(agent(), members).expect("成员有效")
}

fn member(principal_id: PrincipalId, role: AgentRole) -> AgentMember {
    AgentMember::restore(
        agent(),
        principal_id,
        role,
        principal_id,
        time(NOW - 10_000),
        None,
    )
    .expect("成员有效")
}

fn access(active: bool, device_id: DeviceId) -> AgentLobbyAccessRecord {
    AgentLobbyAccessRecord {
        agent_id: agent(),
        agent_instance_id: instance(),
        device_id,
        principal_id: owner_id(),
        matrix_user_id: matrix_user(),
        active,
    }
}

fn placement(
    reservation: u128,
    catalog_id: u128,
    kind: AgentRoomKind,
    status: RoomCatalogStatus,
) -> AgentRoomPlacement {
    AgentRoomPlacement {
        reservation_id: RoomReservationId::from_uuid(Uuid::from_u128(reservation)),
        agent_instance_id: instance(),
        catalog_id: catalog(catalog_id),
        room_instance_id: RoomInstanceId::from_uuid(Uuid::from_u128(catalog_id + 1)),
        matrix_room_id: matrix_room(catalog_id),
        kind,
        name: format!("房间 {catalog_id}"),
        status,
    }
}

fn released(reservation_id: RoomReservationId, at: UtcMillis) -> RoomReservation {
    RoomReservation::restore(
        reservation_id,
        RoomReservationFields {
            catalog_id: catalog(10),
            room_instance_id: RoomInstanceId::from_uuid(Uuid::from_u128(11)),
            agent_instance_id: instance(),
            reserved_at: time(NOW - 1_000),
            expires_at: time(NOW + 59_000),
            state: RoomReservationState::Released,
            finalized_at: Some(at),
        },
    )
    .expect("预约有效")
}

fn matrix_room(catalog_id: u128) -> MatrixRoomReference {
    MatrixRoomReference::new(format!("!room{catalog_id}:matrix.test")).expect("房间有效")
}

fn matrix_user() -> MatrixUserId {
    MatrixUserId::new("@_agent_00000000000000000000000000000001:matrix.test")
        .expect("Matrix 用户有效")
}

fn agent() -> AgentId {
    AgentId::from_uuid(Uuid::from_u128(1))
}

fn instance() -> AgentInstanceId {
    AgentInstanceId::from_uuid(Uuid::from_u128(2))
}

fn device() -> DeviceId {
    DeviceId::from_uuid(Uuid::from_u128(3))
}

fn owner_id() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(5))
}

fn catalog(value: u128) -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(value))
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("时间有效")
}
