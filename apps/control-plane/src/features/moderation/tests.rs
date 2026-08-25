use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    moderation::{
        ApplyModerationAction, ListModerationAudit, ListMyModerationCases, ListRoomModeration,
        ModerationResult, ModerationUseCases, ReverseModerationAction, SubmitModerationReport,
    },
    ports::{PortFuture, SecretValue},
};
use agent_room_domain::{
    ids::{AuditEventId, ModerationActionId, ModerationCaseId, PrincipalId, RoomCatalogId},
    moderation::{
        ModerationAction, ModerationActionKind, ModerationAuditEvent, ModerationAuditOutcome,
        ModerationCase, ModerationEvidence, ModerationReason, ModerationTarget,
        ModerationTargetKind,
    },
    time::UtcMillis,
};
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
    middleware,
};
use axum_extra::extract::cookie::Cookie;
use serde_json::{Value, json};
use tower::ServiceExt;
use url::Url;
use uuid::Uuid;

use super::{ModerationHttpState, router};

const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const CASE_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e47";
const ACTION_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e48";
const AUDIT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e49";
const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";

#[derive(Default)]
struct FakeModeration {
    report: Mutex<Option<SubmitModerationReport>>,
    action: Mutex<Option<ApplyModerationAction>>,
    reversal: Mutex<Option<ReverseModerationAction>>,
    audit: Mutex<Option<ListModerationAudit>>,
}

impl ModerationUseCases for FakeModeration {
    fn submit_report(
        &self,
        request: SubmitModerationReport,
    ) -> PortFuture<'_, ModerationResult<ModerationCase>> {
        *self.report.lock().expect("举报请求锁可用") = Some(request);
        Box::pin(async { Ok(case()) })
    }

    fn list_my_cases(
        &self,
        _request: ListMyModerationCases,
    ) -> PortFuture<'_, ModerationResult<Vec<ModerationCase>>> {
        Box::pin(async { Ok(vec![case()]) })
    }

    fn apply_action(
        &self,
        request: ApplyModerationAction,
    ) -> PortFuture<'_, ModerationResult<ModerationAction>> {
        *self.action.lock().expect("动作请求锁可用") = Some(request);
        Box::pin(async { Ok(applied_action()) })
    }

    fn reverse_action(
        &self,
        request: ReverseModerationAction,
    ) -> PortFuture<'_, ModerationResult<ModerationAction>> {
        *self.reversal.lock().expect("撤销请求锁可用") = Some(request);
        Box::pin(async { Ok(reversed_action()) })
    }

    fn list_room_actions(
        &self,
        _request: ListRoomModeration,
    ) -> PortFuture<'_, ModerationResult<Vec<ModerationAction>>> {
        Box::pin(async { Ok(vec![applied_action()]) })
    }

    fn list_audit(
        &self,
        request: ListModerationAudit,
    ) -> PortFuture<'_, ModerationResult<Vec<ModerationAuditEvent>>> {
        *self.audit.lock().expect("审计请求锁可用") = Some(request);
        Box::pin(async { Ok(vec![audit_event()]) })
    }
}

#[derive(Default)]
struct FakeAuthentication {
    requirements: Mutex<Vec<AuthenticationRequirement>>,
}

impl AuthenticationUseCases for FakeAuthentication {
    fn begin_login(
        &self,
        _request: BeginLogin,
    ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>> {
        Box::pin(async { unreachable!("治理路由不会开始登录") })
    }

    fn complete_login<'a>(
        &'a self,
        _request: CompleteLogin<'a>,
    ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
        Box::pin(async { unreachable!("治理路由不会完成登录") })
    }

    fn authenticate<'a>(
        &'a self,
        session_secret: &'a SecretValue,
        requirement: AuthenticationRequirement,
    ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>> {
        assert_eq!(session_secret.expose(), "session-secret");
        self.requirements
            .lock()
            .expect("认证要求锁可用")
            .push(requirement);
        Box::pin(async { Ok(actor()) })
    }

    fn logout<'a>(
        &'a self,
        _session_secret: &'a SecretValue,
    ) -> PortFuture<'a, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("治理路由不会注销") })
    }

    fn suspend_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("治理路由不会暂停主体") })
    }
}

