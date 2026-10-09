use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
    },
};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    moderation::{
        ApplyModerationAction, InspectModerationCapabilities, ListModerationAudit,
        ListRoomModerationCases, ModerationDependencies, ModerationExpiryOutcome,
        ModerationExpiryRetry, ModerationExpiryUseCases, ModerationFailureKind, ModerationResult,
        ModerationService, ModerationUseCases, ReverseModerationAction, SubmitModerationReport,
    },
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        Clock, MatrixEventId, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult,
        MatrixRoomId, MatrixUserId, ModerationActionReservationOutcome, ModerationAuthority,
        ModerationEffectGateway, ModerationEffectTarget, ModerationExpiryClaim,
        ModerationExpiryRepository, ModerationExpiryReschedule, ModerationIdentifierFactory,
        ModerationMuteLedger, ModerationMuteLock, ModerationMuteStanding, ModerationReportPolicy,
        ModerationReportSubmissionOutcome, ModerationRepository, ModerationRoomContext, PortFuture,
    },
};
use agent_room_domain::{
    ids::{AuditEventId, ModerationActionId, ModerationCaseId, PrincipalId, RoomCatalogId},
    moderation::{
        ModerationAction, ModerationActionKind, ModerationActionStatus, ModerationAuditEvent,
        ModerationCase, ModerationEvidence, ModerationReason, ModerationRole, ModerationTarget,
        ModerationTargetKind,
    },
    rooms::RoomCatalogKind,
    time::{DurationMillis, UtcMillis},
};
use uuid::Uuid;

const NOW: i64 = 1_700_000_000_000;

/// 从 `NOW` 起走的时钟，测到期时往后拨。
#[derive(Default)]
struct TestRuntime {
    elapsed: AtomicI64,
}

impl TestRuntime {
    fn advance(&self, milliseconds: i64) {
        self.elapsed.fetch_add(milliseconds, Ordering::SeqCst);
    }
}

impl Clock for TestRuntime {
    fn now(&self) -> UtcMillis {
        time(NOW + self.elapsed.load(Ordering::SeqCst))
    }
}

impl ModerationIdentifierFactory for TestRuntime {
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

struct FakeRepository {
    cases: Mutex<Vec<ModerationCase>>,
    actions: Mutex<Vec<ModerationAction>>,
    audits: Mutex<Vec<ModerationAuditEvent>>,
    calls: Arc<Mutex<Vec<&'static str>>>,
    rate_limit_at: Mutex<Option<UtcMillis>>,
    /// 下一次落终态之前，别处（另一个管理员、到期解除）已经在这个时间把动作撤掉了。
    reversed_elsewhere_at: Mutex<Option<UtcMillis>>,
}

impl FakeRepository {
    fn new(calls: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            cases: Mutex::new(Vec::new()),
            actions: Mutex::new(Vec::new()),
            audits: Mutex::new(Vec::new()),
            calls,
            rate_limit_at: Mutex::new(None),
            reversed_elsewhere_at: Mutex::new(None),
        }
    }

    fn action(&self, action_id: ModerationActionId) -> ModerationAction {
        self.actions
            .lock()
            .expect("动作锁可用")
            .iter()
            .find(|action| action.id() == action_id)
            .cloned()
            .expect("动作已经预留")
    }

    /// 正在落的这条落下了。
    fn mark_applied(&self, action_id: ModerationActionId) {
        self.actions
            .lock()
            .expect("动作锁可用")
            .iter_mut()
            .find(|action| action.id() == action_id)
            .expect("动作已经预留")
            .mark_applied()
            .expect("正在落的动作可以落下");
    }

    fn audit_actions(&self, action_id: ModerationActionId) -> Vec<String> {
        let target = self.action(action_id).target().clone();
        self.audits
            .lock()
            .expect("审计锁可用")
            .iter()
            .filter(|event| event.target == target)
            .map(|event| event.action.clone())
            .collect()
    }
}

impl ModerationRepository for FakeRepository {
    fn submit_case<'a>(
        &'a self,
        case: &'a ModerationCase,
        audit: &'a ModerationAuditEvent,
        _policy: ModerationReportPolicy,
    ) -> PortFuture<'a, RepositoryResult<ModerationReportSubmissionOutcome>> {
        self.calls.lock().expect("调用锁可用").push("submit_case");
        let limited = *self.rate_limit_at.lock().expect("限速锁可用");
        let outcome = if let Some(retry_at) = limited {
            ModerationReportSubmissionOutcome::RateLimited { retry_at }
        } else {
            self.cases.lock().expect("案件锁可用").push(case.clone());
            self.audits.lock().expect("审计锁可用").push(audit.clone());
            ModerationReportSubmissionOutcome::Created(case.clone())
        };
        Box::pin(async move { Ok(outcome) })
    }

    fn find_case(
        &self,
        case_id: ModerationCaseId,
    ) -> PortFuture<'_, RepositoryResult<Option<ModerationCase>>> {
        let case = self
            .cases
            .lock()
            .expect("案件锁可用")
            .iter()
            .find(|case| case.id() == case_id)
            .cloned();
        Box::pin(async move { Ok(case) })
    }

    fn list_cases_for_reporter(
        &self,
        reporter_principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<ModerationCase>>> {
        let cases = self
            .cases
            .lock()
            .expect("案件锁可用")
            .iter()
            .filter(|case| case.reporter_principal_id() == reporter_principal_id)
            .cloned()
            .collect();
        Box::pin(async move { Ok(cases) })
    }

    fn list_room_cases(
        &self,
        room_catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<ModerationCase>>> {
        let cases = self
            .cases
            .lock()
            .expect("案件锁可用")
            .iter()
            .filter(|case| case.evidence().room_catalog_id() == Some(room_catalog_id))
            .cloned()
            .collect();
        Box::pin(async move { Ok(cases) })
    }

    fn reserve_action<'a>(
        &'a self,
        action: &'a ModerationAction,
        audit: &'a ModerationAuditEvent,
    ) -> PortFuture<'a, RepositoryResult<ModerationActionReservationOutcome>> {
        self.calls.lock().expect("调用锁可用").push("reserve");
        self.actions
            .lock()
            .expect("动作锁可用")
            .push(action.clone());
        self.audits.lock().expect("审计锁可用").push(audit.clone());
        let outcome = ModerationActionReservationOutcome::Reserved(action.clone());
        Box::pin(async move { Ok(outcome) })
    }

    fn find_action(
        &self,
        action_id: ModerationActionId,
    ) -> PortFuture<'_, RepositoryResult<Option<ModerationAction>>> {
        let action = self
            .actions
            .lock()
            .expect("动作锁可用")
            .iter()
            .find(|action| action.id() == action_id)
            .cloned();
        Box::pin(async move { Ok(action) })
    }

    /// 和真实仓库一样只认 待执行→已生效/失败、已生效→已撤销；一模一样的终态原样交回。
    fn finalize_action<'a>(
        &'a self,
        action: &'a ModerationAction,
        audit: &'a ModerationAuditEvent,
    ) -> PortFuture<'a, RepositoryResult<ModerationAction>> {
        self.calls.lock().expect("调用锁可用").push("finalize");
        let mut actions = self.actions.lock().expect("动作锁可用");
        let stored = actions
            .iter_mut()
            .find(|stored| stored.id() == action.id())
            .expect("动作已经预留");
        if let Some(at) = self
            .reversed_elsewhere_at
            .lock()
            .expect("撤销锁可用")
            .take()
        {
            stored.reverse(at).expect("别处撤掉的是已生效的动作");
        }
        let unchanged = stored.status() == action.status()
            && stored.failure_code() == action.failure_code()
            && stored.reversed_at() == action.reversed_at();
        let valid = matches!(
            (stored.status(), action.status()),
            (
                ModerationActionStatus::Pending,
                ModerationActionStatus::Applied | ModerationActionStatus::Failed
            ) | (
                ModerationActionStatus::Applied,
                ModerationActionStatus::Reversed
            )
        );
        let result = if unchanged {
            Ok(stored.clone())
        } else if valid {
            *stored = action.clone();
            self.audits.lock().expect("审计锁可用").push(audit.clone());
            Ok(action.clone())
        } else {
            Err(RepositoryError::new(
                "moderation.finalize_action",
                RepositoryErrorKind::Conflict,
            ))
        };
        Box::pin(async move { result })
    }

    fn list_room_actions(
        &self,
        room_catalog_id: RoomCatalogId,
    ) -> PortFuture<'_, RepositoryResult<Vec<ModerationAction>>> {
        let actions = self
            .actions
            .lock()
            .expect("动作锁可用")
            .iter()
            .filter(|action| action.room_catalog_id() == room_catalog_id)
            .cloned()
            .collect();
        Box::pin(async move { Ok(actions) })
    }

    fn append_audit<'a>(
        &'a self,
        audit: &'a ModerationAuditEvent,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        self.audits.lock().expect("审计锁可用").push(audit.clone());
        Box::pin(async { Ok(()) })
    }

    fn list_audit(
        &self,
        room_catalog_id: Option<RoomCatalogId>,
        limit: u16,
    ) -> PortFuture<'_, RepositoryResult<Vec<ModerationAuditEvent>>> {
        let audits = self
            .audits
            .lock()
            .expect("审计锁可用")
            .iter()
            .filter(|event| room_catalog_id.is_none_or(|room| event.room_catalog_id == Some(room)))
            .take(usize::from(limit))
            .cloned()
            .collect();
        Box::pin(async move { Ok(audits) })
    }
}

