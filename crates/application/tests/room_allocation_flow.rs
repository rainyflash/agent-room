use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        Clock, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult,
        RoomAllocationEvidence, RoomAllocationMode, RoomAllocationStore, RoomMembershipGateway,
        RoomReservationClaim, RoomReservationOutcome,
    },
    rooms::{
        JoinLobbyDependencies, JoinLobbyFailure, JoinLobbyOutcome, JoinLobbyRequest,
        JoinLobbyService, LobbyJoinKind, LobbyJoinPolicy, RoomReservationIdentifierFactory,
    },
};
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, RoomCatalogId, RoomInstanceId, RoomReservationId},
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomInstance, RoomInstanceFields, RoomInstanceState,
        RoomReservation, RoomReservationFields, RoomReservationState,
    },
    time::{DurationMillis, UtcMillis},
};
use uuid::Uuid;

struct 测试时钟;

impl Clock for 测试时钟 {
    fn now(&self) -> UtcMillis {
        time(1_000)
    }
}

struct 测试标识;

impl RoomReservationIdentifierFactory for 测试标识 {
    fn room_reservation_id(&self) -> RoomReservationId {
        reservation_id()
    }
}

struct 记录成员网关 {
    join_calls: AtomicUsize,
    leave_calls: AtomicUsize,
    fail_join: AtomicBool,
    fail_leave: AtomicBool,
}

impl 记录成员网关 {
    fn new() -> Self {
        Self {
            join_calls: AtomicUsize::new(0),
            leave_calls: AtomicUsize::new(0),
            fail_join: AtomicBool::new(false),
            fail_leave: AtomicBool::new(false),
        }
    }
}

impl RoomMembershipGateway for 记录成员网关 {
    fn join<'a>(
        &'a self,
        _room_id: &'a MatrixRoomReference,
    ) -> agent_room_application::ports::PortFuture<'a, MatrixResult<()>> {
        self.join_calls.fetch_add(1, Ordering::SeqCst);
        let failure = self.fail_join.load(Ordering::SeqCst);
        Box::pin(async move {
            if failure {
                Err(MatrixFailure::new(
                    MatrixOperation::Join,
                    MatrixFailureKind::DependencyUnavailable,
                ))
            } else {
                Ok(())
            }
        })
    }

    fn leave<'a>(
        &'a self,
        _room_id: &'a MatrixRoomReference,
    ) -> agent_room_application::ports::PortFuture<'a, MatrixResult<()>> {
        self.leave_calls.fetch_add(1, Ordering::SeqCst);
        let failure = self.fail_leave.load(Ordering::SeqCst);
        Box::pin(async move {
            if failure {
                Err(MatrixFailure::new(
                    MatrixOperation::Leave,
                    MatrixFailureKind::DependencyUnavailable,
                ))
            } else {
                Ok(())
            }
        })
    }
}

struct 记录分配仓储 {
    outcome: Mutex<Option<RoomReservationOutcome>>,
    reservation: Mutex<RoomReservation>,
    transitions: Mutex<Vec<(RoomReservationState, RoomReservationState)>>,
    fail_commit: AtomicBool,
    fail_release: AtomicBool,
}

impl 记录分配仓储 {
    fn reserved() -> Self {
        let reservation = reserved_reservation();
        Self {
            outcome: Mutex::new(Some(RoomReservationOutcome::Reserved {
                reservation: reservation.clone(),
                room: room(),
            })),
            reservation: Mutex::new(reservation),
            transitions: Mutex::new(Vec::new()),
            fail_commit: AtomicBool::new(false),
            fail_release: AtomicBool::new(false),
        }
    }

    fn existing() -> Self {
        let reservation = committed_reservation();
        Self {
            outcome: Mutex::new(Some(RoomReservationOutcome::ExistingAssignment {
                reservation: reservation.clone(),
                room: room(),
            })),
            reservation: Mutex::new(reservation),
            transitions: Mutex::new(Vec::new()),
            fail_commit: AtomicBool::new(false),
            fail_release: AtomicBool::new(false),
        }
    }
}

