use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use agent_room_application::{
    persistence::RepositoryResult,
    ports::{
        Clock, MatrixCreateRoom, MatrixEventId, MatrixFailure, MatrixFailureKind, MatrixOperation,
        MatrixResult, MatrixRoomAliasLocalpart, MatrixRoomId, PortFuture, RoomProvisioningClaim,
        RoomProvisioningClaimOutcome, RoomProvisioningFailureCode, RoomProvisioningGateway,
        RoomProvisioningJob, RoomProvisioningKind, RoomProvisioningStore,
    },
    rooms::{
        LobbyProvisioningDependencies, LobbyProvisioningFailure, LobbyProvisioningFailureStage,
        LobbyProvisioningIdentifierFactory, LobbyProvisioningOutcome, LobbyProvisioningPolicy,
        LobbyProvisioningRequest, LobbyProvisioningService,
    },
};
use agent_room_domain::{
    ids::{RoomCatalogId, RoomInstanceId, RoomProvisioningJobId, RoomProvisioningLeaseId},
    rooms::{
        MatrixRoomReference, RoomCatalog, RoomCatalogFields, RoomCatalogKind, RoomCatalogStatus,
        RoomCatalogVisibility, RoomInstance, RoomLanguage, RoomRegion, RoomSlug,
    },
    time::{DurationMillis, UtcMillis},
};
use uuid::Uuid;

#[derive(Debug)]
struct 测试时钟;

impl Clock for 测试时钟 {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(10_000).expect("测试时间有效")
    }
}

#[derive(Debug)]
struct 测试标识;

impl LobbyProvisioningIdentifierFactory for 测试标识 {
    fn room_provisioning_job_id(&self) -> RoomProvisioningJobId {
        RoomProvisioningJobId::from_uuid(Uuid::now_v7())
    }

    fn room_provisioning_lease_id(&self) -> RoomProvisioningLeaseId {
        RoomProvisioningLeaseId::from_uuid(Uuid::now_v7())
    }

    fn room_instance_id(&self) -> RoomInstanceId {
        RoomInstanceId::from_uuid(Uuid::now_v7())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StoreCall {
    Claim(RoomProvisioningKind),
    Checkpoint(RoomProvisioningKind, String),
    CompleteSpace(String),
    CompleteInstance(String),
    Release(RoomProvisioningKind, RoomProvisioningFailureCode),
}

struct 测试Store {
    catalog: Mutex<RoomCatalog>,
    calls: Mutex<Vec<StoreCall>>,
    busy_kind: Option<RoomProvisioningKind>,
    checkpoints: Vec<(RoomProvisioningKind, MatrixRoomReference)>,
}

impl 测试Store {
    fn new(catalog: RoomCatalog) -> Self {
        Self {
            catalog: Mutex::new(catalog),
            calls: Mutex::new(Vec::new()),
            busy_kind: None,
            checkpoints: Vec::new(),
        }
    }

    fn busy(mut self, kind: RoomProvisioningKind) -> Self {
        self.busy_kind = Some(kind);
        self
    }

    fn with_checkpoint(mut self, kind: RoomProvisioningKind, room_id: &str) -> Self {
        self.checkpoints.push((
            kind,
            MatrixRoomReference::new(room_id).expect("断点房间标识有效"),
        ));
        self
    }

    fn calls(&self) -> Vec<StoreCall> {
        self.calls.lock().expect("调用记录锁可用").clone()
    }

    fn checkpoint(&self, kind: RoomProvisioningKind) -> Option<MatrixRoomReference> {
        self.checkpoints
            .iter()
            .find(|(candidate, _)| *candidate == kind)
            .map(|(_, room_id)| room_id.clone())
    }
}

impl RoomProvisioningStore for 测试Store {
    fn claim<'a>(
        &'a self,
        claim: &'a RoomProvisioningClaim,
    ) -> PortFuture<'a, RepositoryResult<RoomProvisioningClaimOutcome>> {
        Box::pin(async move {
            let kind = claim.target().kind();
            self.calls
                .lock()
                .expect("调用记录锁可用")
                .push(StoreCall::Claim(kind));
            if self.busy_kind == Some(kind) {
                return Ok(RoomProvisioningClaimOutcome::Busy {
                    retry_at: claim.expires_at(),
                });
            }
            Ok(RoomProvisioningClaimOutcome::Claimed(
                RoomProvisioningJob::restore(
                    claim.job_id(),
                    claim.lease_id(),
                    self.catalog.lock().expect("目录锁可用").clone(),
                    claim.target().clone(),
                    claim.alias_localpart().clone(),
                    self.checkpoint(kind),
                    claim.expires_at(),
                ),
            ))
        })
    }

