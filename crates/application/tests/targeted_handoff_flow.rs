use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    devices::AuthenticatedDevice,
    handoffs::{
        ClaimNextTargetedHandoff, CreateTargetedHandoff, ListTargetedHandoffTargets,
        RecordTargetedHandoffReceiptCommand, TargetedHandoffDependencies,
        TargetedHandoffFailureKind, TargetedHandoffPolicy, TargetedHandoffService,
        TargetedHandoffUseCases,
    },
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        ClaimTargetedHandoff, Clock, ContentAccessMode, ContentAccessPolicy,
        ContentAuthorizationDecision, ContentAuthorizationRequest, ContentAuthorizationResult,
        ContentEventBinding, ContentLifecycleTransition, ContentMembershipAuthorizer,
        ContentRepository, ContentUploadClaim, ContentUploadClaimOutcome, MatrixEventId,
        MatrixRoomId, PortFuture, PrincipalAccount, QueueTargetedHandoff,
        QueueTargetedHandoffOutcome, ReclaimableContentQuery, RecordTargetedHandoffReceipt,
        TargetedHandoffReceiptOutcome, TargetedHandoffRepository,
        TargetedHandoffRequestFingerprint, TargetedHandoffTargetRecord,
    },
};
use agent_room_domain::{
    agents::AgentInstanceStatus,
    content::{
        ContentByteLength, ContentEncryptionMode, ContentLifecycleState, ContentMediaType,
        ContentObject, ContentObjectFields, ContentScanState, ContentStorageKey, Sha256Digest,
    },
    devices::DevicePlatform,
    handoff::{
        HandoffPermission, HandoffPermissions, HandoffPurpose, HandoffSourceEventId,
        TargetedHandoff, TargetedHandoffStatus,
    },
    identity::Principal,
    ids::{AgentId, AgentInstanceId, ContentId, DeviceId, HandoffId, MessageId, PrincipalId},
    rooms::MatrixRoomReference,
    time::{DurationMillis, UtcMillis},
};
use uuid::Uuid;

const NOW: i64 = 1_700_000_000_000;
const HANDOFF_TTL: i64 = 120_000;

struct 固定时钟;

impl Clock for 固定时钟 {
    fn now(&self) -> UtcMillis {
        time(NOW)
    }
}

#[derive(Clone)]
struct 已排队交接 {
    handoff: TargetedHandoff,
    fingerprint: TargetedHandoffRequestFingerprint,
}

struct 交接仓库 {
    targets: Mutex<Vec<TargetedHandoffTargetRecord>>,
    queued: Mutex<Vec<已排队交接>>,
    claims: Mutex<Vec<ClaimTargetedHandoff>>,
    receipts: Mutex<Vec<RecordTargetedHandoffReceipt>>,
}

impl 交接仓库 {
    fn new(targets: Vec<TargetedHandoffTargetRecord>) -> Self {
        Self {
            targets: Mutex::new(targets),
            queued: Mutex::new(Vec::new()),
            claims: Mutex::new(Vec::new()),
            receipts: Mutex::new(Vec::new()),
        }
    }
}