struct FakeAuthority {
    may_report: bool,
    room_role: Mutex<ModerationRole>,
    platform_role: Mutex<ModerationRole>,
    room_kind: Mutex<RoomCatalogKind>,
    rooms: Mutex<Vec<MatrixRoomId>>,
}

impl FakeAuthority {
    /// 换成平台管理员看一个分成这些分片的公开大厅。
    fn public_lobby(&self, rooms: &[&str]) {
        *self.room_role.lock().expect("房间角色锁可用") = ModerationRole::PlatformModerator;
        *self.room_kind.lock().expect("房间类别锁可用") = RoomCatalogKind::PublicLobby;
        *self.rooms.lock().expect("分片锁可用") =
            rooms.iter().map(|room| matrix_room(room)).collect();
    }
}

impl ModerationAuthority for FakeAuthority {
    fn may_report<'a>(
        &'a self,
        _principal_id: PrincipalId,
        _target: &'a ModerationTarget,
        _room_catalog_id: Option<RoomCatalogId>,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        let allowed = self.may_report;
        Box::pin(async move { Ok(allowed) })
    }

    fn inspect_room<'a>(
        &'a self,
        _principal_id: PrincipalId,
        _room_catalog_id: RoomCatalogId,
        target: &'a ModerationTarget,
    ) -> PortFuture<'a, RepositoryResult<Option<ModerationRoomContext>>> {
        let role = *self.room_role.lock().expect("房间角色锁可用");
        let room_kind = *self.room_kind.lock().expect("房间类别锁可用");
        let matrix_room_ids = self.rooms.lock().expect("分片锁可用").clone();
        let target_matrix_user_id = (target.kind() == ModerationTargetKind::Principal)
            .then(|| MatrixUserId::new("@target:matrix.test").expect("测试 Matrix 用户有效"));
        Box::pin(async move {
            Ok(Some(ModerationRoomContext {
                role,
                room_kind,
                matrix_room_ids,
                target_matrix_user_id,
            }))
        })
    }

    fn platform_role(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<ModerationRole>> {
        let role = *self.platform_role.lock().expect("平台角色锁可用");
        Box::pin(async move { Ok(role) })
    }
}

/// 到期解除领到的记账：领过几次、下次能领的时间、上次撤不成的失败码。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Lease {
    attempts: u32,
    next_attempt_at: Option<UtcMillis>,
    failure_code: Option<String>,
}

/// 到期解除读的是同一份动作和房间：动作从仓库里挑；分片和房间类别照治理上下文给，不看角色。领取和
/// 排下次照 Postgres 的规矩记在 `leases` 里。
struct FakeExpiry {
    repository: Arc<FakeRepository>,
    authority: Arc<FakeAuthority>,
    leases: Mutex<HashMap<ModerationActionId, Lease>>,
}

impl FakeExpiry {
    fn lease(&self, action: &ModerationAction) -> Lease {
        self.leases
            .lock()
            .expect("租约锁可用")
            .get(&action.id())
            .cloned()
            .unwrap_or_default()
    }
}

impl ModerationExpiryRepository for FakeExpiry {
    fn claim_due_action(
        &self,
        now: UtcMillis,
        lease_until: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<ModerationExpiryClaim>>> {
        let mut leases = self.leases.lock().expect("租约锁可用");
        let mut due: Vec<ModerationAction> = self
            .repository
            .actions
            .lock()
            .expect("动作锁可用")
            .iter()
            .filter(|action| action.is_due_at(now))
            .filter(|action| {
                leases
                    .get(&action.id())
                    .and_then(|lease| lease.next_attempt_at)
                    .is_none_or(|at| at <= now)
            })
            .cloned()
            .collect();
        due.sort_by_key(|action| (action.expires_at(), action.id()));
        let claim = due.into_iter().next().map(|action| {
            let lease = leases.entry(action.id()).or_default();
            lease.attempts += 1;
            lease.next_attempt_at = Some(lease_until);
            ModerationExpiryClaim {
                attempt: lease.attempts,
                previous_failure_code: lease.failure_code.clone(),
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
        let applied =
            self.repository.action(claim.action.id()).status() == ModerationActionStatus::Applied;
        let mut leases = self.leases.lock().expect("租约锁可用");
        let lease = leases.entry(claim.action.id()).or_default();
        let ours = applied && lease.attempts == claim.attempt;
        if ours {
            match reschedule {
                ModerationExpiryReschedule::Retry { at, failure_code } => {
                    lease.next_attempt_at = Some(at);
                    lease.failure_code = Some(failure_code.to_owned());
                }
                ModerationExpiryReschedule::Defer { at } => {
                    lease.next_attempt_at = Some(at);
                    lease.attempts -= 1;
                }
            }
        }
        Box::pin(async move { Ok(ours) })
    }

    fn has_other_effective_action<'a>(
        &'a self,
        action: &'a ModerationAction,
        now: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        let covered = self
            .repository
            .actions
            .lock()
            .expect("动作锁可用")
            .iter()
            .any(|other| {
                other.id() != action.id()
                    && other.room_catalog_id() == action.room_catalog_id()
                    && other.kind() == action.kind()
                    && other.target() == action.target()
                    && other.is_effective_at(now)
            });
        Box::pin(async move { Ok(covered) })
    }

    fn expiry_room<'a>(
        &'a self,
        action: &'a ModerationAction,
    ) -> PortFuture<'a, RepositoryResult<Option<ModerationRoomContext>>> {
        let context = ModerationRoomContext {
            role: ModerationRole::None,
            room_kind: *self.authority.room_kind.lock().expect("房间类别锁可用"),
            matrix_room_ids: self.authority.rooms.lock().expect("分片锁可用").clone(),
            target_matrix_user_id: (action.target().kind() == ModerationTargetKind::Principal)
                .then(|| MatrixUserId::new("@target:matrix.test").expect("测试 Matrix 用户有效")),
        };
        Box::pin(async move { Ok(Some(context)) })
    }
}

/// 第一次读完禁言情况以后、动完再看一眼之前发生的事。
type BetweenReads = Box<dyn FnOnce() + Send>;

/// 结束禁言前读的情况：禁言从仓库里按 UUID 挑，分片和房间类别照治理上下文给；发言权、锁可以调。
struct FakeMutes {
    repository: Arc<FakeRepository>,
    authority: Arc<FakeAuthority>,
    may_speak: Mutex<bool>,
    /// 他的 Matrix 账号没了。
    account_gone: Mutex<bool>,
    /// 正拿着的锁。
    held: Arc<Mutex<HashSet<String>>>,
    between_reads: Mutex<Option<BetweenReads>>,
}

impl FakeMutes {
    /// 别处拿着这个人的禁言锁。
    fn hold(&self, target: &ModerationTarget) {
        self.held.lock().expect("锁表可用").insert(lock_key(target));
    }

    fn release(&self, target: &ModerationTarget) {
        self.held
            .lock()
            .expect("锁表可用")
            .remove(&lock_key(target));
    }

