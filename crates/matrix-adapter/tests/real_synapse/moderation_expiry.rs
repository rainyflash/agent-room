//! 带时限的禁言到期，后台解除（`specs/moderation/timed-mute-expiry.md` 第 2 步）。
//!
//! 应用层的到期解除接真实 Synapse，治理记录放在内存里。两个公开大厅分片里禁言一个人，到期以后他在两个
//! 分片里都能说话，旁人不受影响；同一个人还有一条不限时的禁言时，到期的那条只记成解除，他还是说不了。

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
    },
};

use agent_room_application::{
    moderation::{ModerationDependencies, ModerationExpiryUseCases, ModerationService},
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        Clock, ModerationActionReservationOutcome, ModerationAuthority, ModerationExpiryClaim,
        ModerationExpiryRepository, ModerationExpiryReschedule, ModerationIdentifierFactory,
        ModerationMuteLedger, ModerationMuteLock, ModerationMuteStanding, ModerationReportPolicy,
        ModerationReportSubmissionOutcome, ModerationRepository, ModerationRoomContext, PortFuture,
    },
};
use agent_room_domain::{
    ids::{AuditEventId, ModerationCaseId},
    moderation::{ModerationActionStatus, ModerationAuditEvent, ModerationCase, ModerationRole},
};

use super::{public_lobby_moderation::create_lobby_shard, *};

const START: i64 = 1_800_000_000_000;
const HOUR: i64 = 3_600_000;

#[tokio::test]
#[ignore = "需要由 tools/matrix.py 提供真实 Synapse 治理配置"]
async fn 真实_synapse_禁言到期后台解除_两个分片都能说话_还有不限时的禁言时照样禁着() {
    let base_url = required_environment("AGENT_ROOM_MATRIX_TEST_BASE_URL");
    let provisioner = Arc::new(application_service_provisioner(
        &base_url,
        required_environment("AGENT_ROOM_MATRIX_TEST_APPSERVICE_TOKEN"),
    ));
    let factory = factory(&base_url, TEST_REQUEST_TIMEOUT, 5);
    let speaker = managed_user(&provisioner, &factory).await;
    let bystander = managed_user(&provisioner, &factory).await;
    let shards = vec![
        create_lobby_shard(&provisioner).await,
        create_lobby_shard(&provisioner).await,
    ];
    for room in &shards {
        join_with_retry(speaker.gateway(), room).await;
        join_with_retry(bystander.gateway(), room).await;
    }
    let ledger = Arc::new(MemoryLedger::new(
        shards.clone(),
        speaker.session().metadata().user_id().clone(),
    ));
    let clock = Arc::new(TestClock(AtomicI64::new(START)));
    let service = expiry_service(&ledger, provisioner.clone(), &clock);

    // 一条一小时的禁言：两个分片都落下，他说不了，旁人照常。
    let timed = ledger.applied_mute(&clock, Some(HOUR));
    mute_everywhere(&provisioner, &ledger, &timed).await;
    assert_silenced(&speaker, &bystander, &shards).await;

    clock.advance(HOUR);
    let outcome = service.expire_due_actions().await.expect("到期解除应跑完");
    assert_eq!((outcome.expired, outcome.retrying.len()), (1, 0));
    assert_eq!(ledger.status(&timed), ModerationActionStatus::Reversed);
    for room in &shards {
        send_with_retry(
            speaker.gateway(),
            room,
            &message_event(unique_value("expiry-lifted"), "到期以后又能说话"),
        )
        .await;
    }

    // 再禁两回：一条一小时的，一条不限时的。一小时的到期以后，他还是说不了。
    let short = ledger.applied_mute(&clock, Some(HOUR));
    let indefinite = ledger.applied_mute(&clock, None);
    mute_everywhere(&provisioner, &ledger, &short).await;
    clock.advance(HOUR);
    let outcome = service.expire_due_actions().await.expect("到期解除应跑完");
    assert_eq!((outcome.expired, outcome.retrying.len()), (1, 0));
    assert_eq!(ledger.status(&short), ModerationActionStatus::Reversed);
    assert_eq!(ledger.status(&indefinite), ModerationActionStatus::Applied);
    assert_silenced(&speaker, &bystander, &shards).await;
}