impl TargetedHandoffRepository for 交接仓库 {
    fn list_targets(
        &self,
        _principal_id: PrincipalId,
        _observed_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<TargetedHandoffTargetRecord>>> {
        let targets = self.targets.lock().expect("目标锁有效").clone();
        Box::pin(async move { Ok(targets) })
    }

    fn queue<'a>(
        &'a self,
        request: QueueTargetedHandoff<'a>,
    ) -> PortFuture<'a, RepositoryResult<QueueTargetedHandoffOutcome>> {
        Box::pin(async move {
            let mut queued = self.queued.lock().expect("交接锁有效");
            if let Some(existing) = queued
                .iter()
                .find(|existing| existing.handoff.fields().id == request.handoff.fields().id)
            {
                if existing.fingerprint != request.request_fingerprint {
                    return Err(RepositoryError::new(
                        "handoff.test.queue",
                        RepositoryErrorKind::Conflict,
                    ));
                }
                return Ok(QueueTargetedHandoffOutcome::Existing(
                    existing.handoff.clone(),
                ));
            }
            let stored = request.handoff.clone();
            queued.push(已排队交接 {
                handoff: stored.clone(),
                fingerprint: request.request_fingerprint,
            });
            Ok(QueueTargetedHandoffOutcome::Created(stored))
        })
    }

    fn find_for_principal(
        &self,
        handoff_id: HandoffId,
        principal_id: PrincipalId,
        _observed_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<TargetedHandoff>>> {
        let handoff = self
            .queued
            .lock()
            .expect("交接锁有效")
            .iter()
            .find(|entry| {
                entry.handoff.fields().id == handoff_id
                    && entry.handoff.fields().principal_id == principal_id
            })
            .map(|entry| entry.handoff.clone());
        Box::pin(async move { Ok(handoff) })
    }

    fn revoke(
        &self,
        handoff_id: HandoffId,
        principal_id: PrincipalId,
        revoked_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<TargetedHandoff>>> {
        Box::pin(async move {
            let mut queued = self.queued.lock().expect("交接锁有效");
            let Some(entry) = queued.iter_mut().find(|entry| {
                entry.handoff.fields().id == handoff_id
                    && entry.handoff.fields().principal_id == principal_id
            }) else {
                return Ok(None);
            };
            entry
                .handoff
                .revoke(revoked_at)
                .map_err(|_| test_repository_error())?;
            Ok(Some(entry.handoff.clone()))
        })
    }

    fn claim_next(
        &self,
        request: ClaimTargetedHandoff,
    ) -> PortFuture<'_, RepositoryResult<Option<TargetedHandoff>>> {
        Box::pin(async move {
            self.claims.lock().expect("领取锁有效").push(request);
            let mut queued = self.queued.lock().expect("交接锁有效");
            let Some(entry) = queued.iter_mut().find(|entry| {
                entry.handoff.fields().target_instance_id == request.target_instance_id
                    && entry.handoff.status() == TargetedHandoffStatus::Queued
            }) else {
                return Ok(None);
            };
            entry
                .handoff
                .mark_delivered(request.claimed_at)
                .map_err(|_| test_repository_error())?;
            Ok(Some(entry.handoff.clone()))
        })
    }

    fn record_receipt(
        &self,
        request: RecordTargetedHandoffReceipt,
    ) -> PortFuture<'_, RepositoryResult<Option<TargetedHandoff>>> {
        Box::pin(async move {
            self.receipts
                .lock()
                .expect("回执锁有效")
                .push(request.clone());
            let mut queued = self.queued.lock().expect("交接锁有效");
            let Some(entry) = queued
                .iter_mut()
                .find(|entry| entry.handoff.fields().id == request.handoff_id)
            else {
                return Ok(None);
            };
            match request.outcome {
                TargetedHandoffReceiptOutcome::Consumed => entry
                    .handoff
                    .consume(request.recorded_at)
                    .map_err(|_| test_repository_error())?,
                TargetedHandoffReceiptOutcome::Declined(code) => entry
                    .handoff
                    .decline(code, request.recorded_at)
                    .map_err(|_| test_repository_error())?,
                TargetedHandoffReceiptOutcome::Failed(code) => entry
                    .handoff
                    .fail(code, request.recorded_at)
                    .map_err(|_| test_repository_error())?,
            }
            Ok(Some(entry.handoff.clone()))
        })
    }
}

struct 内容仓库 {
    content: Mutex<Option<ContentObject>>,
    policy: Mutex<Option<ContentAccessPolicy>>,
}

