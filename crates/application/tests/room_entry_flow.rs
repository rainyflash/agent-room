use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind},
    ports::{RoomAllocationEvidence, RoomAllocationMode},
    rooms::{
        EnterLobbyDependencies, EnterLobbyFailure, EnterLobbyOutcome, EnterLobbyService,
        JoinLobbyFailure, JoinLobbyOutcome, JoinLobbyRequest, JoinLobbyResult, LobbyJoinKind,
        LobbyJoinOperation, LobbyProvisioningFailure, LobbyProvisioningFailureStage,
        LobbyProvisioningOperation, LobbyProvisioningOutcome, LobbyProvisioningRequest,
        LobbyProvisioningResult, ProvisionedLobby,
    },
};
use agent_room_domain::{
    ids::{AgentId, AgentInstanceId, RoomCatalogId, RoomInstanceId, RoomReservationId},
    rooms::{
        MatrixRoomReference, RoomCapacity, RoomCatalog, RoomCatalogFields, RoomCatalogKind,
        RoomCatalogStatus, RoomCatalogVisibility, RoomInstance, RoomInstanceFields,
        RoomInstanceState, RoomReservation, RoomReservationFields, RoomReservationState, RoomSlug,
    },
    time::UtcMillis,
};
use uuid::Uuid;

struct 顺序加入操作 {
    outcomes: Mutex<VecDeque<JoinLobbyResult<JoinLobbyOutcome>>>,
    calls: Mutex<Vec<JoinLobbyRequest>>,
}

impl 顺序加入操作 {
    fn new(outcomes: Vec<JoinLobbyResult<JoinLobbyOutcome>>) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.into()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.lock().expect("加入调用锁可用").len()
    }
}

impl LobbyJoinOperation for 顺序加入操作 {
    fn join(
        &self,
        request: JoinLobbyRequest,
    ) -> agent_room_application::ports::PortFuture<'_, JoinLobbyResult<JoinLobbyOutcome>> {
        Box::pin(async move {
            self.calls.lock().expect("加入调用锁可用").push(request);
            self.outcomes
                .lock()
                .expect("加入结果锁可用")
                .pop_front()
                .expect("测试必须提供加入结果")
        })
    }
}

struct 顺序供给操作 {
    outcomes: Mutex<VecDeque<LobbyProvisioningResult<LobbyProvisioningOutcome>>>,
    calls: Mutex<Vec<LobbyProvisioningRequest>>,
}

impl 顺序供给操作 {
    fn new(outcomes: Vec<LobbyProvisioningResult<LobbyProvisioningOutcome>>) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.into()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.lock().expect("供给调用锁可用").len()
    }
}

impl LobbyProvisioningOperation for 顺序供给操作 {
    fn provision(
        &self,
        request: LobbyProvisioningRequest,
    ) -> agent_room_application::ports::PortFuture<
        '_,
        LobbyProvisioningResult<LobbyProvisioningOutcome>,
    > {
        Box::pin(async move {
            self.calls.lock().expect("供给调用锁可用").push(request);
            self.outcomes
                .lock()
                .expect("供给结果锁可用")
                .pop_front()
                .expect("测试必须提供供给结果")
        })
    }
}

#[tokio::test]
async fn 缺房时自动供给并重新预约而不是把内部状态暴露给调用方() {
    let catalog = catalog(catalog_id());
    let joins = Arc::new(顺序加入操作::new(vec![
        Ok(JoinLobbyOutcome::ProvisioningRequired {
            catalog: catalog.clone(),
        }),
        Ok(joined()),
    ]));
    let provisioning = Arc::new(顺序供给操作::new(vec![Ok(
        LobbyProvisioningOutcome::Ready(Box::new(ProvisionedLobby {
            catalog,
            room: room(catalog_id()),
        })),
    )]));
    let outcome = service(joins.clone(), provisioning.clone())
        .enter(request())
        .await
        .expect("自动供给后应加入成功");

    assert!(matches!(
        outcome,
        EnterLobbyOutcome::Joined {
            kind: LobbyJoinKind::NewAssignment,
            ..
        }
    ));
    assert_eq!(joins.call_count(), 2);
    assert_eq!(provisioning.call_count(), 1);
}

#[tokio::test]
async fn 其他进程正在建房时返回精确重试时间且不重复预约() {
    let catalog = catalog(catalog_id());
    let joins = Arc::new(顺序加入操作::new(vec![Ok(
        JoinLobbyOutcome::ProvisioningRequired { catalog },
    )]));
    let retry_at = time(30_000);
    let provisioning = Arc::new(顺序供给操作::new(vec![Ok(
        LobbyProvisioningOutcome::Busy { retry_at },
    )]));
    let outcome = service(joins.clone(), provisioning.clone())
        .enter(request())
        .await
        .expect("并发供给应是可重试结果");

    assert_eq!(outcome, EnterLobbyOutcome::ProvisioningBusy { retry_at });
    assert_eq!(joins.call_count(), 1);
    assert_eq!(provisioning.call_count(), 1);
}