/// 像落治理那样，在每个分片上禁言。
async fn mute_everywhere(
    provisioner: &MatrixApplicationServiceProvisioner,
    ledger: &MemoryLedger,
    mute: &ModerationAction,
) {
    for room in &ledger.rooms {
        let target = ModerationEffectTarget {
            matrix_room_id: room.clone(),
            room_kind: RoomCatalogKind::PublicLobby,
            target: mute.target().clone(),
            target_matrix_user_id: Some(ledger.target_user.clone()),
        };
        ModerationEffectGateway::apply(provisioner, mute, &target)
            .await
            .expect("公开大厅禁言应成功");
    }
}

/// 他在每个分片里都说不了，旁人照常说。
async fn assert_silenced(
    speaker: &MatrixConnection,
    bystander: &MatrixConnection,
    rooms: &[MatrixRoomId],
) {
    for room in rooms {
        let denied = speaker
            .gateway()
            .send_event(
                room,
                &message_event(unique_value("expiry-muted"), "禁言期间说不了话"),
            )
            .await
            .expect_err("禁言期间 Matrix 必须在服务端拒绝");
        assert_eq!(denied.kind(), MatrixFailureKind::Forbidden);
        send_with_retry(
            bystander.gateway(),
            room,
            &message_event(unique_value("expiry-bystander"), "旁人照常说话"),
        )
        .await;
    }
}

fn expiry_service(
    ledger: &Arc<MemoryLedger>,
    provisioner: Arc<MatrixApplicationServiceProvisioner>,
    clock: &Arc<TestClock>,
) -> ModerationService {
    ModerationService::new(ModerationDependencies {
        repository: ledger.clone(),
        authority: ledger.clone(),
        expiry: ledger.clone(),
        mutes: ledger.clone(),
        effects: provisioner,
        identifiers: clock.clone(),
        clock: clock.clone(),
        report_policy: ModerationReportPolicy {
            maximum_reports: 5,
            window: DurationMillis::new(60_000).expect("限速窗口有效"),
        },
    })
}

/// 从 `START` 起走的时钟。
struct TestClock(AtomicI64);

impl TestClock {
    fn advance(&self, milliseconds: i64) {
        self.0.fetch_add(milliseconds, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(self.0.load(Ordering::SeqCst)).expect("测试时间有效")
    }
}

impl ModerationIdentifierFactory for TestClock {
    fn moderation_case_id(&self) -> ModerationCaseId {
        ModerationCaseId::from_uuid(Uuid::now_v7())
    }

    fn moderation_action_id(&self) -> ModerationActionId {
        ModerationActionId::from_uuid(Uuid::now_v7())
    }

    fn moderation_audit_event_id(&self) -> AuditEventId {
        AuditEventId::from_uuid(Uuid::now_v7())
    }
}

/// 内存里的治理记录，只管一个公开大厅里的一个人。到期解除只用得到动作、领取和禁言情况，别的接口这个
/// 用例不走。
struct MemoryLedger {
    catalog_id: RoomCatalogId,
    person: ModerationTarget,
    rooms: Vec<MatrixRoomId>,
    target_user: MatrixUserId,
    actions: Mutex<Vec<ModerationAction>>,
    /// 每个动作领过几次、下次能领的时间。
    leases: Mutex<HashMap<ModerationActionId, (u32, UtcMillis)>>,
}

impl MemoryLedger {
    fn new(rooms: Vec<MatrixRoomId>, target_user: MatrixUserId) -> Self {
        Self {
            catalog_id: RoomCatalogId::from_uuid(Uuid::now_v7()),
            person: ModerationTarget::new(
                ModerationTargetKind::Principal,
                PrincipalId::from_uuid(Uuid::now_v7()).to_string(),
            )
            .expect("主体目标有效"),
            rooms,
            target_user,
            actions: Mutex::new(Vec::new()),
            leases: Mutex::new(HashMap::new()),
        }
    }