    fn between_reads(&self, happen: impl FnOnce() + Send + 'static) {
        *self.between_reads.lock().expect("钩子锁可用") = Some(Box::new(happen));
    }
}

fn lock_key(target: &ModerationTarget) -> String {
    Uuid::parse_str(target.reference()).map_or_else(
        |_| target.reference().to_owned(),
        |person| person.to_string(),
    )
}

impl ModerationMuteLedger for FakeMutes {
    fn mute_standing<'a>(
        &'a self,
        room_catalog_id: RoomCatalogId,
        target: &'a ModerationTarget,
    ) -> PortFuture<'a, RepositoryResult<Option<ModerationMuteStanding>>> {
        let person = Uuid::parse_str(target.reference()).ok();
        let mutes = self
            .repository
            .actions
            .lock()
            .expect("动作锁可用")
            .iter()
            .filter(|action| {
                action.room_catalog_id() == room_catalog_id
                    && action.kind() == ModerationActionKind::Mute
                    && matches!(
                        action.status(),
                        ModerationActionStatus::Applied | ModerationActionStatus::Pending
                    )
                    && Uuid::parse_str(action.target().reference()).ok() == person
            })
            .cloned()
            .collect();
        let standing = ModerationMuteStanding {
            room_kind: *self.authority.room_kind.lock().expect("房间类别锁可用"),
            matrix_room_ids: self.authority.rooms.lock().expect("分片锁可用").clone(),
            target_matrix_user_id: (!*self.account_gone.lock().expect("账号锁可用"))
                .then(|| MatrixUserId::new("@target:matrix.test").expect("测试 Matrix 用户有效")),
            may_speak: *self.may_speak.lock().expect("发言权锁可用"),
            mutes,
        };
        if let Some(happen) = self.between_reads.lock().expect("钩子锁可用").take() {
            happen();
        }
        Box::pin(async move { Ok(Some(standing)) })
    }

    fn try_lock_mutes<'a>(
        &'a self,
        _room_catalog_id: RoomCatalogId,
        target: &'a ModerationTarget,
    ) -> PortFuture<'a, RepositoryResult<Option<Box<dyn ModerationMuteLock>>>> {
        let key = lock_key(target);
        let taken = self.held.lock().expect("锁表可用").insert(key.clone());
        let lock = taken.then(|| {
            Box::new(FakeMuteLock {
                held: self.held.clone(),
                key,
            }) as Box<dyn ModerationMuteLock>
        });
        Box::pin(async move { Ok(lock) })
    }
}

struct FakeMuteLock {
    held: Arc<Mutex<HashSet<String>>>,
    key: String,
}

impl ModerationMuteLock for FakeMuteLock {
    fn release(self: Box<Self>) -> PortFuture<'static, ()> {
        self.held.lock().expect("锁表可用").remove(&self.key);
        Box::pin(async {})
    }
}

#[derive(Default)]
struct FakeEffects {
    calls: Arc<Mutex<Vec<&'static str>>>,
    failure: Mutex<Option<MatrixFailure>>,
    /// 只在这个分片上失败。
    failing_room: Mutex<Option<MatrixRoomId>>,
    applied: Mutex<Vec<ModerationEffectTarget>>,
    reversed: Mutex<Vec<ModerationEffectTarget>>,
    /// 每个分片里有哪些事件。
    events: Mutex<HashMap<String, Vec<String>>>,
    /// 读不了的分片。
    unreadable_rooms: Mutex<Vec<MatrixRoomId>>,
    /// 按先后记下问过哪些分片。
    lookups: Mutex<Vec<String>>,
}

impl FakeEffects {
    fn result_in(&self, room: &MatrixRoomId) -> MatrixResult<()> {
        if self.failing_room.lock().expect("副作用锁可用").as_ref() == Some(room) {
            return Err(MatrixFailure::new(
                MatrixOperation::Ban,
                MatrixFailureKind::DependencyUnavailable,
            ));
        }
        self.failure
            .lock()
            .expect("副作用锁可用")
            .map_or(Ok(()), Err)
    }

    fn put_event(&self, room: &str, event: &str) {
        self.events
            .lock()
            .expect("事件锁可用")
            .entry(room.to_owned())
            .or_default()
            .push(event.to_owned());
    }

    fn applied_rooms(&self) -> Vec<String> {
        rooms_of(&self.applied.lock().expect("副作用锁可用"))
    }

    fn reversed_rooms(&self) -> Vec<String> {
        rooms_of(&self.reversed.lock().expect("副作用锁可用"))
    }

    fn looked_up(&self) -> Vec<String> {
        self.lookups.lock().expect("查找锁可用").clone()
    }
}

fn rooms_of(targets: &[ModerationEffectTarget]) -> Vec<String> {
    targets
        .iter()
        .map(|target| target.matrix_room_id.as_str().to_owned())
        .collect()
}

impl ModerationEffectGateway for FakeEffects {
    fn apply<'a>(
        &'a self,
        _action: &'a ModerationAction,
        target: &'a ModerationEffectTarget,
    ) -> PortFuture<'a, MatrixResult<()>> {
        self.calls.lock().expect("调用锁可用").push("effect_apply");
        self.applied
            .lock()
            .expect("副作用锁可用")
            .push(target.clone());
        let result = self.result_in(&target.matrix_room_id);
        Box::pin(async move { result })
    }

    fn reverse<'a>(
        &'a self,
        _action: &'a ModerationAction,
        target: &'a ModerationEffectTarget,
    ) -> PortFuture<'a, MatrixResult<()>> {
        self.calls
            .lock()
            .expect("调用锁可用")
            .push("effect_reverse");
        self.reversed
            .lock()
            .expect("副作用锁可用")
            .push(target.clone());
        let result = self.result_in(&target.matrix_room_id);
        Box::pin(async move { result })
    }

    fn contains_event<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event_id: &'a MatrixEventId,
    ) -> PortFuture<'a, MatrixResult<bool>> {
        self.lookups
            .lock()
            .expect("查找锁可用")
            .push(room_id.as_str().to_owned());
        let result = if self
            .unreadable_rooms
            .lock()
            .expect("分片锁可用")
            .contains(room_id)
        {
            Err(MatrixFailure::new(
                MatrixOperation::ReadRoomEvent,
                MatrixFailureKind::Timeout,
            ))
        } else {
            Ok(self
                .events
                .lock()
                .expect("事件锁可用")
                .get(room_id.as_str())
                .is_some_and(|events| events.iter().any(|event| event == event_id.as_str())))
        };
        Box::pin(async move { result })
    }
}

struct Fixture {
    service: ModerationService,
    repository: Arc<FakeRepository>,
    authority: Arc<FakeAuthority>,
    expiry: Arc<FakeExpiry>,
    mutes: Arc<FakeMutes>,
    effects: Arc<FakeEffects>,
    runtime: Arc<TestRuntime>,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl Fixture {
    fn new() -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let repository = Arc::new(FakeRepository::new(calls.clone()));
        let authority = Arc::new(FakeAuthority {
            may_report: true,
            room_role: Mutex::new(ModerationRole::RoomManager),
            platform_role: Mutex::new(ModerationRole::None),
            room_kind: Mutex::new(RoomCatalogKind::PrivateRoom),
            rooms: Mutex::new(vec![matrix_room("!room:matrix.test")]),
        });
        let effects = Arc::new(FakeEffects {
            calls: calls.clone(),
            ..FakeEffects::default()
        });
        let runtime = Arc::new(TestRuntime::default());
        let expiry = Arc::new(FakeExpiry {
            repository: repository.clone(),
            authority: authority.clone(),
            leases: Mutex::new(HashMap::new()),
        });
        let mutes = Arc::new(FakeMutes {
            repository: repository.clone(),
            authority: authority.clone(),
            may_speak: Mutex::new(true),
            account_gone: Mutex::new(false),
            held: Arc::new(Mutex::new(HashSet::new())),
            between_reads: Mutex::new(None),
        });
        let service = ModerationService::new(ModerationDependencies {
            repository: repository.clone(),
            authority: authority.clone(),
            expiry: expiry.clone(),
            mutes: mutes.clone(),
            effects: effects.clone(),
            identifiers: runtime.clone(),
            clock: runtime.clone(),
            report_policy: ModerationReportPolicy {
                maximum_reports: 5,
                window: DurationMillis::new(600_000).expect("窗口有效"),
            },
        });
        Self {
            service,
            repository,
            authority,
            expiry,
            mutes,
            effects,
            runtime,
            calls,
        }
    }
}