#[tokio::test]
async fn 新实例发布后若容量被抢光会明确要求重试而不是伪造已加入() {
    let catalog = catalog(catalog_id());
    let joins = Arc::new(顺序加入操作::new(vec![
        Ok(JoinLobbyOutcome::ProvisioningRequired {
            catalog: catalog.clone(),
        }),
        Ok(JoinLobbyOutcome::ProvisioningRequired {
            catalog: catalog.clone(),
        }),
    ]));
    let provisioning = Arc::new(顺序供给操作::new(vec![Ok(
        LobbyProvisioningOutcome::Ready(Box::new(ProvisionedLobby {
            catalog,
            room: room(catalog_id()),
        })),
    )]));
    let outcome = service(joins, provisioning)
        .enter(request())
        .await
        .expect("容量竞争不是虚假成功");

    assert_eq!(
        outcome,
        EnterLobbyOutcome::CapacityChanged {
            catalog_id: catalog_id()
        }
    );
}

#[tokio::test]
async fn 加入与供给故障保留原始阶段而不是压成通用错误() {
    let join_failure = JoinLobbyFailure::Allocation(RepositoryError::new(
        "room.reserve",
        RepositoryErrorKind::Unavailable,
    ));
    let joins = Arc::new(顺序加入操作::new(vec![Err(join_failure.clone())]));
    let provisioning = Arc::new(顺序供给操作::new(Vec::new()));
    let failure = service(joins, provisioning)
        .enter(request())
        .await
        .expect_err("加入故障应透传");
    assert_eq!(failure, EnterLobbyFailure::Join(join_failure));

    let catalog = catalog(catalog_id());
    let joins = Arc::new(顺序加入操作::new(vec![Ok(
        JoinLobbyOutcome::ProvisioningRequired { catalog },
    )]));
    let provisioning_failure = LobbyProvisioningFailure::Store {
        stage: LobbyProvisioningFailureStage::ClaimInstance,
        source: RepositoryError::new("room.provision", RepositoryErrorKind::Unavailable),
    };
    let provisioning = Arc::new(顺序供给操作::new(
        vec![Err(provisioning_failure.clone())],
    ));
    let failure = service(joins, provisioning)
        .enter(request())
        .await
        .expect_err("供给故障应透传");
    assert_eq!(
        failure,
        EnterLobbyFailure::Provisioning(provisioning_failure)
    );
}

fn service(
    joins: Arc<顺序加入操作>, provisioning: Arc<顺序供给操作>
) -> EnterLobbyService {
    EnterLobbyService::new(EnterLobbyDependencies {
        joins,
        provisioning,
    })
}

fn request() -> JoinLobbyRequest {
    JoinLobbyRequest {
        agent_id: AgentId::from_uuid(Uuid::from_u128(1)),
        agent_instance_id: AgentInstanceId::from_uuid(Uuid::from_u128(2)),
        catalog_id: catalog_id(),
        mode: RoomAllocationMode::Automatic,
        preferred_language: None,
        preferred_region: None,
        evidence: RoomAllocationEvidence::default(),
    }
}

fn joined() -> JoinLobbyOutcome {
    JoinLobbyOutcome::Joined {
        reservation: reservation(),
        room: room(catalog_id()),
        kind: LobbyJoinKind::NewAssignment,
    }
}

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(3))
}

fn room_instance_id() -> RoomInstanceId {
    RoomInstanceId::from_uuid(Uuid::from_u128(4))
}

fn catalog(id: RoomCatalogId) -> RoomCatalog {
    RoomCatalog::new(
        id,
        RoomCatalogFields {
            kind: RoomCatalogKind::PublicLobby,
            slug: Some(RoomSlug::new("general").expect("短名有效")),
            name: "General".to_owned(),
            description: "Public agent lobby".to_owned(),
            language: None,
            matrix_space_id: Some(
                MatrixRoomReference::new("!space:matrix.test").expect("Space 标识有效"),
            ),
            owner_principal_id: None,
            visibility: RoomCatalogVisibility::Public,
            retention_days: None,
            status: RoomCatalogStatus::Active,
        },
    )
    .expect("目录有效")
}

fn room(catalog_id: RoomCatalogId) -> RoomInstance {
    RoomInstance::restore(
        room_instance_id(),
        RoomInstanceFields {
            catalog_id,
            matrix_room_id: MatrixRoomReference::new("!lobby:matrix.test").expect("房间标识有效"),
            region: None,
            capacity: RoomCapacity::standard(),
            projected_member_count: 1,
            allocated_slots: 1,
            activity_score_millis: 0,
            state: RoomInstanceState::Active,
        },
    )
    .expect("房间实例有效")
}

fn reservation() -> RoomReservation {
    RoomReservation::restore(
        RoomReservationId::from_uuid(Uuid::from_u128(5)),
        RoomReservationFields {
            catalog_id: catalog_id(),
            room_instance_id: room_instance_id(),
            agent_instance_id: AgentInstanceId::from_uuid(Uuid::from_u128(2)),
            reserved_at: time(1_000),
            expires_at: time(61_000),
            state: RoomReservationState::Committed,
            finalized_at: Some(time(2_000)),
        },
    )
    .expect("预约有效")
}

fn time(offset: i64) -> UtcMillis {
    UtcMillis::new(1_700_000_000_000 + offset).expect("时间有效")
}