    fn checkpoint_matrix_room<'a>(
        &'a self,
        job: &'a RoomProvisioningJob,
        matrix_room_id: &'a MatrixRoomReference,
        _checkpointed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("调用记录锁可用")
                .push(StoreCall::Checkpoint(
                    job.target().kind(),
                    matrix_room_id.as_str().to_owned(),
                ));
            Ok(())
        })
    }

    fn complete_space<'a>(
        &'a self,
        job: &'a RoomProvisioningJob,
        matrix_space_id: &'a MatrixRoomReference,
        _completed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<RoomCatalog>> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("调用记录锁可用")
                .push(StoreCall::CompleteSpace(
                    matrix_space_id.as_str().to_owned(),
                ));
            let updated = public_catalog(job.catalog().id(), Some(matrix_space_id.clone()));
            *self.catalog.lock().expect("目录锁可用") = updated.clone();
            Ok(updated)
        })
    }

    fn complete_instance<'a>(
        &'a self,
        _job: &'a RoomProvisioningJob,
        room: &'a RoomInstance,
        _completed_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<RoomInstance>> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("调用记录锁可用")
                .push(StoreCall::CompleteInstance(
                    room.matrix_room_id().as_str().to_owned(),
                ));
            Ok(room.clone())
        })
    }

    fn release<'a>(
        &'a self,
        job: &'a RoomProvisioningJob,
        failure: RoomProvisioningFailureCode,
        _released_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("调用记录锁可用")
                .push(StoreCall::Release(job.target().kind(), failure));
            Ok(())
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MatrixCall {
    Create(String),
    Resolve(String),
    Attach { space: String, child: String },
}

struct 测试Matrix {
    create_results: Mutex<VecDeque<MatrixResult<MatrixRoomId>>>,
    resolve_results: Mutex<VecDeque<MatrixResult<MatrixRoomId>>>,
    attach_result: MatrixResult<MatrixEventId>,
    calls: Mutex<Vec<MatrixCall>>,
}

impl 测试Matrix {
    fn new(create_results: Vec<MatrixResult<MatrixRoomId>>) -> Self {
        Self {
            create_results: Mutex::new(create_results.into()),
            resolve_results: Mutex::new(VecDeque::new()),
            attach_result: Ok(matrix_event("$attach:matrix.test")),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn resolving(mut self, results: Vec<MatrixResult<MatrixRoomId>>) -> Self {
        self.resolve_results = Mutex::new(results.into());
        self
    }

    fn failing_attach(mut self, failure: MatrixFailure) -> Self {
        self.attach_result = Err(failure);
        self
    }

    fn calls(&self) -> Vec<MatrixCall> {
        self.calls.lock().expect("Matrix 调用锁可用").clone()
    }
}

impl RoomProvisioningGateway for 测试Matrix {
    fn create_room<'a>(
        &'a self,
        request: &'a MatrixCreateRoom,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("Matrix 调用锁可用")
                .push(MatrixCall::Create(
                    request
                        .alias_localpart()
                        .expect("建房必须携带别名")
                        .as_str()
                        .to_owned(),
                ));
            self.create_results
                .lock()
                .expect("建房结果锁可用")
                .pop_front()
                .expect("测试必须提供建房结果")
        })
    }

    fn resolve_room_alias<'a>(
        &'a self,
        alias_localpart: &'a MatrixRoomAliasLocalpart,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomId>> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("Matrix 调用锁可用")
                .push(MatrixCall::Resolve(alias_localpart.as_str().to_owned()));
            self.resolve_results
                .lock()
                .expect("别名解析结果锁可用")
                .pop_front()
                .expect("测试必须提供别名解析结果")
        })
    }

    fn attach_child<'a>(
        &'a self,
        space_id: &'a MatrixRoomId,
        child_id: &'a MatrixRoomId,
    ) -> PortFuture<'a, MatrixResult<MatrixEventId>> {
        Box::pin(async move {
            self.calls
                .lock()
                .expect("Matrix 调用锁可用")
                .push(MatrixCall::Attach {
                    space: space_id.as_str().to_owned(),
                    child: child_id.as_str().to_owned(),
                });
            self.attach_result.clone()
        })
    }
}