impl RoomAllocationStore for 记录分配仓储 {
    fn reserve<'a>(
        &'a self,
        _claim: &'a RoomReservationClaim,
    ) -> agent_room_application::ports::PortFuture<'a, RepositoryResult<RoomReservationOutcome>>
    {
        let outcome = self.outcome.lock().expect("预约结果锁可用").clone();
        Box::pin(async move {
            outcome
                .ok_or_else(|| RepositoryError::new("room.reserve", RepositoryErrorKind::Conflict))
        })
    }

    fn transition(
        &self,
        _reservation_id: RoomReservationId,
        expected: RoomReservationState,
        target: RoomReservationState,
        changed_at: UtcMillis,
    ) -> agent_room_application::ports::PortFuture<'_, RepositoryResult<RoomReservation>> {
        self.transitions
            .lock()
            .expect("转换记录锁可用")
            .push((expected, target));
        let should_fail = match target {
            RoomReservationState::Committed => self.fail_commit.load(Ordering::SeqCst),
            RoomReservationState::Released => self.fail_release.load(Ordering::SeqCst),
            RoomReservationState::Reserved | RoomReservationState::Expired => false,
        };
        if should_fail {
            return Box::pin(async {
                Err(RepositoryError::new(
                    "room.transition",
                    RepositoryErrorKind::Unavailable,
                ))
            });
        }
        let mut reservation = self.reservation.lock().expect("预约锁可用");
        if reservation.state() != expected {
            return Box::pin(async {
                Err(RepositoryError::new(
                    "room.transition",
                    RepositoryErrorKind::Conflict,
                ))
            });
        }
        let result = match target {
            RoomReservationState::Committed => reservation.commit(changed_at),
            RoomReservationState::Released => reservation.release(changed_at),
            RoomReservationState::Expired => reservation.expire(changed_at),
            RoomReservationState::Reserved => {
                Err(agent_room_domain::DomainError::InvalidTransition {
                    entity: "room_reservation",
                    from: reservation.state().as_str(),
                    to: "reserved",
                })
            }
        };
        if result.is_err() {
            return Box::pin(async {
                Err(RepositoryError::new(
                    "room.transition",
                    RepositoryErrorKind::Constraint,
                ))
            });
        }
        let updated = reservation.clone();
        Box::pin(async move { Ok(updated) })
    }

    fn expire_pending(
        &self,
        _now: UtcMillis,
        _limit: u16,
    ) -> agent_room_application::ports::PortFuture<'_, RepositoryResult<u16>> {
        Box::pin(async { Ok(0) })
    }
}

#[tokio::test]
async fn 成功加入后才把预约确认为当前分配() {
    let store = Arc::new(记录分配仓储::reserved());
    let matrix = Arc::new(记录成员网关::new());
    let service = service(store.clone(), matrix.clone());

    let outcome = service.join(request()).await.expect("加入成功");

    assert!(matches!(
        outcome,
        JoinLobbyOutcome::Joined {
            kind: LobbyJoinKind::NewAssignment,
            ..
        }
    ));
    assert_eq!(matrix.join_calls.load(Ordering::SeqCst), 1);
    assert_eq!(matrix.leave_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        store.transitions.lock().expect("转换记录锁可用").as_slice(),
        [(
            RoomReservationState::Reserved,
            RoomReservationState::Committed
        )]
    );
}

#[tokio::test]
async fn matrix_加入失败会释放槽位且不会返回虚假已加入() {
    let store = Arc::new(记录分配仓储::reserved());
    let matrix = Arc::new(记录成员网关::new());
    matrix.fail_join.store(true, Ordering::SeqCst);
    let service = service(store.clone(), matrix);

    let failure = service.join(request()).await.expect_err("加入必须失败");

    assert!(matches!(failure, JoinLobbyFailure::MatrixJoin(_)));
    assert_eq!(
        store.transitions.lock().expect("转换记录锁可用").as_slice(),
        [(
            RoomReservationState::Reserved,
            RoomReservationState::Released
        )]
    );
}

#[tokio::test]
async fn 加入失败且补偿失败会保留双重故障而不是吞错() {
    let store = Arc::new(记录分配仓储::reserved());
    store.fail_release.store(true, Ordering::SeqCst);
    let matrix = Arc::new(记录成员网关::new());
    matrix.fail_join.store(true, Ordering::SeqCst);
    let service = service(store, matrix);

    let failure = service.join(request()).await.expect_err("加入必须失败");

    assert!(matches!(
        failure,
        JoinLobbyFailure::MatrixJoinCompensation { .. }
    ));
}

