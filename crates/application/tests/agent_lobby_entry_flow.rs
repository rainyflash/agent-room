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
        MatrixResult, MatrixUserId, PortFuture, PrincipalAccount, RoomAllocationStore,
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
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomInstance, RoomInstanceFields, RoomInstanceState,
        RoomReservation, RoomReservationFields, RoomReservationState,
    },
    time::{DurationMillis, UtcMillis},
};
use uuid::Uuid;

const NOW: i64 = 1_700_000_000_000;

struct 固定访问仓储(Option<AgentLobbyAccessRecord>);

impl AgentLobbyAccessRepository for 固定访问仓储 {
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
}

impl RoomAllocationStore for 固定分配仓储 {
    fn reserve<'a>(
        &'a self,
        claim: &'a RoomReservationClaim,
    ) -> PortFuture<'a, RepositoryResult<RoomReservationOutcome>> {
        Box::pin(async move {
            assert_eq!(claim.agent_id, agent_id());
            assert_eq!(claim.agent_instance_id, instance_id());
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

fn service(
    access: AgentLobbyAccessRecord,
    memberships: Arc<记录成员工厂>,
) -> AgentLobbyEntryService {
    let runtime = Arc::new(固定运行时);
    AgentLobbyEntryService::new(
        AgentLobbyEntryDependencies {
            access: Arc::new(固定访问仓储(Some(access))),
            allocations: Arc::new(固定分配仓储 {
                reservation: reservation(),
                room: room(),
            }),
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
        matrix_user_id: matrix_user_id(),
        active,
    }
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