#[tokio::test]
async fn 治理能力投影分别表达房间管理与平台审计权限() {
    let fixture = Fixture::new();
    let room_manager = fixture
        .service
        .inspect_capabilities(InspectModerationCapabilities {
            actor: actor(false),
            room_catalog_id: room_id(),
        })
        .await
        .expect("房间治理能力可读取");
    assert!(room_manager.can_moderate_room);
    assert!(!room_manager.can_read_audit);

    *fixture.authority.room_role.lock().expect("房间角色锁可用") = ModerationRole::AuditReader;
    *fixture
        .authority
        .platform_role
        .lock()
        .expect("平台角色锁可用") = ModerationRole::AuditReader;
    let audit_reader = fixture
        .service
        .inspect_capabilities(InspectModerationCapabilities {
            actor: actor(false),
            room_catalog_id: room_id(),
        })
        .await
        .expect("审计能力可读取");
    assert!(!audit_reader.can_moderate_room);
    assert!(audit_reader.can_read_audit);
}

#[tokio::test]
async fn 举报只提交显式最小证据且限速失败可重试() {
    let fixture = Fixture::new();
    let created = fixture
        .service
        .submit_report(report_request())
        .await
        .expect("举报可创建");
    assert!(created.evidence().end_to_end_encrypted());
    assert_eq!(created.evidence().reporter_submitted_excerpt(), None);
    assert_eq!(
        fixture.repository.audits.lock().expect("审计锁可用").len(),
        1
    );

    *fixture.repository.rate_limit_at.lock().expect("限速锁可用") = Some(time(NOW + 30_000));
    let failure = fixture
        .service
        .submit_report(report_request())
        .await
        .expect_err("超额举报必须拒绝");
    assert_eq!(failure.kind(), ModerationFailureKind::RateLimited);
    assert_eq!(failure.retry_at(), Some(time(NOW + 30_000)));
}

#[tokio::test]
async fn 治理动作先持久化待执行记录再触发_matrix_并写入终态() {
    let fixture = Fixture::new();
    let action = fixture
        .service
        .apply_action(action_request())
        .await
        .expect("治理动作可执行");

    assert_eq!(action.status(), ModerationActionStatus::Applied);
    assert_eq!(
        *fixture.calls.lock().expect("调用锁可用"),
        vec!["reserve", "effect_apply", "finalize"]
    );
}

#[tokio::test]
async fn 房间案件队列只对当前管理者开放且不跨房泄漏() {
    let fixture = Fixture::new();
    let expected = fixture
        .service
        .submit_report(report_request())
        .await
        .expect("房间举报应成功");
    let visible = fixture
        .service
        .list_room_cases(ListRoomModerationCases {
            actor: actor(false),
            room_catalog_id: room_id(),
        })
        .await
        .expect("房间管理者可读取案件队列");
    assert_eq!(visible, vec![expected]);

    let other_room = fixture
        .repository
        .list_room_cases(RoomCatalogId::from_uuid(Uuid::from_u128(4)))
        .await
        .expect("其他房间查询应成功");
    assert!(other_room.is_empty());

    *fixture.authority.room_role.lock().expect("房间角色锁可用") = ModerationRole::None;
    let failure = fixture
        .service
        .list_room_cases(ListRoomModerationCases {
            actor: actor(false),
            room_catalog_id: room_id(),
        })
        .await
        .expect_err("失去当前权限后必须拒绝读取");
    assert_eq!(failure.kind(), ModerationFailureKind::Forbidden);
}

#[tokio::test]
async fn matrix_失败会落库失败终态且绝不伪装成功() {
    let fixture = Fixture::new();
    *fixture.effects.failure.lock().expect("副作用锁可用") = Some(MatrixFailure::new(
        MatrixOperation::UpdatePowerLevels,
        MatrixFailureKind::DependencyUnavailable,
    ));

    let failure = fixture
        .service
        .apply_action(action_request())
        .await
        .expect_err("Matrix 失败必须向上返回");
    assert_eq!(failure.kind(), ModerationFailureKind::DependencyUnavailable);
    let stored = fixture.repository.actions.lock().expect("动作锁可用");
    assert_eq!(stored[0].status(), ModerationActionStatus::Failed);
    assert_eq!(stored[0].failure_code(), Some("matrix.unavailable"));
}

#[tokio::test]
async fn 撤销治理每次重读当前权限且审计读取使用独立角色() {
    let fixture = Fixture::new();
    let action = fixture
        .service
        .apply_action(action_request())
        .await
        .expect("先应用治理");
    *fixture.authority.room_role.lock().expect("房间角色锁可用") = ModerationRole::None;

    let failure = fixture
        .service
        .reverse_action(ReverseModerationAction {
            actor: actor(true),
            action_id: action.id(),
            impact_acknowledged: true,
        })
        .await
        .expect_err("失去权限后不能撤销");
    assert_eq!(failure.kind(), ModerationFailureKind::Forbidden);

    *fixture
        .authority
        .platform_role
        .lock()
        .expect("平台角色锁可用") = ModerationRole::AuditReader;
    let audit = fixture
        .service
        .list_audit(ListModerationAudit {
            actor: actor(false),
            room_catalog_id: Some(room_id()),
            limit: 20,
        })
        .await
        .expect("审计角色可读取元数据");
    assert!(!audit.is_empty());
    assert!(audit.iter().all(|event| !event.action.contains("正文")));
}

#[tokio::test]
async fn 治理和撤销要求近期认证及明确影响确认() {
    let fixture = Fixture::new();
    let mut request = action_request();
    request.actor = actor(false);
    let failure = fixture
        .service
        .apply_action(request)
        .await
        .expect_err("旧认证不能治理");
    assert_eq!(failure.kind(), ModerationFailureKind::Forbidden);
    assert!(
        fixture
            .repository
            .actions
            .lock()
            .expect("动作锁可用")
            .is_empty()
    );
}

const SPAM_EVENT: &str = "$spam:matrix.test";

#[tokio::test]
async fn 公开大厅有好几个分片时隐藏写进消息所在的分片_撤销也回到那里() {
    let fixture = Fixture::new();
    fixture.authority.public_lobby(&[
        "!busy:matrix.test",
        "!quiet:matrix.test",
        "!new:matrix.test",
    ]);
    fixture.effects.put_event("!quiet:matrix.test", SPAM_EVENT);

    let action = fixture
        .service
        .apply_action(hide_request(SPAM_EVENT))
        .await
        .expect("隐藏应成功");
    assert_eq!(action.status(), ModerationActionStatus::Applied);
    assert_eq!(fixture.effects.applied_rooms(), ["!quiet:matrix.test"]);
    // 最活跃的分片先问，找到就不再往下问。
    assert_eq!(
        fixture.effects.looked_up(),
        ["!busy:matrix.test", "!quiet:matrix.test"]
    );

    let reversed = fixture
        .service
        .reverse_action(ReverseModerationAction {
            actor: actor(true),
            action_id: action.id(),
            impact_acknowledged: true,
        })
        .await
        .expect("撤销隐藏应成功");
    assert_eq!(reversed.status(), ModerationActionStatus::Reversed);
    assert_eq!(fixture.effects.reversed_rooms(), ["!quiet:matrix.test"]);
}

#[tokio::test]
async fn 消息不在任何分片里时说找不到_不预留动作也不落副作用() {
    let fixture = Fixture::new();
    fixture
        .authority
        .public_lobby(&["!busy:matrix.test", "!quiet:matrix.test"]);

    let failure = fixture
        .service
        .apply_action(hide_request(SPAM_EVENT))
        .await
        .expect_err("哪个分片里都没有的消息不能隐藏");
    assert_eq!(failure.kind(), ModerationFailureKind::NotFound);
    assert_eq!(
        fixture.effects.looked_up(),
        ["!busy:matrix.test", "!quiet:matrix.test"]
    );
    assert!(fixture.calls.lock().expect("调用锁可用").is_empty());
    assert!(
        fixture
            .repository
            .actions
            .lock()
            .expect("动作锁可用")
            .is_empty()
    );
}

