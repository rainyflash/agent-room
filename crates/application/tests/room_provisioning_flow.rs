use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use agent_room_application::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        Clock, MatrixCreateRoom, MatrixEventId, MatrixFailure, MatrixFailureKind, MatrixOperation,
        MatrixResult, MatrixRoomAliasLocalpart, MatrixRoomId, MatrixUserId,
        ModerationEffectGateway, ModerationEffectTarget, PortFuture, RoomProvisioningClaim,
        RoomProvisioningClaimOutcome, RoomProvisioningFailureCode, RoomProvisioningGateway,
        RoomProvisioningJob, RoomProvisioningKind, RoomProvisioningStore, StandingModeration,
        StandingModerationSource,
    },
    rooms::{
        LobbyProvisioningDependencies, LobbyProvisioningFailure, LobbyProvisioningFailureStage,
        LobbyProvisioningIdentifierFactory, LobbyProvisioningOutcome, LobbyProvisioningPolicy,
        LobbyProvisioningRequest, LobbyProvisioningService,
    },
};
use agent_room_domain::{
    ids::{
        ModerationActionId, PrincipalId, RoomCatalogId, RoomInstanceId, RoomProvisioningJobId,
        RoomProvisioningLeaseId,
    },
    moderation::{
        ModerationAction, ModerationActionKind, ModerationActionStatus, ModerationReason,
        ModerationTarget, ModerationTargetKind,
    },
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
    /// 补到新分片上的治理（动作、房间、被管的人）。和存储调用记在同一条时间线上，看得出它在发布
    /// 实例（开始接人）之前。
    CarryOver(ModerationActionKind, String, String),
}

struct 测试Store {
    catalog: Mutex<RoomCatalog>,
    calls: Mutex<Vec<StoreCall>>,
    busy_kind: Option<RoomProvisioningKind>,
    /// 和真的存储一样记得住断点：建房失败放掉租约以后，下一次接手同一个 Matrix 房间。
    checkpoints: Mutex<Vec<(RoomProvisioningKind, MatrixRoomReference)>>,
}

impl 测试Store {
    fn new(catalog: RoomCatalog) -> Self {
        Self {
            catalog: Mutex::new(catalog),
            calls: Mutex::new(Vec::new()),
            busy_kind: None,
            checkpoints: Mutex::new(Vec::new()),
        }
    }

    fn busy(mut self, kind: RoomProvisioningKind) -> Self {
        self.busy_kind = Some(kind);
        self
    }

    fn with_checkpoint(self, kind: RoomProvisioningKind, room_id: &str) -> Self {
        self.checkpoints.lock().expect("断点锁可用").push((
            kind,
            MatrixRoomReference::new(room_id).expect("断点房间标识有效"),
        ));
        self
    }

    fn calls(&self) -> Vec<StoreCall> {
        self.calls.lock().expect("调用记录锁可用").clone()
    }

    fn record(&self, call: StoreCall) {
        self.calls.lock().expect("调用记录锁可用").push(call);
    }