#[tokio::test]
async fn 确认失败会先离开_matrix_再释放预约() {
    let store = Arc::new(记录分配仓储::reserved());
    store.fail_commit.store(true, Ordering::SeqCst);
    let matrix = Arc::new(记录成员网关::new());
    let service = service(store.clone(), matrix.clone());

    let failure = service.join(request()).await.expect_err("确认必须失败");

    assert!(matches!(
        failure,
        JoinLobbyFailure::ConfirmationRolledBack(_)
    ));
    assert_eq!(matrix.leave_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        store.transitions.lock().expect("转换记录锁可用").as_slice(),
        [
            (
                RoomReservationState::Reserved,
                RoomReservationState::Committed
            ),
            (
                RoomReservationState::Reserved,
                RoomReservationState::Released
            )
        ]
    );
}

#[tokio::test]
async fn 恢复已有分配只做幂等加入而不重复占槽() {
    let store = Arc::new(记录分配仓储::existing());
    let matrix = Arc::new(记录成员网关::new());
    let service = service(store.clone(), matrix.clone());

    let outcome = service.join(request()).await.expect("恢复成功");

    assert!(matches!(
        outcome,
        JoinLobbyOutcome::Joined {
            kind: LobbyJoinKind::RecoveredAssignment,
            ..
        }
    ));
    assert_eq!(matrix.join_calls.load(Ordering::SeqCst), 1);
    assert!(store.transitions.lock().expect("转换记录锁可用").is_empty());
}

fn service(store: Arc<记录分配仓储>, matrix: Arc<记录成员网关>) -> JoinLobbyService {
    JoinLobbyService::new(
        JoinLobbyDependencies {
            allocations: store,
            membership: matrix,
            identifiers: Arc::new(测试标识),
            clock: Arc::new(测试时钟),
        },
        LobbyJoinPolicy::new(DurationMillis::new(60_000).expect("预约时间有效")).expect("策略有效"),
    )
}

fn request() -> JoinLobbyRequest {
    JoinLobbyRequest {
        agent_id: AgentId::from_uuid(Uuid::now_v7()),
        agent_instance_id: agent_instance_id(),
        catalog_id: catalog_id(),
        mode: RoomAllocationMode::Automatic,
        preferred_language: None,
        preferred_region: None,
        evidence: RoomAllocationEvidence::default(),
    }
}

fn room() -> RoomInstance {
    RoomInstance::restore(
        room_instance_id(),
        RoomInstanceFields {
            catalog_id: catalog_id(),
            matrix_room_id: MatrixRoomReference::new("!lobby:matrix.test").expect("房间 ID 有效"),
            region: None,
            capacity: RoomCapacity::standard(),
            projected_member_count: 0,
            allocated_slots: 1,
            activity_score_millis: 0,
            state: RoomInstanceState::Active,
        },
    )
    .expect("房间有效")
}

fn reserved_reservation() -> RoomReservation {
    RoomReservation::reserve(
        reservation_id(),
        catalog_id(),
        room_instance_id(),
        agent_instance_id(),
        time(1_000),
        time(61_000),
    )
    .expect("预约有效")
}

fn committed_reservation() -> RoomReservation {
    RoomReservation::restore(
        reservation_id(),
        RoomReservationFields {
            catalog_id: catalog_id(),
            room_instance_id: room_instance_id(),
            agent_instance_id: agent_instance_id(),
            reserved_at: time(1_000),
            expires_at: time(61_000),
            state: RoomReservationState::Committed,
            finalized_at: Some(time(2_000)),
        },
    )
    .expect("已确认分配有效")
}

fn reservation_id() -> RoomReservationId {
    RoomReservationId::from_uuid(
        Uuid::parse_str("01945c1e-7b5a-7c7f-8a28-2de53f56a901").expect("UUID 有效"),
    )
}

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(
        Uuid::parse_str("01945c1e-7b5a-7c7f-8a28-2de53f56a902").expect("UUID 有效"),
    )
}

fn room_instance_id() -> RoomInstanceId {
    RoomInstanceId::from_uuid(
        Uuid::parse_str("01945c1e-7b5a-7c7f-8a28-2de53f56a903").expect("UUID 有效"),
    )
}

fn agent_instance_id() -> AgentInstanceId {
    AgentInstanceId::from_uuid(
        Uuid::parse_str("01945c1e-7b5a-7c7f-8a28-2de53f56a904").expect("UUID 有效"),
    )
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