#[tokio::test]
async fn 有分片读不了时别处找到照样隐藏_都没找到就报依赖不可用() {
    let fixture = Fixture::new();
    fixture
        .authority
        .public_lobby(&["!busy:matrix.test", "!quiet:matrix.test"]);
    fixture
        .effects
        .unreadable_rooms
        .lock()
        .expect("分片锁可用")
        .push(matrix_room("!busy:matrix.test"));

    let failure = fixture
        .service
        .apply_action(hide_request(SPAM_EVENT))
        .await
        .expect_err("读不了又没找到时不能当成没有");
    assert_eq!(failure.kind(), ModerationFailureKind::DependencyUnavailable);
    assert!(
        fixture
            .repository
            .actions
            .lock()
            .expect("动作锁可用")
            .is_empty()
    );

    fixture.effects.put_event("!quiet:matrix.test", SPAM_EVENT);
    fixture
        .service
        .apply_action(hide_request(SPAM_EVENT))
        .await
        .expect("别的分片里找到了就照样隐藏");
    assert_eq!(fixture.effects.applied_rooms(), ["!quiet:matrix.test"]);
}

#[tokio::test]
async fn 只有一个分片时隐藏不用找_直接写进去() {
    let fixture = Fixture::new();
    fixture.authority.public_lobby(&["!only:matrix.test"]);

    fixture
        .service
        .apply_action(hide_request(SPAM_EVENT))
        .await
        .expect("只有一个分片时照旧直接隐藏");
    assert!(fixture.effects.looked_up().is_empty());
    assert_eq!(fixture.effects.applied_rooms(), ["!only:matrix.test"]);
}

#[tokio::test]
async fn 禁言踢出封禁落到每个活跃分片_撤销也在每个分片上撤() {
    for kind in [
        ModerationActionKind::Mute,
        ModerationActionKind::Kick,
        ModerationActionKind::Ban,
    ] {
        let fixture = Fixture::new();
        fixture
            .authority
            .public_lobby(&["!busy:matrix.test", "!quiet:matrix.test"]);

        let action = fixture
            .service
            .apply_action(person_request(kind))
            .await
            .expect("管人的治理应成功");
        assert_eq!(
            fixture.effects.applied_rooms(),
            ["!busy:matrix.test", "!quiet:matrix.test"],
            "{kind:?} 要落到每个分片"
        );
        assert!(
            fixture
                .effects
                .applied
                .lock()
                .expect("副作用锁可用")
                .iter()
                .all(|target| target.room_kind == RoomCatalogKind::PublicLobby
                    && target
                        .target_matrix_user_id
                        .as_ref()
                        .map(MatrixUserId::as_str)
                        == Some("@target:matrix.test"))
        );
        assert!(fixture.effects.looked_up().is_empty());

        fixture
            .service
            .reverse_action(ReverseModerationAction {
                actor: actor(true),
                action_id: action.id(),
                impact_acknowledged: true,
            })
            .await
            .expect("撤销应成功");
        assert_eq!(
            fixture.effects.reversed_rooms(),
            ["!busy:matrix.test", "!quiet:matrix.test"],
            "{kind:?} 要在每个分片上撤"
        );
    }
}

#[tokio::test]
async fn 有分片落不成时动作记成失败_照实报依赖不可用() {
    let fixture = Fixture::new();
    fixture.authority.public_lobby(&[
        "!busy:matrix.test",
        "!quiet:matrix.test",
        "!new:matrix.test",
    ]);
    *fixture.effects.failing_room.lock().expect("副作用锁可用") =
        Some(matrix_room("!quiet:matrix.test"));

    let failure = fixture
        .service
        .apply_action(person_request(ModerationActionKind::Ban))
        .await
        .expect_err("没落全不能说成功");
    assert_eq!(failure.kind(), ModerationFailureKind::DependencyUnavailable);
    // 落不成就停下，剩下的分片等管理员再做一次：已经落了的分片那时什么也不变。
    assert_eq!(
        fixture.effects.applied_rooms(),
        ["!busy:matrix.test", "!quiet:matrix.test"]
    );
    let stored = fixture.repository.actions.lock().expect("动作锁可用");
    assert_eq!(stored[0].status(), ModerationActionStatus::Failed);
    assert_eq!(stored[0].failure_code(), Some("matrix.unavailable"));
}

const HOUR: i64 = 3_600_000;
const SECOND: i64 = 1_000;
const EXPIRED: &str = "moderation.action.expired";
const EXPIRE_FAILED: &str = "moderation.action.expire_failed";

#[tokio::test]
async fn 到期的禁言自动解除_每个活跃分片都撤到并留下审计() {
    let fixture = Fixture::new();
    fixture
        .authority
        .public_lobby(&["!busy:matrix.test", "!quiet:matrix.test"]);
    let action = apply(&fixture, mute(2, Some(HOUR))).await;

    fixture.runtime.advance(HOUR);
    let outcome = expire(&fixture).await;

    assert_eq!(outcome.expired, 1);
    assert!(outcome.retrying.is_empty());
    assert_eq!(
        fixture.effects.reversed_rooms(),
        ["!busy:matrix.test", "!quiet:matrix.test"]
    );
    let expired = fixture.repository.action(action.id());
    assert_eq!(expired.status(), ModerationActionStatus::Reversed);
    // 撤销时间不早于到期时间：网页台账据此说“到期解除”，不说“已撤回”。
    assert_eq!(expired.reversed_at(), Some(time(NOW + HOUR)));
    assert_eq!(
        fixture.repository.audit_actions(action.id()),
        [
            "moderation.action.requested",
            "moderation.action.applied",
            EXPIRED
        ]
    );

    assert_eq!(expire(&fixture).await.expired, 0, "解除过的不再解除");
    assert_eq!(fixture.effects.reversed_rooms().len(), 2);
}

#[tokio::test]
async fn 没到期的和没有期限的都不动() {
    let fixture = Fixture::new();
    let later = apply(&fixture, mute(2, Some(2 * HOUR))).await;
    let indefinite = apply(&fixture, mute(4, None)).await;

    fixture.runtime.advance(HOUR);
    assert_eq!(
        expire(&fixture).await,
        ModerationExpiryOutcome::default(),
        "一小时后两个都还在生效"
    );
    assert!(fixture.effects.reversed_rooms().is_empty());

    fixture.runtime.advance(365 * 24 * HOUR);
    assert_eq!(expire(&fixture).await.expired, 1);
    assert_eq!(
        status_of(&fixture, &later),
        ModerationActionStatus::Reversed
    );
    assert_eq!(
        status_of(&fixture, &indefinite),
        ModerationActionStatus::Applied,
        "没有期限的一直不动"
    );
    assert_eq!(fixture.effects.reversed_rooms().len(), 1);
}

#[tokio::test]
async fn 整台聊天服务器不通时这一轮先停_撤不成的退避以后再试() {
    let fixture = Fixture::new();
    fixture
        .authority
        .public_lobby(&["!busy:matrix.test", "!quiet:matrix.test"]);
    let first = apply(&fixture, mute(2, Some(HOUR))).await;
    let second = apply(&fixture, mute(4, Some(2 * HOUR))).await;
    *fixture.effects.failing_room.lock().expect("副作用锁可用") =
        Some(matrix_room("!quiet:matrix.test"));

    fixture.runtime.advance(2 * HOUR);
    let outcome = expire(&fixture).await;
    assert_eq!(outcome.expired, 0);
    // 聊天服务器连不上，这一轮到此为止：第二个不用再试。
    assert_eq!(
        outcome.retrying,
        [ModerationExpiryRetry {
            action_id: first.id(),
            failure_code: "matrix.unavailable",
        }]
    );
    assert_eq!(
        fixture.effects.reversed_rooms(),
        ["!busy:matrix.test", "!quiet:matrix.test"]
    );
    for action in [&first, &second] {
        assert_eq!(status_of(&fixture, action), ModerationActionStatus::Applied);
    }
    assert_eq!(
        fixture.repository.audit_actions(first.id()).last(),
        Some(&EXPIRE_FAILED.to_owned())
    );

    *fixture.effects.failing_room.lock().expect("副作用锁可用") = None;
    assert_eq!(expire(&fixture).await.expired, 1, "第二个这一轮就解除");
    assert_eq!(status_of(&fixture, &first), ModerationActionStatus::Applied);
    assert_eq!(
        status_of(&fixture, &second),
        ModerationActionStatus::Reversed
    );
    fixture.runtime.advance(30 * SECOND);
    assert_eq!(expire(&fixture).await.expired, 1, "第一个 30 秒以后再试");
    assert_eq!(
        status_of(&fixture, &first),
        ModerationActionStatus::Reversed
    );
    // 头一回撤了两个分片没撤成，之后两个动作各在两个分片上撤了一遍。
    assert_eq!(fixture.effects.reversed_rooms().len(), 6);
}