    fn checkpoint(&self, kind: RoomProvisioningKind) -> Option<MatrixRoomReference> {
        self.checkpoints
            .lock()
            .expect("断点锁可用")
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
            self.checkpoints
                .lock()
                .expect("断点锁可用")
                .push((job.target().kind(), matrix_room_id.clone()));
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
    Create {
        alias: String,
        retention_days: Option<u16>,
    },
    Resolve(String),
    Attach {
        space: String,
        child: String,
    },
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
                .push(MatrixCall::Create {
                    alias: request
                        .alias_localpart()
                        .expect("建房必须携带别名")
                        .as_str()
                        .to_owned(),
                    retention_days: request.retention_days(),
                });
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

/// 这个大厅的治理记录和落治理的 Matrix 效果。补上的治理记在建房存储的时间线上。
struct 测试治理 {
    store: Arc<测试Store>,
    standing: RepositoryResult<Vec<StandingModeration>>,
    /// 下一次落治理时失败一次。
    failure: Mutex<Option<MatrixFailure>>,
    queries: Mutex<Vec<(RoomCatalogId, UtcMillis)>>,
}

impl 测试治理 {
    fn new(store: Arc<测试Store>, standing: Vec<StandingModeration>) -> Self {
        Self {
            store,
            standing: Ok(standing),
            failure: Mutex::new(None),
            queries: Mutex::new(Vec::new()),
        }
    }

    fn unreadable(store: Arc<测试Store>, failure: RepositoryError) -> Self {
        Self {
            standing: Err(failure),
            ..Self::new(store, Vec::new())
        }
    }

    fn failing_once(self, failure: MatrixFailure) -> Self {
        *self.failure.lock().expect("失败设置锁可用") = Some(failure);
        self
    }

    fn queries(&self) -> Vec<(RoomCatalogId, UtcMillis)> {
        self.queries.lock().expect("查询记录锁可用").clone()
    }
}

impl StandingModerationSource for 测试治理 {
    fn standing_person_actions(
        &self,
        room_catalog_id: RoomCatalogId,
        now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<StandingModeration>>> {
        Box::pin(async move {
            self.queries
                .lock()
                .expect("查询记录锁可用")
                .push((room_catalog_id, now));
            self.standing.clone()
        })
    }
}

impl ModerationEffectGateway for 测试治理 {
    fn apply<'a>(
        &'a self,
        action: &'a ModerationAction,
        target: &'a ModerationEffectTarget,
    ) -> PortFuture<'a, MatrixResult<()>> {
        Box::pin(async move {
            assert_eq!(&target.target, action.target(), "补的是这条治理管的人");
            // 公开大厅的禁言只压低这个人；按私人房间那套会把整个大厅禁言。
            assert_eq!(target.room_kind, RoomCatalogKind::PublicLobby);
            if let Some(failure) = self.failure.lock().expect("失败设置锁可用").take() {
                return Err(failure);
            }
            self.store.record(StoreCall::CarryOver(
                action.kind(),
                target.matrix_room_id.as_str().to_owned(),
                target
                    .target_matrix_user_id
                    .as_ref()
                    .expect("管人的治理带着 Matrix 账号")
                    .as_str()
                    .to_owned(),
            ));
            Ok(())
        })
    }

    fn reverse<'a>(
        &'a self,
        _action: &'a ModerationAction,
        _target: &'a ModerationEffectTarget,
    ) -> PortFuture<'a, MatrixResult<()>> {
        panic!("新分片只补生效的治理，不撤销任何东西")
    }

    fn contains_event<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, MatrixResult<bool>> {
        panic!("隐藏不补到新分片，用不着找消息")
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
            MatrixCall::Create {
                alias: format!(
                    "agent-room-space-{}",
                    catalog.slug().expect("目录短名存在").as_str()
                ),
                retention_days: None,
            },
            MatrixCall::Resolve(format!(
                "agent-room-space-{}",
                catalog.slug().expect("目录短名存在").as_str()
            )),
            MatrixCall::Create {
                alias: matrix
                    .calls()
                    .iter()
                    .find_map(|call| match call {
                        MatrixCall::Create { alias, .. } if !alias.contains("space") => {
                            Some(alias.clone())
                        }
                        _ => None,
                    })
                    .expect("实例使用确定性别名"),
                retention_days: Some(30),
            },
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

#[tokio::test]
async fn 新分片开始接人之前补上这个大厅生效的封禁和禁言() {
    use ModerationActionKind::{Ban, Mute};
    use ModerationActionStatus::Applied;
    let catalog = lobby_with_space();
    let store = Arc::new(测试Store::new(catalog.clone()));
    let matrix = Arc::new(测试Matrix::new(vec![Ok(matrix_room(
        "!instance:matrix.test",
    ))]));
    let moderation = Arc::new(测试治理::new(
        store.clone(),
        vec![
            standing(Ban, Applied, None, "@banned:matrix.test"),
            // 禁言到第 20 秒：此刻还没到期。
            standing(Mute, Applied, Some(20_000), "@muted:matrix.test"),
        ],
    ));
    let outcome = service_with(store.clone(), matrix, moderation.clone())
        .provision(request(catalog.clone()))
        .await
        .expect("补上治理后应发布实例");

    assert!(matches!(outcome, LobbyProvisioningOutcome::Ready(_)));
    assert_eq!(
        moderation.queries(),
        [(catalog.id(), UtcMillis::new(10_000).expect("测试时间有效"))],
        "按这个大厅、此刻的时间读要补的治理"
    );
    assert_eq!(
        store.calls(),
        [
            StoreCall::Claim(RoomProvisioningKind::Instance),
            StoreCall::Checkpoint(
                RoomProvisioningKind::Instance,
                "!instance:matrix.test".to_owned()
            ),
            carried(Ban, "@banned:matrix.test"),
            carried(Mute, "@muted:matrix.test"),
            StoreCall::CompleteInstance("!instance:matrix.test".to_owned()),
        ],
        "补在发布实例（开始接人）之前"
    );
}

#[tokio::test]
async fn 已撤销_已过期和没落成的不补_踢出和隐藏也不补() {
    use ModerationActionKind::{Ban, Hide, Kick, Mute};
    use ModerationActionStatus::{Applied, Failed, Pending, Reversed};
    let catalog = lobby_with_space();
    let store = Arc::new(测试Store::new(catalog.clone()));
    let matrix = Arc::new(测试Matrix::new(vec![Ok(matrix_room(
        "!instance:matrix.test",
    ))]));
    let moderation = Arc::new(测试治理::new(
        store.clone(),
        vec![
            standing(Ban, Reversed, None, "@forgiven:matrix.test"),
            // 禁言到第 5 秒就到期了，此刻是第 10 秒。
            standing(Mute, Applied, Some(5_000), "@expired:matrix.test"),
            standing(Ban, Failed, None, "@failed:matrix.test"),
            standing(Ban, Pending, None, "@pending:matrix.test"),
            standing(Kick, Applied, None, "@kicked:matrix.test"),
            standing(Hide, Applied, None, "@author:matrix.test"),
            standing(Ban, Applied, None, "@banned:matrix.test"),
        ],
    ));
    service_with(store.clone(), matrix, moderation)
        .provision(request(catalog))
        .await
        .expect("补上治理后应发布实例");

    let carried_over: Vec<StoreCall> = store
        .calls()
        .into_iter()
        .filter(|call| matches!(call, StoreCall::CarryOver(..)))
        .collect();
    assert_eq!(
        carried_over,
        [carried(Ban, "@banned:matrix.test")],
        "只补此刻还生效的禁言和封禁"
    );
}

#[tokio::test]
async fn 补不上时放掉建房租约_不发布实例_下一个进大厅的人接着补() {
    use ModerationActionKind::Ban;
    let catalog = lobby_with_space();
    let store = Arc::new(测试Store::new(catalog.clone()));
    let matrix = Arc::new(测试Matrix::new(vec![Ok(matrix_room(
        "!instance:matrix.test",
    ))]));
    let matrix_failure = MatrixFailure::new(
        MatrixOperation::Ban,
        MatrixFailureKind::DependencyUnavailable,
    );
    let moderation = Arc::new(
        测试治理::new(
            store.clone(),
            vec![standing(
                Ban,
                ModerationActionStatus::Applied,
                None,
                "@banned:matrix.test",
            )],
        )
        .failing_once(matrix_failure),
    );
    let service = service_with(store.clone(), matrix.clone(), moderation);

    let failure = service
        .provision(request(catalog.clone()))
        .await
        .expect_err("补不上治理不得发布实例");
    assert_eq!(
        failure,
        LobbyProvisioningFailure::Matrix {
            stage: LobbyProvisioningFailureStage::CarryOverModeration,
            source: matrix_failure,
        }
    );
    assert_eq!(
        store.calls().last(),
        Some(&StoreCall::Release(
            RoomProvisioningKind::Instance,
            RoomProvisioningFailureCode::ModerationCarryOver
        )),
        "和建房间失败一样放掉租约，别人马上能接着建"
    );

    service
        .provision(request(catalog))
        .await
        .expect("下一次接着建应成功");
    assert_eq!(
        store.calls()[3..],
        [
            StoreCall::Claim(RoomProvisioningKind::Instance),
            carried(Ban, "@banned:matrix.test"),
            StoreCall::CompleteInstance("!instance:matrix.test".to_owned()),
        ],
        "接手同一个 Matrix 房间，补上以后才发布"
    );
    assert_eq!(
        matrix
            .calls()
            .iter()
            .filter(|call| matches!(call, MatrixCall::Create { .. }))
            .count(),
        1,
        "Matrix 房间已经记下，不再重建"
    );
}

#[tokio::test]
async fn 读不到要补的治理时不发布实例() {
    let catalog = lobby_with_space();
    let store = Arc::new(测试Store::new(catalog.clone()));
    let matrix = Arc::new(测试Matrix::new(vec![Ok(matrix_room(
        "!instance:matrix.test",
    ))]));
    let unavailable = RepositoryError::new(
        "moderation.standing_person_actions",
        RepositoryErrorKind::Unavailable,
    );
    let moderation = Arc::new(测试治理::unreadable(store.clone(), unavailable.clone()));
    let failure = service_with(store.clone(), matrix, moderation)
        .provision(request(catalog))
        .await
        .expect_err("不知道该补什么时不得发布实例");

    assert_eq!(
        failure,
        LobbyProvisioningFailure::Store {
            stage: LobbyProvisioningFailureStage::CarryOverModeration,
            source: unavailable,
        }
    );
    assert!(
        !store
            .calls()
            .iter()
            .any(|call| matches!(call, StoreCall::CompleteInstance(_))),
    );
}

fn service(store: Arc<测试Store>, matrix: Arc<测试Matrix>) -> LobbyProvisioningService {
    let moderation = Arc::new(测试治理::new(store.clone(), Vec::new()));
    service_with(store, matrix, moderation)
}

fn service_with(
    store: Arc<测试Store>,
    matrix: Arc<测试Matrix>,
    moderation: Arc<测试治理>,
) -> LobbyProvisioningService {
    LobbyProvisioningService::new(
        LobbyProvisioningDependencies {
            store,
            matrix,
            moderation: moderation.clone(),
            moderation_effects: moderation,
            identifiers: Arc::new(测试标识),
            clock: Arc::new(测试时钟),
        },
        LobbyProvisioningPolicy::new(DurationMillis::new(30_000).expect("租约时长有效"))
            .expect("租约策略有效"),
    )
}

/// 已经建好 Space 的公开大厅：建房从分片开始。
fn lobby_with_space() -> RoomCatalog {
    public_catalog(
        RoomCatalogId::from_uuid(Uuid::now_v7()),
        Some(MatrixRoomReference::new("!space:matrix.test").expect("Space 标识有效")),
    )
}

/// 一条治理记录：第 1 秒做的，到期时间和状态由测试给。此刻是第 10 秒。
fn standing(
    kind: ModerationActionKind,
    status: ModerationActionStatus,
    expires_at: Option<i64>,
    matrix_user_id: &str,
) -> StandingModeration {
    let time = |value| UtcMillis::new(value).expect("测试时间有效");
    let target = if kind == ModerationActionKind::Hide {
        ModerationTarget::new(ModerationTargetKind::Event, "$spam:matrix.test")
    } else {
        ModerationTarget::new(
            ModerationTargetKind::Principal,
            PrincipalId::from_uuid(Uuid::now_v7()).to_string(),
        )
    }
    .expect("治理目标有效");
    let action = ModerationAction::restore(
        ModerationActionId::from_uuid(Uuid::now_v7()),
        None,
        PrincipalId::from_uuid(Uuid::now_v7()),
        RoomCatalogId::from_uuid(Uuid::now_v7()),
        kind,
        target,
        ModerationReason::Harassment,
        time(1_000),
        expires_at.map(time),
        status,
        (status == ModerationActionStatus::Failed).then(|| "matrix.unavailable".to_owned()),
        (status == ModerationActionStatus::Reversed).then(|| time(2_000)),
    )
    .expect("治理记录有效");
    StandingModeration {
        action,
        target_matrix_user_id: MatrixUserId::new(matrix_user_id).expect("Matrix 账号有效"),
    }
}

fn carried(kind: ModerationActionKind, matrix_user_id: &str) -> StoreCall {
    StoreCall::CarryOver(
        kind,
        "!instance:matrix.test".to_owned(),
        matrix_user_id.to_owned(),
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