    /// 记一条已经落下的禁言，`expires_in` 是从此刻起过多久到期。
    fn applied_mute(&self, clock: &TestClock, expires_in: Option<i64>) -> ModerationAction {
        let now = clock.now();
        let mut mute = ModerationAction::reserve(
            ModerationActionId::from_uuid(Uuid::now_v7()),
            None,
            PrincipalId::from_uuid(Uuid::now_v7()),
            self.catalog_id,
            ModerationActionKind::Mute,
            self.person.clone(),
            ModerationReason::Harassment,
            now,
            expires_in.map(|offset| UtcMillis::new(now.value() + offset).expect("到期时间有效")),
        )
        .expect("禁言有效");
        mute.mark_applied().expect("禁言可以落下");
        self.actions.lock().expect("动作锁可用").push(mute.clone());
        mute
    }

    fn status(&self, action: &ModerationAction) -> ModerationActionStatus {
        self.actions
            .lock()
            .expect("动作锁可用")
            .iter()
            .find(|stored| stored.id() == action.id())
            .expect("动作记下了")
            .status()
    }
}

fn unused<T: Send + 'static>() -> PortFuture<'static, RepositoryResult<T>> {
    Box::pin(async {
        Err(RepositoryError::new(
            "moderation.unused_in_expiry",
            RepositoryErrorKind::Unavailable,
        ))
    })
}

impl ModerationRepository for MemoryLedger {
    fn submit_case<'a>(
        &'a self,
        _case: &'a ModerationCase,
        _audit: &'a ModerationAuditEvent,
        _policy: ModerationReportPolicy,
    ) -> PortFuture<'a, RepositoryResult<ModerationReportSubmissionOutcome>> {
        unused()
    }

    fn find_case(
        &self,
        _case_id: ModerationCaseId,
    ) -> PortFuture<'_, RepositoryResult<Option<ModerationCase>>> {
        unused()
    }

    fn list_cases_for_reporter(
        &self,
        _reporter_principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<ModerationCase>>> {
        unused()
    }

    fn list_room_cases(
        &self,
        _room_catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<ModerationCase>>> {
        unused()
    }

    fn reserve_action<'a>(
        &'a self,
        _action: &'a ModerationAction,
        _audit: &'a ModerationAuditEvent,
    ) -> PortFuture<'a, RepositoryResult<ModerationActionReservationOutcome>> {
        unused()
    }

    fn find_action(
        &self,
        _action_id: ModerationActionId,
    ) -> PortFuture<'_, RepositoryResult<Option<ModerationAction>>> {
        unused()
    }

    /// 到期解除只会把已生效的改成已撤销。
    fn finalize_action<'a>(
        &'a self,
        action: &'a ModerationAction,
        _audit: &'a ModerationAuditEvent,
    ) -> PortFuture<'a, RepositoryResult<ModerationAction>> {
        let mut actions = self.actions.lock().expect("动作锁可用");
        let result = match actions.iter_mut().find(|stored| stored.id() == action.id()) {
            Some(stored)
                if stored.status() == ModerationActionStatus::Applied
                    && action.status() == ModerationActionStatus::Reversed =>
            {
                *stored = action.clone();
                Ok(action.clone())
            }
            _ => Err(RepositoryError::new(
                "moderation.finalize_action",
                RepositoryErrorKind::Conflict,
            )),
        };
        Box::pin(async move { result })
    }

    fn list_room_actions(
        &self,
        _room_catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<ModerationAction>>> {
        unused()
    }

    fn append_audit<'a>(
        &'a self,
        _audit: &'a ModerationAuditEvent,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        Box::pin(async { Ok(()) })
    }

    fn list_audit(
        &self,
        _room_catalog_id: Option<RoomCatalogId>,
        _limit: u16,
    ) -> PortFuture<'_, RepositoryResult<Vec<ModerationAuditEvent>>> {
        unused()
    }
}