#[tokio::test]
async fn 撤不成时退避_三十秒起每次翻倍_最长十五分钟_审计只写第一次() {
    let fixture = Fixture::new();
    let action = apply(&fixture, mute(2, Some(HOUR))).await;
    *fixture.effects.failure.lock().expect("副作用锁可用") = Some(MatrixFailure::new(
        MatrixOperation::UpdatePowerLevels,
        MatrixFailureKind::Forbidden,
    ));
    fixture.runtime.advance(HOUR);

    for (index, delay) in [30, 60, 120, 240, 480, 900, 900].into_iter().enumerate() {
        let now = fixture.runtime.now();
        assert_eq!(expire(&fixture).await.retrying.len(), 1);
        let lease = fixture.expiry.lease(&action);
        assert_eq!(lease.attempts, u32::try_from(index + 1).expect("次数不大"));
        assert_eq!(
            lease.next_attempt_at,
            Some(time(now.value() + delay * SECOND)),
            "第 {} 次撤不成以后隔 {delay} 秒",
            index + 1
        );
        assert_eq!(lease.failure_code.as_deref(), Some("matrix.forbidden"));
        fixture.runtime.advance(delay * SECOND - 1);
        assert_eq!(
            expire(&fixture).await,
            ModerationExpiryOutcome::default(),
            "退避期间不领"
        );
        fixture.runtime.advance(1);
    }

    let failures = fixture
        .repository
        .audit_actions(action.id())
        .into_iter()
        .filter(|audit| audit == EXPIRE_FAILED)
        .count();
    assert_eq!(failures, 1, "同一条连着撤不成，审计只写第一次");
    assert_eq!(
        status_of(&fixture, &action),
        ModerationActionStatus::Applied
    );
}

#[tokio::test]
async fn 只是撤不了这一个时接着撤别的() {
    let fixture = Fixture::new();
    let first = apply(&fixture, mute(2, Some(HOUR))).await;
    let second = apply(&fixture, mute(4, Some(HOUR))).await;
    *fixture.effects.failure.lock().expect("副作用锁可用") = Some(MatrixFailure::new(
        MatrixOperation::UpdatePowerLevels,
        MatrixFailureKind::Forbidden,
    ));

    fixture.runtime.advance(HOUR);
    let outcome = expire(&fixture).await;

    assert_eq!(outcome.expired, 0);
    assert_eq!(
        outcome.retrying,
        [first.id(), second.id()].map(|action_id| ModerationExpiryRetry {
            action_id,
            failure_code: "matrix.forbidden",
        })
    );
}

#[tokio::test]
async fn 同一个人还有别的禁言在生效时不放人_在每个分片上确保禁着() {
    let fixture = Fixture::new();
    fixture
        .authority
        .public_lobby(&["!busy:matrix.test", "!quiet:matrix.test"]);
    let short = apply(&fixture, mute(2, Some(HOUR))).await;
    let indefinite = apply(&fixture, mute(2, None)).await;
    let applied_before = fixture.effects.applied_rooms().len();

    fixture.runtime.advance(HOUR);
    assert_eq!(expire(&fixture).await.expired, 1);

    assert!(
        fixture.effects.reversed_rooms().is_empty(),
        "另一条禁言还在生效，不能撤掉发言权"
    );
    assert_eq!(
        fixture.effects.applied_rooms()[applied_before..],
        ["!busy:matrix.test", "!quiet:matrix.test"],
        "把还生效的那条在每个分片上再落一次"
    );
    assert_eq!(
        status_of(&fixture, &short),
        ModerationActionStatus::Reversed
    );
    assert_eq!(
        status_of(&fixture, &indefinite),
        ModerationActionStatus::Applied
    );
}

#[tokio::test]
async fn 同一个人写法不一样的禁言也认得出来() {
    let fixture = Fixture::new();
    let short = apply(&fixture, mute(2, Some(HOUR))).await;
    let shouted = Uuid::from_u128(2).simple().to_string().to_uppercase();
    apply(
        &fixture,
        ApplyModerationAction {
            target: ModerationTarget::new(ModerationTargetKind::Principal, shouted)
                .expect("大写、不带横线的 UUID 也是有效目标"),
            ..mute(2, None)
        },
    )
    .await;

    fixture.runtime.advance(HOUR);
    assert_eq!(expire(&fixture).await.expired, 1);

    assert!(
        fixture.effects.reversed_rooms().is_empty(),
        "写法不一样也是同一个人，他还禁着"
    );
    assert_eq!(
        status_of(&fixture, &short),
        ModerationActionStatus::Reversed
    );
}

#[tokio::test]
async fn 有一条禁言正在落时先不动_落下以后确保禁着() {
    let fixture = Fixture::new();
    let short = apply(&fixture, mute(2, Some(HOUR))).await;
    fixture.runtime.advance(HOUR);
    let landing = landing_mute(&fixture, 2);

    assert_eq!(expire(&fixture).await, ModerationExpiryOutcome::default());
    assert!(
        fixture.effects.reversed_rooms().is_empty(),
        "正在落的那条落下以前什么也不动"
    );
    assert_eq!(status_of(&fixture, &short), ModerationActionStatus::Applied);
    assert_eq!(
        fixture.expiry.lease(&short),
        Lease {
            attempts: 0,
            next_attempt_at: Some(time(NOW + HOUR + 20 * SECOND)),
            failure_code: None,
        },
        "下一轮再看：不算失败，领过的次数退回去"
    );

    fixture.repository.mark_applied(landing.id());
    fixture.runtime.advance(20 * SECOND);
    assert_eq!(expire(&fixture).await.expired, 1);
    assert!(fixture.effects.reversed_rooms().is_empty());
    assert_eq!(
        fixture.effects.applied_rooms().len(),
        2,
        "落下以后确保他禁着"
    );
}

#[tokio::test]
async fn 正在落了五分钟还没落完的当它没有() {
    let fixture = Fixture::new();
    let short = apply(&fixture, mute(2, Some(HOUR))).await;
    // 落到一半控制面断了，一直停在正在落。
    landing_mute(&fixture, 2);

    fixture.runtime.advance(HOUR);
    assert_eq!(expire(&fixture).await.expired, 1);

    assert_eq!(fixture.effects.reversed_rooms(), ["!room:matrix.test"]);
    assert_eq!(
        status_of(&fixture, &short),
        ModerationActionStatus::Reversed
    );
}

#[tokio::test]
async fn 动完再看一眼又多了一条禁言_这次不记_下一轮确保禁着() {
    let fixture = Fixture::new();
    let short = apply(&fixture, mute(2, Some(HOUR))).await;
    fixture.runtime.advance(HOUR);
    // 后台刚解禁，管理员又禁了他：他那条先记成正在落，再去动 Matrix。
    let landing = pending_mute(2, time(NOW + HOUR));
    let landing_id = landing.id();
    let repository = fixture.repository.clone();
    fixture.mutes.between_reads(move || {
        repository.actions.lock().expect("动作锁可用").push(landing);
    });

    assert_eq!(expire(&fixture).await, ModerationExpiryOutcome::default());
    assert_eq!(fixture.effects.reversed_rooms(), ["!room:matrix.test"]);
    assert_eq!(
        status_of(&fixture, &short),
        ModerationActionStatus::Applied,
        "情况变了，这次不记"
    );

    fixture.repository.mark_applied(landing_id);
    fixture.runtime.advance(20 * SECOND);
    assert_eq!(expire(&fixture).await.expired, 1);
    let calls = fixture.calls.lock().expect("调用锁可用").clone();
    assert_eq!(
        calls.iter().rev().find(|call| call.starts_with("effect_")),
        Some(&"effect_apply"),
        "最后落在 Matrix 上的是禁言"
    );
}

#[tokio::test]
async fn 动完再看一眼新开了分片_这次不记_下一轮在每个分片上撤() {
    let fixture = Fixture::new();
    fixture
        .authority
        .public_lobby(&["!busy:matrix.test", "!quiet:matrix.test"]);
    let short = apply(&fixture, mute(2, Some(HOUR))).await;
    fixture.runtime.advance(HOUR);
    let authority = fixture.authority.clone();
    fixture.mutes.between_reads(move || {
        authority
            .rooms
            .lock()
            .expect("分片锁可用")
            .push(matrix_room("!new:matrix.test"));
    });

    assert_eq!(expire(&fixture).await.expired, 0);
    assert_eq!(status_of(&fixture, &short), ModerationActionStatus::Applied);

    fixture.runtime.advance(20 * SECOND);
    assert_eq!(expire(&fixture).await.expired, 1);
    assert_eq!(
        fixture.effects.reversed_rooms(),
        [
            "!busy:matrix.test",
            "!quiet:matrix.test",
            "!busy:matrix.test",
            "!quiet:matrix.test",
            "!new:matrix.test"
        ]
    );
}