impl ContentRepository for 内容仓库 {
    fn claim_upload<'a>(
        &'a self,
        _claim: &'a ContentUploadClaim,
    ) -> PortFuture<'a, RepositoryResult<ContentUploadClaimOutcome>> {
        unsupported_repository()
    }

    fn find_content(
        &self,
        _content_id: ContentId,
    ) -> PortFuture<'_, RepositoryResult<Option<ContentObject>>> {
        let content = self.content.lock().expect("内容锁有效").clone();
        Box::pin(async move { Ok(content) })
    }

    fn find_access_policy(
        &self,
        _content_id: ContentId,
    ) -> PortFuture<'_, RepositoryResult<Option<ContentAccessPolicy>>> {
        let policy = self.policy.lock().expect("策略锁有效").clone();
        Box::pin(async move { Ok(policy) })
    }

    fn activate(
        &self,
        _content_id: ContentId,
        _activated_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<ContentObject>> {
        unsupported_repository()
    }

    fn record_scan(
        &self,
        _content_id: ContentId,
        _outcome: ContentScanState,
        _scanned_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<ContentObject>> {
        unsupported_repository()
    }

    fn bind_event<'a>(
        &'a self,
        _binding: &'a ContentEventBinding,
    ) -> PortFuture<'a, RepositoryResult<ContentAccessPolicy>> {
        unsupported_repository()
    }

    fn transition<'a>(
        &'a self,
        _transition: &'a ContentLifecycleTransition,
    ) -> PortFuture<'a, RepositoryResult<ContentObject>> {
        unsupported_repository()
    }

    fn list_reclaimable<'a>(
        &'a self,
        _query: &'a ReclaimableContentQuery,
    ) -> PortFuture<'a, RepositoryResult<Vec<ContentObject>>> {
        unsupported_repository()
    }

    fn mark_deleted(
        &self,
        _content_id: ContentId,
        _deleted_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<ContentObject>> {
        unsupported_repository()
    }
}

struct 房间授权器 {
    decision: Mutex<ContentAuthorizationResult<ContentAuthorizationDecision>>,
    requests: Mutex<Vec<ContentAuthorizationRequest>>,
}

impl ContentMembershipAuthorizer for 房间授权器 {
    fn authorize<'a>(
        &'a self,
        request: &'a ContentAuthorizationRequest,
    ) -> PortFuture<'a, ContentAuthorizationResult<ContentAuthorizationDecision>> {
        self.requests
            .lock()
            .expect("授权请求锁有效")
            .push(request.clone());
        let decision = self.decision.lock().expect("授权决策锁有效").clone();
        Box::pin(async move { decision })
    }
}

#[tokio::test]
async fn 目标目录同时展示在线与离线实例且不依赖本机_bridge() {
    let fixture = Fixture::new();
    let online = fixture.target(AgentInstanceStatus::Online, Some(NOW + 60_000));
    let offline = TargetedHandoffTargetRecord {
        instance_id: AgentInstanceId::from_uuid(Uuid::now_v7()),
        device_id: DeviceId::from_uuid(Uuid::now_v7()),
        instance_status: AgentInstanceStatus::Offline,
        lease_expires_at: None,
        ..online.clone()
    };
    let runtime = fixture.runtime(vec![online, offline]);

    let targets = runtime
        .service
        .list_targets(ListTargetedHandoffTargets {
            actor: fixture.human(NOW + 60_000),
            room_id: fixture.room_reference(),
        })
        .await
        .expect("目录查询成功");

    assert_eq!(targets.len(), 2);
    assert!(targets[0].online);
    assert!(!targets[1].online);
    assert_eq!(
        runtime
            .authorizer
            .requests
            .lock()
            .expect("授权锁有效")
            .len(),
        1
    );
}

#[tokio::test]
async fn 离线目标可以排队且相同幂等键只建立一次交接() {
    let fixture = Fixture::new();
    let target = fixture.target(AgentInstanceStatus::Offline, None);
    let runtime = fixture.runtime(vec![target]);
    let request = fixture.create_request(NOW + HANDOFF_TTL);

    let created = runtime
        .service
        .create(request.clone())
        .await
        .expect("离线排队成功");
    let replay = runtime.service.create(request).await.expect("幂等重放成功");

    assert!(created.created);
    assert!(!replay.created);
    assert_eq!(created.handoff.status(), TargetedHandoffStatus::Queued);
    let queued = runtime.store.queued.lock().expect("交接锁有效");
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].handoff.fields().source_message_id,
        fixture.message
    );
}

