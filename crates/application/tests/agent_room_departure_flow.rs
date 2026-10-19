//! 撤销电脑以后它的 Agent 离开房间（specs/computer-revocation/design.md）。

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use agent_room_application::{
    agent_room_departures::{
        AgentRoomDepartureDependencies, AgentRoomDepartureService, AgentRoomDepartureUseCases,
        DepartureRetry, DepartureRound,
    },
    persistence::RepositoryResult,
    ports::{
        AgentRoomKind, AgentRoomMembershipFactory, AgentRoomPlacement, AgentRoomPlacementStore,
        Clock, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId,
        MatrixUserId, PortFuture, PrivateMatrixMembership, PrivateMatrixSpeakingAssignment,
        PrivateRoomMatrixGateway, RevokedAgentDepartureAudit, RevokedAgentDepartureAuditOutcome,
        RevokedAgentDepartureClaim, RevokedAgentDepartureFacts, RevokedAgentDepartureStore,
        RoomAllocationStore, RoomMembershipGateway, RoomReservationClaim, RoomReservationOutcome,
    },
};
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, DeviceId, RoomCatalogId, RoomInstanceId, RoomReservationId},
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

/// 排着队的几条，领一条给一条；每条配好做之前看到的样子。
struct 假队列 {
    pending: Mutex<
        VecDeque<(
            RevokedAgentDepartureClaim,
            Option<RevokedAgentDepartureFacts>,
        )>,
    >,
    facts: Mutex<Vec<(RoomInstanceId, Option<RevokedAgentDepartureFacts>)>>,
    claimed_for: Mutex<Vec<Option<DeviceId>>>,
    audits: Mutex<Vec<RevokedAgentDepartureAudit>>,
    log: Arc<调用记录>,
}

impl 假队列 {
    fn new(
        entries: Vec<(
            RevokedAgentDepartureClaim,
            Option<RevokedAgentDepartureFacts>,
        )>,
        log: Arc<调用记录>,
    ) -> Self {
        Self {
            pending: Mutex::new(entries.into_iter().collect()),
            facts: Mutex::new(Vec::new()),
            claimed_for: Mutex::new(Vec::new()),
            audits: Mutex::new(Vec::new()),
            log,
        }
    }
}

impl RevokedAgentDepartureStore for 假队列 {
    fn claim_departure(
        &self,
        device_id: Option<DeviceId>,
        now: UtcMillis,
        lease_until: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<RevokedAgentDepartureClaim>>> {
        assert_eq!(now, time(NOW));
        assert_eq!(lease_until, time(NOW + 120_000), "租约 2 分钟");
        self.claimed_for.lock().expect("锁可用").push(device_id);
        let next = self.pending.lock().expect("锁可用").pop_front();
        let claim = next.map(|(claim, facts)| {
            self.facts
                .lock()
                .expect("锁可用")
                .push((claim.room_instance_id, facts));
            claim
        });
        Box::pin(async move { Ok(claim) })
    }

    fn departure_facts<'a>(
        &'a self,
        claim: &'a RevokedAgentDepartureClaim,
    ) -> PortFuture<'a, RepositoryResult<Option<RevokedAgentDepartureFacts>>> {
        let facts = self
            .facts
            .lock()
            .expect("锁可用")
            .iter()
            .find(|(room, _)| *room == claim.room_instance_id)
            .and_then(|(_, facts)| facts.clone());
        Box::pin(async move { Ok(facts) })
    }

    fn complete_departure<'a>(
        &'a self,
        claim: &'a RevokedAgentDepartureClaim,
        audit: Option<&'a RevokedAgentDepartureAudit>,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        self.log
            .记(format!("complete {}", room_number(claim.room_instance_id)));
        if let Some(audit) = audit {
            self.audits.lock().expect("锁可用").push(audit.clone());
        }
        Box::pin(async { Ok(true) })
    }

    fn retry_departure<'a>(
        &'a self,
        claim: &'a RevokedAgentDepartureClaim,
        retry_at: UtcMillis,
        failure_code: &'static str,
        audit: Option<&'a RevokedAgentDepartureAudit>,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        self.log.记(format!(
            "retry {} +{} {failure_code}",
            room_number(claim.room_instance_id),
            retry_at.value() - NOW
        ));
        if let Some(audit) = audit {
            self.audits.lock().expect("锁可用").push(audit.clone());
        }
        Box::pin(async { Ok(true) })
    }
}

struct 记录成员(Arc<调用记录>);