#[tokio::test]
async fn 别处正拿着这个人的禁言锁时这一轮先不动() {
    let fixture = Fixture::new();
    let short = apply(&fixture, mute(2, Some(HOUR))).await;
    fixture.runtime.advance(HOUR);
    fixture.mutes.hold(short.target());

    assert_eq!(expire(&fixture).await, ModerationExpiryOutcome::default());
    assert!(fixture.effects.reversed_rooms().is_empty());
    assert_eq!(fixture.expiry.lease(&short).attempts, 0);

    fixture.mutes.release(short.target());
    fixture.runtime.advance(20 * SECOND);
    assert_eq!(expire(&fixture).await.expired, 1);
    assert!(
        fixture.mutes.held.lock().expect("锁表可用").is_empty(),
        "做完把锁放掉"
    );
}

#[tokio::test]
async fn 私人房间里他此刻没有发言权时到期不还给他() {
    let fixture = Fixture::new();
    let short = apply(&fixture, mute(2, Some(HOUR))).await;
    // 禁言期间房主在房间设置里关掉了他的发言权。
    *fixture.mutes.may_speak.lock().expect("发言权锁可用") = false;

    fixture.runtime.advance(HOUR);
    assert_eq!(expire(&fixture).await.expired, 1);

    assert!(fixture.effects.reversed_rooms().is_empty());
    assert_eq!(
        status_of(&fixture, &short),
        ModerationActionStatus::Reversed
    );
}

#[tokio::test]
async fn 踢出到期只记解除_不替管理员发邀请() {
    let fixture = Fixture::new();
    let kick = apply(
        &fixture,
        ApplyModerationAction {
            kind: ModerationActionKind::Kick,
            expires_at: Some(time(NOW + HOUR)),
            ..action_request()
        },
    )
    .await;

    fixture.runtime.advance(HOUR);
    assert_eq!(expire(&fixture).await.expired, 1);

    assert!(fixture.effects.reversed_rooms().is_empty());
    assert_eq!(status_of(&fixture, &kick), ModerationActionStatus::Reversed);
}

#[tokio::test]
async fn 房间关了以后到期照样记成解除() {
    let fixture = Fixture::new();
    let action = apply(&fixture, mute(2, Some(HOUR))).await;
    // 比如房主删了账户，私人房间跟着归档，没有活跃分片了。
    fixture.authority.rooms.lock().expect("分片锁可用").clear();

    fixture.runtime.advance(HOUR);
    assert_eq!(expire(&fixture).await.expired, 1);

    assert!(fixture.effects.reversed_rooms().is_empty());
    assert_eq!(
        status_of(&fixture, &action),
        ModerationActionStatus::Reversed
    );
}

#[tokio::test]
async fn 他的_matrix_账号没了以后到期不动_matrix_照样记成解除() {
    let fixture = Fixture::new();
    let action = apply(&fixture, mute(2, Some(HOUR))).await;
    *fixture.mutes.account_gone.lock().expect("账号锁可用") = true;

    fixture.runtime.advance(HOUR);
    assert_eq!(expire(&fixture).await.expired, 1);

    assert!(fixture.effects.reversed_rooms().is_empty());
    assert_eq!(
        status_of(&fixture, &action),
        ModerationActionStatus::Reversed
    );
}

#[tokio::test]
async fn 撤回禁言时同一个人还有别的禁言在生效_不放人_在每个分片上确保禁着() {
    let fixture = Fixture::new();
    fixture
        .authority
        .public_lobby(&["!busy:matrix.test", "!quiet:matrix.test"]);
    let first = apply(&fixture, mute(2, None)).await;
    let second = apply(&fixture, mute(2, None)).await;
    let applied_before = fixture.effects.applied_rooms().len();

    let reversed = reverse(&fixture, &first).await.expect("撤回应成功");

    assert_eq!(reversed.status(), ModerationActionStatus::Reversed);
    assert!(
        fixture.effects.reversed_rooms().is_empty(),
        "另一条禁言还在生效，不能撤掉发言权"
    );
    assert_eq!(
        fixture.effects.applied_rooms()[applied_before..],
        ["!busy:matrix.test", "!quiet:matrix.test"],
        "把还生效的那条在每个分片上再落一次"
    );
    assert_eq!(
        status_of(&fixture, &second),
        ModerationActionStatus::Applied
    );
    assert!(
        fixture.mutes.held.lock().expect("锁表可用").is_empty(),
        "做完把锁放掉"
    );
}

#[tokio::test]
async fn 撤回禁言时私人房间里他此刻没有发言权_不还给他() {
    let fixture = Fixture::new();
    let action = apply(&fixture, mute(2, None)).await;
    // 禁言期间房主在房间设置里关掉了他的发言权。
    *fixture.mutes.may_speak.lock().expect("发言权锁可用") = false;

    let reversed = reverse(&fixture, &action).await.expect("撤回应成功");

    assert_eq!(reversed.status(), ModerationActionStatus::Reversed);
    assert!(fixture.effects.reversed_rooms().is_empty());
}

#[tokio::test]
async fn 撤回禁言时有一条禁言正在落_回冲突什么也不动() {
    let fixture = Fixture::new();
    let action = apply(&fixture, mute(2, None)).await;
    let applied_before = fixture.effects.applied_rooms().len();
    landing_mute(&fixture, 2);

    let failure = reverse(&fixture, &action)
        .await
        .expect_err("有一条正在落时定不下来");

    assert_eq!(failure.kind(), ModerationFailureKind::Conflict);
    assert!(fixture.effects.reversed_rooms().is_empty());
    assert_eq!(fixture.effects.applied_rooms().len(), applied_before);
    assert_eq!(
        status_of(&fixture, &action),
        ModerationActionStatus::Applied
    );
    assert!(
        !fixture
            .repository
            .audit_actions(action.id())
            .iter()
            .any(|audit| audit.starts_with("moderation.action.reverse")),
        "什么也没做，不写撤回的审计"
    );
    assert!(
        fixture.mutes.held.lock().expect("锁表可用").is_empty(),
        "定不下来也把锁放掉"
    );
}

#[tokio::test]
async fn 撤回禁言时别处拿着他的禁言锁_回冲突_放掉以后能撤回() {
    let fixture = Fixture::new();
    let action = apply(&fixture, mute(2, None)).await;
    fixture.mutes.hold(action.target());

    let failure = reverse(&fixture, &action)
        .await
        .expect_err("别处正在处理这个人");
    assert_eq!(failure.kind(), ModerationFailureKind::Conflict);
    assert!(fixture.effects.reversed_rooms().is_empty());
    assert_eq!(
        status_of(&fixture, &action),
        ModerationActionStatus::Applied
    );

    fixture.mutes.release(action.target());
    let reversed = reverse(&fixture, &action).await.expect("放掉以后能撤回");
    assert_eq!(reversed.status(), ModerationActionStatus::Reversed);
    assert_eq!(fixture.effects.reversed_rooms(), ["!room:matrix.test"]);
}

#[tokio::test]
async fn 撤回禁言时_matrix_没撤成_记下撤回失败_放掉锁() {
    let fixture = Fixture::new();
    let action = apply(&fixture, mute(2, None)).await;
    *fixture.effects.failure.lock().expect("副作用锁可用") = Some(MatrixFailure::new(
        MatrixOperation::Ban,
        MatrixFailureKind::DependencyUnavailable,
    ));

    let failure = reverse(&fixture, &action).await.expect_err("Matrix 没撤成");

    assert_eq!(failure.kind(), ModerationFailureKind::DependencyUnavailable);
    assert_eq!(
        status_of(&fixture, &action),
        ModerationActionStatus::Applied
    );
    assert!(
        fixture
            .repository
            .audit_actions(action.id())
            .contains(&"moderation.action.reverse_failed".to_owned())
    );
    assert!(fixture.mutes.held.lock().expect("锁表可用").is_empty());
}