#[tokio::test]
async fn 过期人类会话和伪造来源都在写队列前失败() {
    let fixture = Fixture::new();
    let runtime = fixture.runtime(vec![fixture.target(AgentInstanceStatus::Online, None)]);
    let mut expired = fixture.create_request(NOW + HANDOFF_TTL);
    expired.actor = fixture.human(NOW);
    let failure = runtime
        .service
        .create(expired)
        .await
        .expect_err("过期会话必须拒绝");
    assert_eq!(failure.kind(), TargetedHandoffFailureKind::Unauthorized);

    let wrong_policy = ContentAccessPolicy::restore(
        fixture.content,
        MatrixRoomId::new(fixture.room.clone()).expect("房间有效"),
        Some(MatrixEventId::new("$other:example.test").expect("事件有效")),
        ContentAccessMode::RoomMember,
        time(NOW - 1_000),
        None,
    )
    .expect("策略有效");
    *runtime.content.policy.lock().expect("策略锁有效") = Some(wrong_policy);
    let failure = runtime
        .service
        .create(fixture.create_request(NOW + HANDOFF_TTL))
        .await
        .expect_err("伪造事件绑定必须拒绝");

    assert_eq!(failure.kind(), TargetedHandoffFailureKind::InvalidSource);
    assert!(runtime.store.queued.lock().expect("交接锁有效").is_empty());
}

#[tokio::test]
async fn 设备签名主体只能领取指定实例并提交不可改写的消费回执() {
    let fixture = Fixture::new();
    let runtime = fixture.runtime(vec![fixture.target(AgentInstanceStatus::Offline, None)]);
    let created = runtime
        .service
        .create(fixture.create_request(NOW + HANDOFF_TTL))
        .await
        .expect("先排队");
    let actor = fixture.device(NOW + 60_000);

    let claimed = runtime
        .service
        .claim_next(ClaimNextTargetedHandoff {
            actor: actor.clone(),
            target_instance_id: fixture.instance,
        })
        .await
        .expect("领取成功")
        .expect("存在待领取交接");
    assert_eq!(claimed.status(), TargetedHandoffStatus::Delivered);

    let consumed = runtime
        .service
        .record_receipt(RecordTargetedHandoffReceiptCommand {
            actor,
            target_instance_id: fixture.instance,
            handoff_id: created.handoff.fields().id,
            outcome: TargetedHandoffReceiptOutcome::Consumed,
        })
        .await
        .expect("消费回执成功");
    assert_eq!(consumed.status(), TargetedHandoffStatus::Consumed);

    let claim = runtime.store.claims.lock().expect("领取锁有效")[0];
    assert_eq!(claim.principal_id, fixture.principal);
    assert_eq!(claim.device_id, fixture.device_id);
    assert_eq!(claim.target_instance_id, fixture.instance);
}

struct RuntimeFixture {
    service: TargetedHandoffService,
    store: Arc<交接仓库>,
    content: Arc<内容仓库>,
    authorizer: Arc<房间授权器>,
}

struct Fixture {
    principal: PrincipalId,
    agent: AgentId,
    instance: AgentInstanceId,
    device_id: DeviceId,
    content: ContentId,
    message: MessageId,
    room: String,
    event: String,
}

impl Fixture {
    fn new() -> Self {
        Self {
            principal: PrincipalId::from_uuid(Uuid::now_v7()),
            agent: AgentId::from_uuid(Uuid::now_v7()),
            instance: AgentInstanceId::from_uuid(Uuid::now_v7()),
            device_id: DeviceId::from_uuid(Uuid::now_v7()),
            content: ContentId::from_uuid(Uuid::now_v7()),
            message: MessageId::from_uuid(Uuid::now_v7()),
            room: "!builders:example.test".to_owned(),
            event: "$source:example.test".to_owned(),
        }
    }

    fn runtime(&self, targets: Vec<TargetedHandoffTargetRecord>) -> RuntimeFixture {
        let store = Arc::new(交接仓库::new(targets));
        let content = Arc::new(内容仓库 {
            content: Mutex::new(Some(self.content_object())),
            policy: Mutex::new(Some(self.content_policy())),
        });
        let authorizer = Arc::new(房间授权器 {
            decision: Mutex::new(Ok(ContentAuthorizationDecision::Allowed)),
            requests: Mutex::new(Vec::new()),
        });
        let service = TargetedHandoffService::new(TargetedHandoffDependencies {
            store: store.clone(),
            content: content.clone(),
            authorizer: authorizer.clone(),
            clock: Arc::new(固定时钟),
            policy: TargetedHandoffPolicy::new(
                DurationMillis::new(60_000).expect("最短期限有效"),
                DurationMillis::new(86_400_000).expect("最长期限有效"),
            )
            .expect("期限策略有效"),
        });
        RuntimeFixture {
            service,
            store,
            content,
            authorizer,
        }
    }