#[tokio::test]
async fn 举报只映射显式证据并使用幂等案件标识() {
    let moderation = Arc::new(FakeModeration::default());
    let authentication = Arc::new(FakeAuthentication::default());
    let response = test_router(moderation.clone(), authentication.clone())
        .oneshot(session_request(
            Method::POST,
            "/moderation/cases",
            &report_body(),
            true,
            Some(CASE_UUID),
        ))
        .await
        .expect("举报路由可调用");

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let request = moderation
        .report
        .lock()
        .expect("举报请求锁可用")
        .clone()
        .expect("举报用例已调用");
    assert_eq!(request.case_id, case_id());
    assert_eq!(
        request.evidence.matrix_event_id(),
        Some("$event:matrix.test")
    );
    assert_eq!(request.evidence.reporter_submitted_excerpt(), None);
    assert!(request.evidence.end_to_end_encrypted());
    assert_eq!(
        authentication
            .requirements
            .lock()
            .expect("认证要求锁可用")
            .as_slice(),
        &[AuthenticationRequirement::ActiveSession]
    );
}

#[tokio::test]
async fn 治理动作和撤销都要求同源近期认证与显式影响确认() {
    let moderation = Arc::new(FakeModeration::default());
    let authentication = Arc::new(FakeAuthentication::default());
    let app = test_router(moderation.clone(), authentication.clone());
    let applied = app
        .clone()
        .oneshot(session_request(
            Method::POST,
            &format!("/rooms/{CATALOG_UUID}/moderation/actions"),
            &action_body(),
            true,
            Some(ACTION_UUID),
        ))
        .await
        .expect("动作路由可调用");
    let reversed = app
        .oneshot(session_request(
            Method::DELETE,
            &format!("/moderation/actions/{ACTION_UUID}"),
            &json!({ "impactAcknowledged": true }),
            true,
            None,
        ))
        .await
        .expect("撤销路由可调用");

    assert_eq!(applied.status(), StatusCode::CREATED);
    assert_eq!(reversed.status(), StatusCode::OK);
    let action = moderation
        .action
        .lock()
        .expect("动作请求锁可用")
        .clone()
        .expect("动作已调用");
    assert_eq!(action.action_id, action_id());
    assert_eq!(action.room_catalog_id, catalog_id());
    assert!(action.impact_acknowledged);
    assert!(
        moderation
            .reversal
            .lock()
            .expect("撤销请求锁可用")
            .as_ref()
            .expect("撤销已调用")
            .impact_acknowledged
    );
    assert_eq!(
        authentication
            .requirements
            .lock()
            .expect("认证要求锁可用")
            .as_slice(),
        &[
            AuthenticationRequirement::RecentAuthentication,
            AuthenticationRequirement::RecentAuthentication,
        ]
    );
}

#[tokio::test]
async fn 审计响应只含白名单字段且不能泄漏案件正文() {
    let moderation = Arc::new(FakeModeration::default());
    let response = test_router(moderation.clone(), Arc::new(FakeAuthentication::default()))
        .oneshot(session_request(
            Method::GET,
            &format!("/moderation/audit?roomCatalogId={CATALOG_UUID}&limit=20"),
            &json!({}),
            false,
            None,
        ))
        .await
        .expect("审计路由可调用");

    assert_eq!(response.status(), StatusCode::OK);
    let payload = response_json(response).await;
    assert_eq!(payload["events"][0]["action"], "moderation.action.applied");
    assert!(payload.to_string().find("description").is_none());
    assert!(payload.to_string().find("body").is_none());
    let request = moderation
        .audit
        .lock()
        .expect("审计请求锁可用")
        .clone()
        .expect("审计已调用");
    assert_eq!(request.room_catalog_id, Some(catalog_id()));
    assert_eq!(request.limit, 20);
}

#[tokio::test]
async fn 错误来源和缺失幂等键在业务调用前失败关闭() {
    let moderation = Arc::new(FakeModeration::default());
    let mut request = session_request(
        Method::POST,
        "/moderation/cases",
        &report_body(),
        true,
        None,
    );
    request.headers_mut().insert(
        header::ORIGIN,
        header::HeaderValue::from_static("https://evil.test"),
    );
    let response = test_router(moderation.clone(), Arc::new(FakeAuthentication::default()))
        .oneshot(request)
        .await
        .expect("拒绝响应可生成");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(moderation.report.lock().expect("举报请求锁可用").is_none());
}