#[tokio::test]
async fn 首次建房对未知提交按别名对账并在挂载后才发布实例() {
    let catalog = public_catalog(RoomCatalogId::from_uuid(Uuid::now_v7()), None);
    let store = Arc::new(测试Store::new(catalog.clone()));
    let matrix = Arc::new(
        测试Matrix::new(vec![
            Err(MatrixFailure::new(
                MatrixOperation::CreateRoom,
                MatrixFailureKind::UnknownCommit,
            )),
            Ok(matrix_room("!instance:matrix.test")),
        ])
        .resolving(vec![Ok(matrix_room("!space:matrix.test"))]),
    );
    let outcome = service(store.clone(), matrix.clone())
        .provision(request(catalog))
        .await
        .expect("建房流程应成功");

    let LobbyProvisioningOutcome::Ready(ready) = outcome else {
        panic!("建房完成后必须返回可用实例")
    };
    let catalog = ready.catalog;
    let room = ready.room;
    assert_eq!(
        catalog.matrix_space_id().expect("目录已有 Space").as_str(),
        "!space:matrix.test"
    );
    assert_eq!(room.matrix_room_id().as_str(), "!instance:matrix.test");
    assert_eq!(room.allocated_slots(), 0);
    assert_eq!(
        matrix.calls(),
        vec![
            MatrixCall::Create(format!(
                "agent-room-space-{}",
                catalog.slug().expect("目录短名存在").as_str()
            )),
            MatrixCall::Resolve(format!(
                "agent-room-space-{}",
                catalog.slug().expect("目录短名存在").as_str()
            )),
            MatrixCall::Create(
                matrix
                    .calls()
                    .iter()
                    .find_map(|call| match call {
                        MatrixCall::Create(alias) if !alias.contains("space") => {
                            Some(alias.clone())
                        }
                        _ => None,
                    })
                    .expect("实例使用确定性别名")
            ),
            MatrixCall::Attach {
                space: "!space:matrix.test".to_owned(),
                child: "!instance:matrix.test".to_owned(),
            },
        ]
    );
    assert!(matches!(
        store.calls().as_slice(),
        [
            StoreCall::Claim(RoomProvisioningKind::Space),
            StoreCall::Checkpoint(RoomProvisioningKind::Space, _),
            StoreCall::CompleteSpace(_),
            StoreCall::Claim(RoomProvisioningKind::Instance),
            StoreCall::Checkpoint(RoomProvisioningKind::Instance, _),
            StoreCall::CompleteInstance(_),
        ]
    ));
}

#[tokio::test]
async fn 已保存_matrix_断点时不重复建房但会幂等重挂_space() {
    let catalog = public_catalog(RoomCatalogId::from_uuid(Uuid::now_v7()), None);
    let store = Arc::new(
        测试Store::new(catalog.clone())
            .with_checkpoint(RoomProvisioningKind::Space, "!space:matrix.test")
            .with_checkpoint(RoomProvisioningKind::Instance, "!instance:matrix.test"),
    );
    let matrix = Arc::new(测试Matrix::new(Vec::new()));
    let outcome = service(store, matrix.clone())
        .provision(request(catalog))
        .await
        .expect("断点续跑应成功");

    assert!(matches!(outcome, LobbyProvisioningOutcome::Ready(_)));
    assert_eq!(
        matrix.calls(),
        vec![MatrixCall::Attach {
            space: "!space:matrix.test".to_owned(),
            child: "!instance:matrix.test".to_owned(),
        }]
    );
}