    fn target(
        &self,
        status: AgentInstanceStatus,
        lease_expires_at: Option<i64>,
    ) -> TargetedHandoffTargetRecord {
        TargetedHandoffTargetRecord {
            instance_id: self.instance,
            agent_id: self.agent,
            agent_display_name: "规划 Agent".to_owned(),
            agent_avatar_content_id: None,
            device_id: self.device_id,
            device_label: "工作站".to_owned(),
            device_platform: DevicePlatform::Windows,
            instance_status: status,
            lease_expires_at: lease_expires_at.map(time),
            last_seen_at: Some(time(NOW - 500)),
            adapter_type: "codex".to_owned(),
            capability_version: "1".to_owned(),
        }
    }

    fn human(&self, expires_at: i64) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            principal_id: self.principal,
            matrix_user_id: "@owner:example.test".to_owned(),
            display_name: "用户".to_owned(),
            locale: "zh-CN".to_owned(),
            authenticated_at: time(NOW - 1_000),
            expires_at: time(expires_at),
            recently_authenticated: true,
        }
    }

    fn device(&self, expires_at: i64) -> AuthenticatedDevice {
        AuthenticatedDevice {
            account: PrincipalAccount {
                principal: Principal::new(self.principal),
                matrix_user_id: "@owner:example.test".to_owned(),
                display_name: "用户".to_owned(),
                avatar_content_id: None,
                locale: "zh-CN".to_owned(),
            },
            device_id: self.device_id,
            access_token_expires_at: time(expires_at),
        }
    }

    fn create_request(&self, expires_at: i64) -> CreateTargetedHandoff {
        CreateTargetedHandoff {
            handoff_id: HandoffId::from_uuid(Uuid::now_v7()),
            actor: self.human(NOW + 60_000),
            source_room_id: self.room_reference(),
            source_event_id: HandoffSourceEventId::new(self.event.clone()).expect("事件有效"),
            source_message_id: self.message,
            target_instance_id: self.instance,
            content_id: self.content,
            permissions: HandoffPermissions::new([
                HandoffPermission::ReadText,
                HandoffPermission::IncludeMetadata,
            ])
            .expect("权限有效"),
            purpose: HandoffPurpose::Summarize,
            expires_at: time(expires_at),
        }
    }

    fn room_reference(&self) -> MatrixRoomReference {
        MatrixRoomReference::new(self.room.clone()).expect("房间有效")
    }

    fn content_object(&self) -> ContentObject {
        ContentObject::restore(ContentObjectFields {
            id: self.content,
            owner_principal_id: self.principal,
            storage_key: ContentStorageKey::new(format!("content/{}/opaque-object", self.content))
                .expect("对象键有效"),
            digest: Sha256Digest::from_bytes([0x42; 32]),
            byte_length: ContentByteLength::new(256).expect("长度有效"),
            media_type: ContentMediaType::new("text/markdown").expect("媒体类型有效"),
            encryption_mode: ContentEncryptionMode::ServerSide,
            scan_state: ContentScanState::Clean,
            lifecycle_state: ContentLifecycleState::Active,
            expires_at: Some(time(NOW + 86_400_000)),
            created_at: time(NOW - 1_000),
            deleted_at: None,
        })
        .expect("内容有效")
    }

    fn content_policy(&self) -> ContentAccessPolicy {
        ContentAccessPolicy::restore(
            self.content,
            MatrixRoomId::new(self.room.clone()).expect("房间有效"),
            Some(MatrixEventId::new(self.event.clone()).expect("事件有效")),
            ContentAccessMode::RoomMember,
            time(NOW - 1_000),
            None,
        )
        .expect("策略有效")
    }
}

fn unsupported_repository<'a, T>() -> PortFuture<'a, RepositoryResult<T>> {
    Box::pin(async { Err(test_repository_error()) })
}

fn test_repository_error() -> RepositoryError {
    RepositoryError::new("handoff.test.unsupported", RepositoryErrorKind::Unavailable)
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