impl ModerationAuthority for MemoryLedger {
    fn may_report<'a>(
        &'a self,
        _principal_id: PrincipalId,
        _target: &'a ModerationTarget,
        _room_catalog_id: Option<RoomCatalogId>,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        unused()
    }

    fn inspect_room<'a>(
        &'a self,
        _principal_id: PrincipalId,
        _room_catalog_id: RoomCatalogId,
        _target: &'a ModerationTarget,
    ) -> PortFuture<'a, RepositoryResult<Option<ModerationRoomContext>>> {
        unused()
    }

    fn platform_role(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<ModerationRole>> {
        unused()
    }
}

impl ModerationExpiryRepository for MemoryLedger {
    fn claim_due_action(
        &self,
        now: UtcMillis,
        lease_until: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<ModerationExpiryClaim>>> {
        let mut leases = self.leases.lock().expect("租约锁可用");
        let mut due: Vec<ModerationAction> = self
            .actions
            .lock()
            .expect("动作锁可用")
            .iter()
            .filter(|action| action.is_due_at(now))
            .filter(|action| leases.get(&action.id()).is_none_or(|(_, at)| *at <= now))
            .cloned()
            .collect();
        due.sort_by_key(|action| (action.expires_at(), action.id()));
        let claim = due.into_iter().next().map(|action| {
            let lease = leases.entry(action.id()).or_insert((0, lease_until));
            *lease = (lease.0 + 1, lease_until);
            ModerationExpiryClaim {
                attempt: lease.0,
                previous_failure_code: None,
                action,
            }
        });
        Box::pin(async move { Ok(claim) })
    }

    fn reschedule_expiry<'a>(
        &'a self,
        claim: &'a ModerationExpiryClaim,
        reschedule: ModerationExpiryReschedule,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        let mut leases = self.leases.lock().expect("租约锁可用");
        let lease = leases
            .get_mut(&claim.action.id())
            .filter(|(attempts, _)| *attempts == claim.attempt);
        let ours = lease.is_some();
        if let Some(lease) = lease {
            *lease = match reschedule {
                ModerationExpiryReschedule::Retry { at, .. } => (lease.0, at),
                ModerationExpiryReschedule::Defer { at } => (lease.0 - 1, at),
            };
        }
        Box::pin(async move { Ok(ours) })
    }

    /// 只有禁言，走的是禁言那一套，用不到。
    fn has_other_effective_action<'a>(
        &'a self,
        _action: &'a ModerationAction,
        _now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        unused()
    }

    fn expiry_room<'a>(
        &'a self,
        _action: &'a ModerationAction,
    ) -> PortFuture<'a, RepositoryResult<Option<ModerationRoomContext>>> {
        unused()
    }
}

impl ModerationMuteLedger for MemoryLedger {
    fn mute_standing<'a>(
        &'a self,
        _room_catalog_id: RoomCatalogId,
        _target: &'a ModerationTarget,
    ) -> PortFuture<'a, RepositoryResult<Option<ModerationMuteStanding>>> {
        let mutes = self
            .actions
            .lock()
            .expect("动作锁可用")
            .iter()
            .filter(|action| {
                matches!(
                    action.status(),
                    ModerationActionStatus::Applied | ModerationActionStatus::Pending
                )
            })
            .cloned()
            .collect();
        let standing = ModerationMuteStanding {
            room_kind: RoomCatalogKind::PublicLobby,
            matrix_room_ids: self.rooms.clone(),
            target_matrix_user_id: Some(self.target_user.clone()),
            may_speak: true,
            mutes,
        };
        Box::pin(async move { Ok(Some(standing)) })
    }

    fn try_lock_mutes<'a>(
        &'a self,
        _room_catalog_id: RoomCatalogId,
        _target: &'a ModerationTarget,
    ) -> PortFuture<'a, RepositoryResult<Option<Box<dyn ModerationMuteLock>>>> {
        Box::pin(async { Ok(Some(Box::new(NobodyElse) as Box<dyn ModerationMuteLock>)) })
    }
}

/// 只有这一个用例在管这个人，锁总拿得到。
struct NobodyElse;

impl ModerationMuteLock for NobodyElse {
    fn release(self: Box<Self>) -> PortFuture<'static, ()> {
        Box::pin(async {})
    }
}