#[tokio::test]
async fn 撤回禁言时动完再看一眼又多了一条正在落的禁言_确保禁着再记成撤回() {
    let fixture = Fixture::new();
    let action = apply(&fixture, mute(2, None)).await;
    // 管理员刚撤，另一个管理员又禁了他：他那条先记成正在落，再去动 Matrix。
    let landing = pending_mute(2, time(NOW));
    let repository = fixture.repository.clone();
    fixture.mutes.between_reads(move || {
        repository.actions.lock().expect("动作锁可用").push(landing);
    });

    let reversed = reverse(&fixture, &action)
        .await
        .expect("撤回不叫管理员等下一轮");

    assert_eq!(reversed.status(), ModerationActionStatus::Reversed);
    assert_eq!(fixture.effects.reversed_rooms(), ["!room:matrix.test"]);
    let calls = fixture.calls.lock().expect("调用锁可用").clone();
    assert_eq!(
        calls.iter().rev().find(|call| call.starts_with("effect_")),
        Some(&"effect_apply"),
        "正在落的那条当它会生效，最后落在 Matrix 上的是禁言"
    );
}

#[tokio::test]
async fn 限时隐藏到期时回到消息所在的分片撤() {
    let fixture = Fixture::new();
    fixture
        .authority
        .public_lobby(&["!busy:matrix.test", "!quiet:matrix.test"]);
    fixture.effects.put_event("!quiet:matrix.test", SPAM_EVENT);
    apply(
        &fixture,
        ApplyModerationAction {
            expires_at: Some(time(NOW + HOUR)),
            ..hide_request(SPAM_EVENT)
        },
    )
    .await;

    fixture.runtime.advance(HOUR);
    assert_eq!(expire(&fixture).await.expired, 1);

    assert_eq!(fixture.effects.reversed_rooms(), ["!quiet:matrix.test"]);
}

#[tokio::test]
async fn 一轮最多领二十个_剩下的下一轮接着做() {
    let fixture = Fixture::new();
    for target in 10..35 {
        apply(&fixture, mute(target, Some(HOUR))).await;
    }

    fixture.runtime.advance(HOUR);
    assert_eq!(expire(&fixture).await.expired, 20);
    assert_eq!(expire(&fixture).await.expired, 5);
    assert_eq!(expire(&fixture).await, ModerationExpiryOutcome::default());
}

#[tokio::test]
async fn 管理员撤回时刚好到期解除了_交回已经解除的记录() {
    let fixture = Fixture::new();
    let action = apply(&fixture, mute(2, Some(1_000))).await;
    fixture.runtime.advance(2_000);
    // 管理员点撤回的同时，定时任务先一步在到期那一刻把它解除了。
    *fixture
        .repository
        .reversed_elsewhere_at
        .lock()
        .expect("撤销锁可用") = Some(time(NOW + 1_000));

    let reversed = fixture
        .service
        .reverse_action(ReverseModerationAction {
            actor: actor(true),
            action_id: action.id(),
            impact_acknowledged: true,
        })
        .await
        .expect("已经解除了就照实交回，不报冲突");

    assert_eq!(reversed.status(), ModerationActionStatus::Reversed);
    assert_eq!(reversed.reversed_at(), Some(time(NOW + 1_000)));
    assert!(
        !fixture
            .repository
            .audit_actions(action.id())
            .contains(&"moderation.action.reversed".to_owned())
    );
}

#[tokio::test]
async fn 到期解除时管理员刚好撤回了_这一轮不算解除() {
    let fixture = Fixture::new();
    let action = apply(&fixture, mute(2, Some(HOUR))).await;
    fixture.runtime.advance(HOUR + 1_000);
    *fixture
        .repository
        .reversed_elsewhere_at
        .lock()
        .expect("撤销锁可用") = Some(time(NOW + HOUR));

    assert_eq!(expire(&fixture).await, ModerationExpiryOutcome::default());

    assert_eq!(
        status_of(&fixture, &action),
        ModerationActionStatus::Reversed
    );
    assert!(
        !fixture
            .repository
            .audit_actions(action.id())
            .contains(&EXPIRED.to_owned())
    );
}

async fn apply(fixture: &Fixture, request: ApplyModerationAction) -> ModerationAction {
    fixture
        .service
        .apply_action(request)
        .await
        .expect("治理动作应成功")
}

async fn reverse(
    fixture: &Fixture,
    action: &ModerationAction,
) -> ModerationResult<ModerationAction> {
    fixture
        .service
        .reverse_action(ReverseModerationAction {
            actor: actor(true),
            action_id: action.id(),
            impact_acknowledged: true,
        })
        .await
}

async fn expire(fixture: &Fixture) -> ModerationExpiryOutcome {
    fixture
        .service
        .expire_due_actions()
        .await
        .expect("到期解除应跑完")
}

fn status_of(fixture: &Fixture, action: &ModerationAction) -> ModerationActionStatus {
    fixture.repository.action(action.id()).status()
}

/// 禁言一个人：`expires_in` 是从 `NOW` 起过多久到期，`None` 是不限时。
fn mute(target: u128, expires_in: Option<i64>) -> ApplyModerationAction {
    ApplyModerationAction {
        kind: ModerationActionKind::Mute,
        target: ModerationTarget::new(
            ModerationTargetKind::Principal,
            PrincipalId::from_uuid(Uuid::from_u128(target)).to_string(),
        )
        .expect("目标有效"),
        expires_at: expires_in.map(|offset| time(NOW + offset)),
        ..action_request()
    }
}

/// 一条正在落的禁言：已经记下，还没落到 Matrix 上。
fn pending_mute(target: u128, starts_at: UtcMillis) -> ModerationAction {
    ModerationAction::reserve(
        ModerationActionId::from_uuid(Uuid::now_v7()),
        None,
        principal_id(),
        room_id(),
        ModerationActionKind::Mute,
        ModerationTarget::new(
            ModerationTargetKind::Principal,
            PrincipalId::from_uuid(Uuid::from_u128(target)).to_string(),
        )
        .expect("目标有效"),
        ModerationReason::Spam,
        starts_at,
        None,
    )
    .expect("禁言有效")
}

/// 此刻有人禁言这个人，还没落完。
fn landing_mute(fixture: &Fixture, target: u128) -> ModerationAction {
    let mute = pending_mute(target, fixture.runtime.now());
    fixture
        .repository
        .actions
        .lock()
        .expect("动作锁可用")
        .push(mute.clone());
    mute
}

fn hide_request(event: &str) -> ApplyModerationAction {
    ApplyModerationAction {
        kind: ModerationActionKind::Hide,
        target: ModerationTarget::new(ModerationTargetKind::Event, event).expect("事件目标有效"),
        expires_at: None,
        ..action_request()
    }
}

fn person_request(kind: ModerationActionKind) -> ApplyModerationAction {
    ApplyModerationAction {
        kind,
        expires_at: None,
        ..action_request()
    }
}

fn matrix_room(room: &str) -> MatrixRoomId {
    MatrixRoomId::new(room).expect("测试 Matrix 房间有效")
}

fn report_request() -> SubmitModerationReport {
    SubmitModerationReport {
        actor: actor(false),
        case_id: ModerationCaseId::from_uuid(Uuid::now_v7()),
        target: ModerationTarget::new(ModerationTargetKind::Event, "$event:matrix.test")
            .expect("目标有效"),
        reason: ModerationReason::Spam,
        description: "只描述必要事实".to_owned(),
        evidence: ModerationEvidence::new(
            Some(room_id()),
            Some("$event:matrix.test".to_owned()),
            None,
            true,
        )
        .expect("证据有效"),
    }
}

fn action_request() -> ApplyModerationAction {
    ApplyModerationAction {
        actor: actor(true),
        action_id: ModerationActionId::from_uuid(Uuid::now_v7()),
        case_id: None,
        room_catalog_id: room_id(),
        kind: ModerationActionKind::Mute,
        target: ModerationTarget::new(ModerationTargetKind::Principal, target_id().to_string())
            .expect("目标有效"),
        reason: ModerationReason::Spam,
        expires_at: Some(time(NOW + 600_000)),
        impact_acknowledged: true,
    }
}

fn actor(recent: bool) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: principal_id(),
        matrix_user_id: "@actor:matrix.test".to_owned(),
        display_name: "治理者".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(NOW - 1_000),
        expires_at: time(NOW + 60_000),
        recently_authenticated: recent,
    }
}

fn principal_id() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(1))
}

fn target_id() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::from_u128(2))
}

fn room_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::from_u128(3))
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
