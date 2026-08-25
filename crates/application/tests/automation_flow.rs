use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    automation::{
        AuthorizeAutomationSend, AutomationAuthorizationOutcome, AutomationDependencies,
        AutomationFailureKind, AutomationSendDenial, AutomationService, AutomationUseCases,
        CreateAutomationGrant, RevokeAutomationGrant,
    },
    devices::AuthenticatedDevice,
    persistence::RepositoryResult,
    ports::{
        AutomationConsumptionOutcome, AutomationConsumptionRequest, AutomationDecisionRecord,
        AutomationGrantRecord, AutomationGrantRepository, AutomationGrantRevocationOutcome,
        AutomationScopeAuthority, AutomationScopeAuthorityRequest, AutomationSendAuthority,
        AutomationSendAuthorityRequest, Clock, MatrixFailure, MatrixFailureKind, MatrixOperation,
        MatrixPowerLevel, MatrixResult, MatrixRoomAuthority, MatrixRoomAuthorityGateway,
        MatrixRoomId, MatrixUserId, PortFuture, PrincipalAccount,
    },
};
use agent_room_domain::{
    identity::Principal,
    ids::{
        AgentId, AgentInstanceId, AutomationGrantId, DeviceId, MessageSubmissionId, PrincipalId,
        RoomCatalogId,
    },
    policy::{
        AutomationAudience, AutomationGrant, AutomationGrantDenial, AutomationGrantFields,
        AutomationGrantLimits, AutomationGrantScope, AutomationGrantStatus, AutomationMessageKind,
        AutomationMessageKinds, AutomationRiskScanOutcome, AutomationUsageSnapshot,
    },
    time::{DurationMillis, UtcMillis},
};
use uuid::Uuid;

const NOW: i64 = 1_700_000_000_000;

#[derive(Clone)]
struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> UtcMillis {
        time(NOW)
    }
}

struct FakeGrants {
    record: Mutex<Option<AutomationGrantRecord>>,
    decisions: Mutex<Vec<AutomationDecisionRecord>>,
    calls: Arc<Mutex<Vec<&'static str>>>,
    consumption: Mutex<Option<AutomationConsumptionOutcome>>,
}

impl FakeGrants {
    fn new(record: AutomationGrantRecord, calls: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            record: Mutex::new(Some(record)),
            decisions: Mutex::new(Vec::new()),
            calls,
            consumption: Mutex::new(None),
        }
    }

    fn set_consumption(&self, outcome: AutomationConsumptionOutcome) {
        *self.consumption.lock().expect("消费结果锁可用") = Some(outcome);
    }
}

impl AutomationGrantRepository for FakeGrants {
    fn create<'a>(
        &'a self,
        grant: &'a AutomationGrant,
    ) -> PortFuture<'a, RepositoryResult<AutomationGrantRecord>> {
        self.calls.lock().expect("调用顺序锁可用").push("create");
        let record = AutomationGrantRecord {
            grant: grant.clone(),
            usage: AutomationUsageSnapshot::default(),
        };
        *self.record.lock().expect("授权锁可用") = Some(record.clone());
        Box::pin(async move { Ok(record) })
    }

    fn list_for_principal(
        &self,
        _principal_id: PrincipalId,
        _now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Vec<AutomationGrantRecord>>> {
        let records = self
            .record
            .lock()
            .expect("授权锁可用")
            .clone()
            .into_iter()
            .collect();
        Box::pin(async move { Ok(records) })
    }

    fn find(
        &self,
        _grant_id: AutomationGrantId,
        _now: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<Option<AutomationGrantRecord>>> {
        self.calls.lock().expect("调用顺序锁可用").push("find");
        let record = self.record.lock().expect("授权锁可用").clone();
        Box::pin(async move { Ok(record) })
    }

    fn revoke(
        &self,
        principal_id: PrincipalId,
        _grant_id: AutomationGrantId,
        revoked_at: UtcMillis,
    ) -> PortFuture<'_, RepositoryResult<AutomationGrantRevocationOutcome>> {
        let mut guard = self.record.lock().expect("授权锁可用");
        let outcome = match guard.as_mut() {
            Some(record) if record.grant.grantor_id() == principal_id => {
                let changed = record.grant.revoke(revoked_at).expect("测试撤销有效");
                if changed {
                    AutomationGrantRevocationOutcome::Revoked(record.clone())
                } else {
                    AutomationGrantRevocationOutcome::AlreadyRevoked(record.clone())
                }
            }
            Some(_) | None => AutomationGrantRevocationOutcome::NotFound,
        };
        Box::pin(async move { Ok(outcome) })
    }

    fn consume<'a>(
        &'a self,
        _request: &'a AutomationConsumptionRequest,
    ) -> PortFuture<'a, RepositoryResult<AutomationConsumptionOutcome>> {
        self.calls.lock().expect("调用顺序锁可用").push("consume");
        let configured = self.consumption.lock().expect("消费结果锁可用").clone();
        let fallback = self.record.lock().expect("授权锁可用").clone().map_or(
            AutomationConsumptionOutcome::NotFound,
            |record| AutomationConsumptionOutcome::Consumed {
                record,
                reused: false,
            },
        );
        Box::pin(async move { Ok(configured.unwrap_or(fallback)) })
    }

    fn record_decision<'a>(
        &'a self,
        record: &'a AutomationDecisionRecord,
    ) -> PortFuture<'a, RepositoryResult<()>> {
        self.calls
            .lock()
            .expect("调用顺序锁可用")
            .push("record_denial");
        self.decisions
            .lock()
            .expect("决策记录锁可用")
            .push(record.clone());
        Box::pin(async { Ok(()) })
    }
}