fn test_router(
    moderation: Arc<FakeModeration>,
    authentication: Arc<FakeAuthentication>,
) -> axum::Router {
    router(ModerationHttpState::new(
        moderation,
        authentication,
        &Url::parse(FRONTEND_ORIGIN).expect("前端地址有效"),
    ))
    .layer(middleware::from_fn(crate::correlation::attach))
}

fn session_request(
    method: Method,
    uri: &str,
    body: &Value,
    include_origin: bool,
    idempotency_key: Option<&str>,
) -> Request<Body> {
    let session = Cookie::new("__Host-agent-room-session", "session-secret");
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, session.to_string());
    if include_origin {
        request = request.header(header::ORIGIN, FRONTEND_ORIGIN);
    }
    if let Some(idempotency_key) = idempotency_key {
        request = request.header("idempotency-key", idempotency_key);
    }
    request
        .body(Body::from(body.to_string()))
        .expect("会话 HTTP 请求有效")
}

fn report_body() -> Value {
    json!({
        "targetKind": "event",
        "targetReference": "$event:matrix.test",
        "reason": "harassment",
        "description": "只描述必要事实",
        "evidence": {
            "roomCatalogId": CATALOG_UUID,
            "matrixEventId": "$event:matrix.test",
            "endToEndEncrypted": true
        }
    })
}

fn action_body() -> Value {
    json!({
        "caseId": CASE_UUID,
        "kind": "hide",
        "targetKind": "event",
        "targetReference": "$event:matrix.test",
        "reason": "harassment",
        "impactAcknowledged": true
    })
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 64 * 1_024)
        .await
        .expect("响应正文可读取");
    serde_json::from_slice(&body).expect("响应正文是 JSON")
}

fn case() -> ModerationCase {
    ModerationCase::open(
        case_id(),
        principal_id(),
        ModerationTarget::new(ModerationTargetKind::Event, "$event:matrix.test")
            .expect("案件目标有效"),
        ModerationReason::Harassment,
        "只描述必要事实",
        ModerationEvidence::new(
            Some(catalog_id()),
            Some("$event:matrix.test".to_owned()),
            None,
            true,
        )
        .expect("证据有效"),
        time(1_700_000_000_000),
    )
    .expect("案件有效")
}

fn pending_action() -> ModerationAction {
    ModerationAction::reserve(
        action_id(),
        Some(case_id()),
        principal_id(),
        catalog_id(),
        ModerationActionKind::Hide,
        ModerationTarget::new(ModerationTargetKind::Event, "$event:matrix.test")
            .expect("动作目标有效"),
        ModerationReason::Harassment,
        time(1_700_000_000_000),
        None,
    )
    .expect("动作有效")
}

fn applied_action() -> ModerationAction {
    let mut action = pending_action();
    action.mark_applied().expect("动作可应用");
    action
}

fn reversed_action() -> ModerationAction {
    let mut action = applied_action();
    action.reverse(time(1_700_000_001_000)).expect("动作可撤销");
    action
}

fn audit_event() -> ModerationAuditEvent {
    ModerationAuditEvent::new(
        audit_id(),
        time(1_700_000_000_000),
        principal_id(),
        "moderation.action.applied",
        ModerationTarget::new(ModerationTargetKind::Event, "$event:matrix.test")
            .expect("审计目标有效"),
        ModerationAuditOutcome::Allowed,
        Some(ModerationReason::Harassment),
        audit_id(),
        Some(catalog_id()),
    )
    .expect("审计有效")
}

fn actor() -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: principal_id(),
        matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
        display_name: "Agent Room User".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(1_700_000_000_000),
        expires_at: time(1_700_000_900_000),
        recently_authenticated: true,
    }
}

fn principal_id() -> PrincipalId {
    PrincipalId::from_uuid(uuid(PRINCIPAL_UUID))
}

fn catalog_id() -> RoomCatalogId {
    RoomCatalogId::from_uuid(uuid(CATALOG_UUID))
}

fn case_id() -> ModerationCaseId {
    ModerationCaseId::from_uuid(uuid(CASE_UUID))
}

fn action_id() -> ModerationActionId {
    ModerationActionId::from_uuid(uuid(ACTION_UUID))
}

fn audit_id() -> AuditEventId {
    AuditEventId::from_uuid(uuid(AUDIT_UUID))
}

fn uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).expect("测试 UUID 有效")
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).expect("测试时间有效")
}