impl AgentRoomPlacementStore for 记录成员 {
    fn placements(
        &self,
        _agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Vec<AgentRoomPlacement>>> {
        Box::pin(async { unreachable!("按记下的房间做，不列位置") })
    }

    fn matrix_user(
        &self,
        _agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<Option<MatrixUserId>>> {
        Box::pin(async { unreachable!("Matrix 账号随事实一起读") })
    }

    fn forget_agent_member(
        &self,
        catalog_id: RoomCatalogId,
        agent_id: AgentId,
    ) -> PortFuture<'_, RepositoryResult<bool>> {
        assert_eq!(agent_id, agent());
        self.0
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
    service: AgentRoomDepartureService,
    queue: Arc<假队列>,
    log: Arc<调用记录>,
}

fn 搭场景(
    entries: Vec<(
        RevokedAgentDepartureClaim,
        Option<RevokedAgentDepartureFacts>,
    )>,
    leave_failure: Option<MatrixFailureKind>,
) -> 场景 {
    let log = Arc::new(调用记录::default());
    let queue = Arc::new(假队列::new(entries, log.clone()));
    let service = AgentRoomDepartureService::new(AgentRoomDepartureDependencies {
        departures: queue.clone(),
        placements: Arc::new(记录成员(log.clone())),
        allocations: Arc::new(记录分配(log.clone())),
        private_matrix: Arc::new(记录私人Matrix(log.clone())),
        memberships: Arc::new(离开工厂(Arc::new(记录离开 {
            log: log.clone(),
            failure: leave_failure,
        }))),
        clock: Arc::new(固定时钟),
    });
    场景 {
        service,
        queue,
        log,
    }
}

#[tokio::test]
async fn 只在这台电脑上的_agent_离开私人房间_收回发言_删成员记录_最后放掉位置() {
    let scene = 搭场景(
        vec![(
            claim(1, None),
            Some(facts(1, AgentRoomKind::PrivateRoom, false, &[7])),
        )],
        None,
    );

    let round = scene
        .service
        .depart_for_device(device())
        .await
        .expect("一轮做完");

    assert_eq!(round.left, 1);
    assert_eq!(round.released_only, 0);
    assert!(round.retrying.is_empty());
    assert_eq!(
        scene.log.全部(),
        [
            "speaking !room1:matrix.test false",
            "leave !room1:matrix.test",
            "forget 1001",
            "release 7",
            "complete 1",
        ]
    );
    assert_eq!(
        scene.queue.claimed_for.lock().expect("锁可用").as_slice(),
        [Some(device()), Some(device())],
        "撤销的请求里只领这台电脑记下的，领到没有为止"
    );
    assert_eq!(
        scene.queue.audits.lock().expect("锁可用").as_slice(),
        [RevokedAgentDepartureAudit {
            occurred_at: time(NOW),
            catalog_id: catalog(1),
            matrix_room_id: matrix_room(1),
            outcome: RevokedAgentDepartureAuditOutcome::Departed {
                left_room: true,
                released_reservations: 1,
            },
        }]
    );
}

#[tokio::test]
async fn 公开大厅不动发言级别也没有成员记录() {
    let scene = 搭场景(
        vec![(
            claim(2, None),
            Some(facts(2, AgentRoomKind::PublicLobby, false, &[8, 9])),
        )],
        None,
    );

    let round = scene.service.depart_due().await.expect("一轮做完");

    assert_eq!(round.left, 1);
    assert_eq!(
        scene.log.全部(),
        [
            "leave !room2:matrix.test",
            "release 8",
            "release 9",
            "complete 2"
        ]
    );
    assert_eq!(
        scene.queue.claimed_for.lock().expect("锁可用").as_slice(),
        [None, None],
        "定时任务领所有电脑的"
    );
}

#[tokio::test]
async fn 同一个房间里别的电脑上还有连接时不离开_只放掉撤销的连接占的位置() {
    let scene = 搭场景(
        vec![(
            claim(3, None),
            Some(facts(3, AgentRoomKind::PrivateRoom, true, &[10])),
        )],
        None,
    );

    let round = scene.service.depart_due().await.expect("一轮做完");

    assert_eq!(round.left, 0);
    assert_eq!(round.released_only, 1);
    assert_eq!(scene.log.全部(), ["release 10", "complete 3"]);
    assert_eq!(
        scene.queue.audits.lock().expect("锁可用")[0].outcome,
        RevokedAgentDepartureAuditOutcome::Departed {
            left_room: false,
            released_reservations: 1,
        }
    );
}

#[tokio::test]
async fn 已经不占位置或者是网络_agent_时什么都不动_直接删掉这一条() {
    let scene = 搭场景(
        vec![
            (
                claim(4, None),
                Some(facts(4, AgentRoomKind::PublicLobby, false, &[])),
            ),
            (claim(5, None), None),
        ],
        None,
    );

    let round = scene.service.depart_due().await.expect("一轮做完");

    assert_eq!(round, DepartureRound::default());
    assert_eq!(scene.log.全部(), ["complete 4", "complete 5"]);
    assert!(scene.queue.audits.lock().expect("锁可用").is_empty());
}

#[tokio::test]
async fn matrix_上已经不在房间里时照样放掉位置() {
    let scene = 搭场景(
        vec![(
            claim(6, None),
            Some(facts(6, AgentRoomKind::PublicLobby, false, &[11])),
        )],
        Some(MatrixFailureKind::Forbidden),
    );

    let round = scene.service.depart_due().await.expect("一轮做完");

    assert_eq!(round.left, 1);
    assert_eq!(
        scene.log.全部(),
        ["leave !room6:matrix.test", "release 11", "complete 6"]
    );
}

#[tokio::test]
async fn 聊天服务器不通时不放位置_往后退避_第一次写审计_这一轮先停() {
    let scene = 搭场景(
        vec![
            (
                claim(7, None),
                Some(facts(7, AgentRoomKind::PublicLobby, false, &[12])),
            ),
            (
                claim(8, None),
                Some(facts(8, AgentRoomKind::PublicLobby, false, &[13])),
            ),
        ],
        Some(MatrixFailureKind::DependencyUnavailable),
    );

    let round = scene.service.depart_due().await.expect("一轮做完");

    assert_eq!(round.left, 0);
    assert_eq!(
        round.retrying,
        [DepartureRetry {
            agent_id: agent(),
            room_instance_id: room_instance(7),
            failure_code: "matrix.unavailable",
        }]
    );
    assert_eq!(
        scene.log.全部(),
        [
            "leave !room7:matrix.test",
            "retry 7 +30000 matrix.unavailable"
        ],
        "第二条留给下一轮"
    );
    assert_eq!(
        scene.queue.audits.lock().expect("锁可用")[0].outcome,
        RevokedAgentDepartureAuditOutcome::Failed {
            failure_code: "matrix.unavailable"
        }
    );
}

#[tokio::test]
async fn 又一次没做成时退避翻倍_不再写审计_别的失败不停下这一轮() {
    let mut retried = claim(9, Some("matrix.timeout"));
    retried.attempt = 3;
    let scene = 搭场景(
        vec![
            (
                retried,
                Some(facts(9, AgentRoomKind::PublicLobby, false, &[14])),
            ),
            (
                claim(10, None),
                Some(facts(10, AgentRoomKind::PublicLobby, false, &[15])),
            ),
        ],
        Some(MatrixFailureKind::Conflict),
    );

    let round = scene.service.depart_due().await.expect("一轮做完");

    assert_eq!(round.retrying.len(), 2);
    assert_eq!(
        scene.log.全部(),
        [
            "leave !room9:matrix.test",
            "retry 9 +120000 matrix.conflict",
            "leave !room10:matrix.test",
            "retry 10 +30000 matrix.conflict",
        ]
    );
    assert_eq!(
        scene.queue.audits.lock().expect("锁可用").len(),
        1,
        "只有第一次没做成的那条写审计"
    );
}

fn claim(room: u128, previous_failure_code: Option<&str>) -> RevokedAgentDepartureClaim {
    RevokedAgentDepartureClaim {
        agent_id: agent(),
        room_instance_id: room_instance(room),
        device_id: device(),
        attempt: 1,
        previous_failure_code: previous_failure_code.map(str::to_owned),
    }
}

fn facts(
    room: u128,
    kind: AgentRoomKind,
    live_connection_remains: bool,
    revoked_reservations: &[u128],
) -> RevokedAgentDepartureFacts {
    RevokedAgentDepartureFacts {
        matrix_user_id: matrix_user(),
        catalog_id: catalog(room),
        matrix_room_id: matrix_room(room),
        kind,
        status: RoomCatalogStatus::Active,
        live_connection_remains,
        revoked_reservations: revoked_reservations
            .iter()
            .map(|value| RoomReservationId::from_uuid(Uuid::from_u128(*value)))
            .collect(),
    }
}

fn released(reservation_id: RoomReservationId, at: UtcMillis) -> RoomReservation {
    RoomReservation::restore(
        reservation_id,
        RoomReservationFields {
            catalog_id: catalog(1),
            room_instance_id: room_instance(1),
            agent_instance_id: AgentInstanceId::from_uuid(Uuid::from_u128(2)),
            reserved_at: time(NOW - 1_000),
            expires_at: time(NOW + 59_000),
            state: RoomReservationState::Released,
            finalized_at: Some(at),
        },
    )
    .expect("预约有效")
}

fn room_number(room: RoomInstanceId) -> u128 {
    room.as_uuid().as_u128()
}

fn matrix_room(room: u128) -> MatrixRoomReference {
    MatrixRoomReference::new(format!("!room{room}:matrix.test")).expect("房间有效")
}

fn matrix_user() -> MatrixUserId {
    MatrixUserId::new("@_agent_00000000000000000000000000000001:matrix.test")
        .expect("Matrix 用户有效")
}

fn agent() -> AgentId {
    AgentId::from_uuid(Uuid::from_u128(1))
}

fn device() -> DeviceId {
    DeviceId::from_uuid(Uuid::from_u128(3))
}

fn room_instance(room: u128) -> RoomInstanceId {
    RoomInstanceId::from_uuid(Uuid::from_u128(room))
}

fn catalog(room: u128) -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(1000 + room))
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("时间有效")
}