struct FakeAuthority {
    may_create: bool,
    send: Mutex<Option<AutomationSendAuthority>>,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl AutomationScopeAuthority for FakeAuthority {
    fn may_create<'a>(
        &'a self,
        _request: &'a AutomationScopeAuthorityRequest,
    ) -> PortFuture<'a, RepositoryResult<bool>> {
        self.calls
            .lock()
            .expect("调用顺序锁可用")
            .push("scope_create");
        let allowed = self.may_create;
        Box::pin(async move { Ok(allowed) })
    }

    fn inspect_send<'a>(
        &'a self,
        _request: &'a AutomationSendAuthorityRequest,
    ) -> PortFuture<'a, RepositoryResult<Option<AutomationSendAuthority>>> {
        self.calls
            .lock()
            .expect("调用顺序锁可用")
            .push("scope_send");
        let authority = self.send.lock().expect("作用域权威锁可用").clone();
        Box::pin(async move { Ok(authority) })
    }
}

struct FakeMatrixAuthority {
    authority: Mutex<MatrixResult<MatrixRoomAuthority>>,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl MatrixRoomAuthorityGateway for FakeMatrixAuthority {
    fn inspect_room_authority<'a>(
        &'a self,
        _room_id: &'a MatrixRoomId,
        _user_id: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<MatrixRoomAuthority>> {
        self.calls.lock().expect("调用顺序锁可用").push("matrix");
        let authority = *self.authority.lock().expect("Matrix 权威锁可用");
        Box::pin(async move { authority })
    }
}

struct Fixture {
    service: AutomationService,
    grants: Arc<FakeGrants>,
    authority: Arc<FakeAuthority>,
    matrix: Arc<FakeMatrixAuthority>,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl Fixture {
    fn new(grant: AutomationGrant) -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let grants = Arc::new(FakeGrants::new(
            AutomationGrantRecord {
                grant,
                usage: AutomationUsageSnapshot::default(),
            },
            calls.clone(),
        ));
        let authority = Arc::new(FakeAuthority {
            may_create: true,
            send: Mutex::new(Some(AutomationSendAuthority {
                agent_matrix_user_id: matrix_user_id(),
                contains_unknown_recipients: false,
            })),
            calls: calls.clone(),
        });
        let matrix = Arc::new(FakeMatrixAuthority {
            authority: Mutex::new(Ok(MatrixRoomAuthority::joined(MatrixPowerLevel::finite(0)))),
            calls: calls.clone(),
        });
        let service = AutomationService::new(AutomationDependencies {
            grants: grants.clone(),
            authority: authority.clone(),
            matrix_authority: matrix.clone(),
            clock: Arc::new(FixedClock),
        });
        Self {
            service,
            grants,
            authority,
            matrix,
            calls,
        }
    }
}

#[tokio::test]
async fn 创建授权同时要求影响确认近期认证和当前作用域权威() {
    let fixture = Fixture::new(grant(AutomationAudience::KnownRoomMembers, false));
    let mut request = create_request();
    request.impact_acknowledged = false;
    let missing_impact = fixture
        .service
        .create(request)
        .await
        .expect_err("没有影响确认必须拒绝");
    assert_eq!(missing_impact.kind(), AutomationFailureKind::Forbidden);
    assert!(fixture.calls.lock().expect("调用顺序锁可用").is_empty());

    let created = fixture
        .service
        .create(create_request())
        .await
        .expect("近期认证且权威允许时可创建");
    assert_eq!(created.grant.grantor_id(), principal_id());
    assert_eq!(
        *fixture.calls.lock().expect("调用顺序锁可用"),
        ["scope_create", "create"]
    );
}

#[tokio::test]
async fn 自动发送严格按权威顺序校验后才原子消费() {
    let fixture = Fixture::new(grant(AutomationAudience::KnownRoomMembers, false));
    let outcome = fixture
        .service
        .authorize_send(send_request())
        .await
        .expect("完整权威链可用");
    assert!(matches!(
        outcome,
        AutomationAuthorizationOutcome::Authorized(receipt) if !receipt.reused
    ));
    assert_eq!(
        *fixture.calls.lock().expect("调用顺序锁可用"),
        ["find", "scope_send", "matrix", "consume"]
    );
}

#[tokio::test]
async fn 陌生受众越界在_matrix_调用之前拒绝并留存原因() {
    let fixture = Fixture::new(grant(AutomationAudience::KnownRoomMembers, false));
    fixture
        .authority
        .send
        .lock()
        .expect("作用域权威锁可用")
        .as_mut()
        .expect("权威存在")
        .contains_unknown_recipients = true;

    let outcome = fixture
        .service
        .authorize_send(send_request())
        .await
        .expect("业务拒绝不是依赖故障");
    assert_eq!(
        outcome,
        AutomationAuthorizationOutcome::Denied(AutomationSendDenial::Grant(
            AutomationGrantDenial::UnknownRecipientNotAllowed,
        ))
    );
    assert_eq!(
        *fixture.calls.lock().expect("调用顺序锁可用"),
        ["find", "scope_send", "record_denial"]
    );
    let decisions = fixture.grants.decisions.lock().expect("决策记录锁可用");
    assert_eq!(
        decisions[0].decision_code,
        "automation.unknown_recipient_not_allowed"
    );
}

#[tokio::test]
async fn 房间成员或发言权变化立即阻止发送() {
    let fixture = Fixture::new(grant(AutomationAudience::KnownRoomMembers, false));
    *fixture.matrix.authority.lock().expect("Matrix 权威锁可用") =
        Ok(MatrixRoomAuthority::joined_with_message_threshold(
            MatrixPowerLevel::finite(0),
            MatrixPowerLevel::finite(50),
        ));

    let outcome = fixture
        .service
        .authorize_send(send_request())
        .await
        .expect("权限变化产生显式业务拒绝");
    assert_eq!(
        outcome,
        AutomationAuthorizationOutcome::Denied(AutomationSendDenial::MatrixPermissionDenied)
    );
    assert_eq!(
        *fixture.calls.lock().expect("调用顺序锁可用"),
        ["find", "scope_send", "matrix", "record_denial"]
    );
}

#[tokio::test]
async fn matrix_状态不确定时记录原因并失败关闭() {
    let fixture = Fixture::new(grant(AutomationAudience::KnownRoomMembers, false));
    *fixture.matrix.authority.lock().expect("Matrix 权威锁可用") = Err(MatrixFailure::new(
        MatrixOperation::InspectRoomAuthority,
        MatrixFailureKind::DependencyUnavailable,
    ));

    let failure = fixture
        .service
        .authorize_send(send_request())
        .await
        .expect_err("Matrix 不确定不得继续发送");
    assert_eq!(failure.kind(), AutomationFailureKind::DependencyUnavailable);
    assert_eq!(
        fixture.grants.decisions.lock().expect("决策记录锁可用")[0].decision_code,
        "automation.matrix_authority_unavailable"
    );
}

#[tokio::test]
async fn 并发窗口在最终消费时耗尽仍拒绝() {
    let fixture = Fixture::new(grant(AutomationAudience::KnownRoomMembers, false));
    fixture
        .grants
        .set_consumption(AutomationConsumptionOutcome::Denied(
            AutomationGrantDenial::RateLimitExceeded,
        ));

    let outcome = fixture
        .service
        .authorize_send(send_request())
        .await
        .expect("原子消费拒绝是业务结果");
    assert_eq!(
        outcome,
        AutomationAuthorizationOutcome::Denied(AutomationSendDenial::Grant(
            AutomationGrantDenial::RateLimitExceeded,
        ))
    );
}

#[tokio::test]
async fn 撤销幂等且下一次发送无需_agent_配合就被阻止() {
    let fixture = Fixture::new(grant(AutomationAudience::KnownRoomMembers, false));
    let first = fixture
        .service
        .revoke(RevokeAutomationGrant {
            actor: web_actor(true),
            grant_id: grant_id(),
        })
        .await
        .expect("首次撤销成功");
    let second = fixture
        .service
        .revoke(RevokeAutomationGrant {
            actor: web_actor(true),
            grant_id: grant_id(),
        })
        .await
        .expect("重复撤销幂等");
    assert_eq!(first.grant.status(), AutomationGrantStatus::Revoked);
    assert_eq!(second.grant.status(), AutomationGrantStatus::Revoked);

    let outcome = fixture
        .service
        .authorize_send(send_request())
        .await
        .expect("撤销后为显式业务拒绝");
    assert_eq!(
        outcome,
        AutomationAuthorizationOutcome::Denied(AutomationSendDenial::Grant(
            AutomationGrantDenial::Revoked,
        ))
    );
}

fn create_request() -> CreateAutomationGrant {
    CreateAutomationGrant {
        actor: web_actor(true),
        grant_id: grant_id(),
        scope: scope(AutomationAudience::KnownRoomMembers, false),
        max_messages_per_minute: 2,
        max_total_messages: Some(3),
        lifetime: DurationMillis::new(60_000).expect("期限有效"),
        impact_acknowledged: true,
    }
}

fn send_request() -> AuthorizeAutomationSend {
    AuthorizeAutomationSend {
        actor: device_actor(),
        grant_id: grant_id(),
        submission_id: submission_id(),
        agent_id: agent_id(),
        agent_instance_id: instance_id(),
        room_catalog_id: room_id(),
        matrix_room_id: matrix_room_id(),
        is_reply: true,
        risk_scan: AutomationRiskScanOutcome::Passed,
    }
}

fn grant(audience: AutomationAudience, requires_risk_scan: bool) -> AutomationGrant {
    AutomationGrant::issue(AutomationGrantFields {
        id: grant_id(),
        grantor_id: principal_id(),
        scope: scope(audience, requires_risk_scan),
        limits: limits(),
        created_at: time(NOW),
    })
    .expect("测试授权有效")
}

fn scope(audience: AutomationAudience, requires_risk_scan: bool) -> AutomationGrantScope {
    AutomationGrantScope::new(
        agent_id(),
        Some(instance_id()),
        room_id(),
        AutomationMessageKinds::new([AutomationMessageKind::Reply]).expect("类别有效"),
        audience,
        requires_risk_scan,
    )
    .expect("作用域有效")
}

fn limits() -> AutomationGrantLimits {
    AutomationGrantLimits::new(3, Some(20), time(NOW), time(NOW + 60_000)).expect("限额有效")
}

fn web_actor(recently_authenticated: bool) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: principal_id(),
        matrix_user_id: "@owner:matrix.test".to_owned(),
        display_name: "测试用户".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(NOW - 1_000),
        expires_at: time(NOW + 60_000),
        recently_authenticated,
    }
}

