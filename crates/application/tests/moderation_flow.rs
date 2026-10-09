use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    moderation::{
        ApplyModerationAction, InspectModerationCapabilities, ListModerationAudit,
        ListRoomModerationCases, ModerationDependencies, ModerationFailureKind, ModerationService,
        ModerationUseCases, ReverseModerationAction, SubmitModerationReport,
    },
    persistence::RepositoryResult,
    ports::{
        Clock, MatrixEventId, MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult,
        MatrixRoomId, MatrixUserId, ModerationActionReservationOutcome, ModerationAuthority,
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
    rooms::RoomCatalogKind,
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
            room_kind: Mutex::new(RoomCatalogKind::PrivateRoom),
            rooms: Mutex::new(vec![matrix_room("!room:matrix.test")]),
        });
        let effects = Arc::new(FakeEffects {
            calls: calls.clone(),
            ..FakeEffects::default()
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
