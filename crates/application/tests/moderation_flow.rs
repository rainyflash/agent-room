use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    moderation::{
        ApplyModerationAction, ListModerationAudit, ListRoomModerationCases,
        ModerationDependencies, ModerationFailureKind, ModerationService, ModerationUseCases,
        ReverseModerationAction, SubmitModerationReport,
    },
    persistence::RepositoryResult,
    ports::{
        Clock, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId,
        MatrixUserId, ModerationActionReservationOutcome, ModerationAuthority,
        ModerationEffectGateway, ModerationEffectTarget, ModerationIdentifierFactory,
        ModerationReportPolicy, ModerationReportSubmissionOutcome, ModerationRepository,
        ModerationRoomContext, PortFuture,
    },
};
use agent_room_domain::{
    ids::{AuditEventId, ModerationActionId, ModerationCaseId, PrincipalId, RoomCatalogId},
    moderation::{
        ModerationAction, ModerationActionKind, ModerationActionStatus, ModerationAuditEvent,
        ModerationCase, ModerationEvidence, ModerationReason, ModerationRole, ModerationTarget,
        ModerationTargetKind,
    },
    time::{DurationMillis, UtcMillis},
};
use uuid::Uuid;

const NOW: i64 = 1_700_000_000_000;

#[derive(Clone)]
struct TestRuntime;

impl Clock for TestRuntime {
    fn now(&self) -> UtcMillis {
        time(NOW)
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
}

impl FakeRepository {
    fn new(calls: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            cases: Mutex::new(Vec::new()),
            actions: Mutex::new(Vec::new()),
            audits: Mutex::new(Vec::new()),
            calls,
            rate_limit_at: Mutex::new(None),
        }
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
        *stored = action.clone();
        self.audits.lock().expect("审计锁可用").push(audit.clone());
        let finalized = action.clone();
        Box::pin(async move { Ok(finalized) })
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
        let target_matrix_user_id = (target.kind() == ModerationTargetKind::Principal)
            .then(|| MatrixUserId::new("@target:matrix.test").expect("测试 Matrix 用户有效"));
        Box::pin(async move {
            Ok(Some(ModerationRoomContext {
                role,
                matrix_room_id: MatrixRoomId::new("!room:matrix.test")
                    .expect("测试 Matrix 房间有效"),
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

struct FakeEffects {
    calls: Arc<Mutex<Vec<&'static str>>>,
    failure: Mutex<Option<MatrixFailure>>,
}

impl ModerationEffectGateway for FakeEffects {
    fn apply<'a>(
        &'a self,
        _action: &'a ModerationAction,
        _target: &'a ModerationEffectTarget,
    ) -> PortFuture<'a, MatrixResult<()>> {
        self.calls.lock().expect("调用锁可用").push("effect_apply");
        let result = self
            .failure
            .lock()
            .expect("副作用锁可用")
            .map_or(Ok(()), Err);
        Box::pin(async move { result })
    }

    fn reverse<'a>(
        &'a self,
        _action: &'a ModerationAction,
        _target: &'a ModerationEffectTarget,
    ) -> PortFuture<'a, MatrixResult<()>> {
        self.calls
            .lock()
            .expect("调用锁可用")
            .push("effect_reverse");
        let result = self
            .failure
            .lock()
            .expect("副作用锁可用")
            .map_or(Ok(()), Err);
        Box::pin(async move { result })
    }
}

struct Fixture {
    service: ModerationService,
    repository: Arc<FakeRepository>,
    authority: Arc<FakeAuthority>,
    effects: Arc<FakeEffects>,
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
        });
        let effects = Arc::new(FakeEffects {
            calls: calls.clone(),
            failure: Mutex::new(None),
        });
        let runtime = Arc::new(TestRuntime);
        let service = ModerationService::new(ModerationDependencies {
            repository: repository.clone(),
            authority: authority.clone(),
            effects: effects.clone(),
            identifiers: runtime.clone(),
            clock: runtime,
            report_policy: ModerationReportPolicy {
                maximum_reports: 5,
                window: DurationMillis::new(600_000).expect("窗口有效"),
            },
        });
        Self {
            service,
            repository,
            authority,
            effects,
            calls,
        }
    }
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