fn device_actor() -> AuthenticatedDevice {
    AuthenticatedDevice {
        account: PrincipalAccount {
            principal: Principal::new(principal_id()),
            matrix_user_id: "@owner:matrix.test".to_owned(),
            display_name: "测试用户".to_owned(),
            avatar_content_id: None,
            locale: "zh-CN".to_owned(),
        },
        device_id: device_id(),
        access_token_expires_at: time(NOW + 60_000),
    }
}

fn matrix_room_id() -> MatrixRoomId {
    MatrixRoomId::new("!automation:matrix.test").expect("Matrix 房间有效")
}

fn matrix_user_id() -> MatrixUserId {
    MatrixUserId::new("@agent:matrix.test").expect("Matrix 用户有效")
}

fn principal_id() -> PrincipalId {
    PrincipalId::from_uuid(uuid("0198b601-77a1-7bb8-83eb-a8fe68c97e41"))
}

fn device_id() -> DeviceId {
    DeviceId::from_uuid(uuid("0198b601-77a1-7bb8-83eb-a8fe68c97e42"))
}

fn agent_id() -> AgentId {
    AgentId::from_uuid(uuid("0198b601-77a1-7bb8-83eb-a8fe68c97e43"))
}

fn instance_id() -> AgentInstanceId {
    AgentInstanceId::from_uuid(uuid("0198b601-77a1-7bb8-83eb-a8fe68c97e44"))
}

fn room_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(uuid("0198b601-77a1-7bb8-83eb-a8fe68c97e45"))
}

fn grant_id() -> AutomationGrantId {
    AutomationGrantId::from_uuid(uuid("0198b601-77a1-7bb8-83eb-a8fe68c97e46"))
}

fn submission_id() -> MessageSubmissionId {
    MessageSubmissionId::from_uuid(uuid("0198b601-77a1-7bb8-83eb-a8fe68c97e47"))
}

fn uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).expect("测试 UUID 有效")
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