#[tokio::test]
async fn space_挂载失败会释放可接管任务且绝不发布实例() {
    let catalog = public_catalog(RoomCatalogId::from_uuid(Uuid::now_v7()), None);
    let store = Arc::new(测试Store::new(catalog.clone()));
    let matrix_failure = MatrixFailure::new(
        MatrixOperation::SendStateEvent,
        MatrixFailureKind::DependencyUnavailable,
    );
    let matrix = Arc::new(
        测试Matrix::new(vec![
            Ok(matrix_room("!space:matrix.test")),
            Ok(matrix_room("!instance:matrix.test")),
        ])
        .failing_attach(matrix_failure),
    );
    let failure = service(store.clone(), matrix)
        .provision(request(catalog))
        .await
        .expect_err("挂载失败不得返回假成功");

    assert_eq!(
        failure,
        LobbyProvisioningFailure::Matrix {
            stage: LobbyProvisioningFailureStage::AttachInstance,
            source: matrix_failure,
        }
    );
    assert!(matches!(
        store.calls().last(),
        Some(StoreCall::Release(
            RoomProvisioningKind::Instance,
            RoomProvisioningFailureCode::SpaceAttach
        ))
    ));
    assert!(
        !store
            .calls()
            .iter()
            .any(|call| matches!(call, StoreCall::CompleteInstance(_)))
    );
}

#[tokio::test]
async fn 已有建房租约时返回明确重试时间且不触碰_matrix() {
    let catalog = public_catalog(RoomCatalogId::from_uuid(Uuid::now_v7()), None);
    let store = Arc::new(测试Store::new(catalog.clone()).busy(RoomProvisioningKind::Space));
    let matrix = Arc::new(测试Matrix::new(Vec::new()));
    let outcome = service(store, matrix.clone())
        .provision(request(catalog))
        .await
        .expect("并发建房不是系统错误");

    assert_eq!(
        outcome,
        LobbyProvisioningOutcome::Busy {
            retry_at: UtcMillis::new(40_000).expect("重试时间有效"),
        }
    );
    assert!(matrix.calls().is_empty());
}

fn service(store: Arc<测试Store>, matrix: Arc<测试Matrix>) -> LobbyProvisioningService {
    LobbyProvisioningService::new(
        LobbyProvisioningDependencies {
            store,
            matrix,
            identifiers: Arc::new(测试标识),
            clock: Arc::new(测试时钟),
        },
        LobbyProvisioningPolicy::new(DurationMillis::new(30_000).expect("租约时长有效"))
            .expect("租约策略有效"),
    )
}

fn request(catalog: RoomCatalog) -> LobbyProvisioningRequest {
    LobbyProvisioningRequest {
        catalog,
        preferred_region: Some(RoomRegion::new("ap-southeast").expect("地区有效")),
    }
}

fn public_catalog(id: RoomCatalogId, matrix_space_id: Option<MatrixRoomReference>) -> RoomCatalog {
    RoomCatalog::new(
        id,
        RoomCatalogFields {
            kind: RoomCatalogKind::PublicLobby,
            slug: Some(RoomSlug::new("general").expect("短名有效")),
            name: "General".to_owned(),
            description: "Public agent lobby".to_owned(),
            language: Some(RoomLanguage::new("zh-CN").expect("语言有效")),
            matrix_space_id,
            owner_principal_id: None,
            visibility: RoomCatalogVisibility::Public,
            retention_days: None,
            status: RoomCatalogStatus::Active,
        },
    )
    .expect("公共目录有效")
}

fn matrix_room(value: &str) -> MatrixRoomId {
    MatrixRoomId::new(value).expect("Matrix 房间标识有效")
}

fn matrix_event(value: &str) -> MatrixEventId {
    MatrixEventId::new(value).expect("Matrix 事件标识有效")
}
