use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    ports::{ActiveRoomModerationGrant, PortFuture, RoomModerationGrant, SecretValue},
    private_rooms::{
        GrantRoomModeration, ListRoomModerationGrants, ModerationGrantFailure,
        ModerationGrantFailureKind, ModerationGrantResult, RevokeRoomModerationGrant,
        RoomModerationGrantUseCases,
    },
};
use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId, RoomModerationGrantId},
    room_moderation_grants::{RoomModerationPower, RoomModerationPowers, RoomModerationRules},
    time::UtcMillis,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
    middleware,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use url::Url;
use uuid::Uuid;

use super::{RoomModerationGrantHttpState, router};

const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const AGENT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e48";
const GRANT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e4a";
const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";
const CREATED_AT: i64 = 1_700_000_100_000;

/// 记下收到的请求；`failure` 设上时下一次调用回答它。
#[derive(Default)]
struct FakeGrants {
    granted: Mutex<Vec<GrantRoomModeration>>,
    listed: Mutex<Vec<ListRoomModerationGrants>>,
    revoked: Mutex<Vec<RevokeRoomModerationGrant>>,
    failure: Mutex<Option<ModerationGrantFailure>>,
}

impl FakeGrants {
    fn answer<T>(&self, value: T) -> ModerationGrantResult<T> {
        self.failure.lock().unwrap().take().map_or(Ok(value), Err)
    }
}

impl RoomModerationGrantUseCases for FakeGrants {
    fn grant(
        &self,
        request: GrantRoomModeration,
    ) -> PortFuture<'_, ModerationGrantResult<ActiveRoomModerationGrant>> {
        let answer = self.answer(active_grant(request.powers.clone()));
        self.granted.lock().unwrap().push(request);
        Box::pin(async move { answer })
    }

    fn list(
        &self,
        request: ListRoomModerationGrants,
    ) -> PortFuture<'_, ModerationGrantResult<Vec<ActiveRoomModerationGrant>>> {
        let answer = self.answer(vec![active_grant(vec![RoomModerationPower::Mute])]);
        self.listed.lock().unwrap().push(request);
        Box::pin(async move { answer })
    }

    fn revoke(
        &self,
        request: RevokeRoomModerationGrant,
    ) -> PortFuture<'_, ModerationGrantResult<()>> {
        let answer = self.answer(());
        self.revoked.lock().unwrap().push(request);
        Box::pin(async move { answer })
    }
}

fn active_grant(powers: Vec<RoomModerationPower>) -> ActiveRoomModerationGrant {
    ActiveRoomModerationGrant {
        grant: RoomModerationGrant {
            id: RoomModerationGrantId::from_uuid(Uuid::parse_str(GRANT_UUID).unwrap()),
            catalog_id: catalog(),
            agent_id: agent(),
            granted_by: actor().principal_id,
            rules: RoomModerationRules::parse("说 spam 的禁言 10 分钟。").unwrap(),
            powers: RoomModerationPowers::new(powers).unwrap(),
            created_at: time(CREATED_AT),
            expires_at: time(CREATED_AT + 3_600_000),
            revoked_at: None,
        },
        agent_display_name: "看门的 #2316".to_owned(),
    }
}

#[tokio::test]
async fn 给一份授权_要最近登录过_回_201_和这一份() {
    let grants = Arc::new(FakeGrants::default());
    let app = test_router(
        grants.clone(),
        AuthenticationRequirement::RecentAuthentication,
    );
    let body = json!({
        "agentId": AGENT_UUID,
        "rules": "说 spam 的禁言 10 分钟。",
        "actions": ["ban", "mute"],
        "lifetimeSeconds": 3_600,
    });
    let response = app
        .oneshot(request(Method::POST, &grants_path(), Some(body), true))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response_json(response).await,
        json!({
            "id": GRANT_UUID,
            "agentId": AGENT_UUID,
            "agentName": "看门的 #2316",
            "grantedBy": PRINCIPAL_UUID,
            "rules": "说 spam 的禁言 10 分钟。",
            "actions": ["mute", "ban"],
            "createdAtUnixMs": CREATED_AT,
            "expiresAtUnixMs": CREATED_AT + 3_600_000,
        })
    );
    let granted = grants.granted.lock().unwrap();
    assert_eq!(granted[0].catalog_id, catalog());
    assert_eq!(granted[0].agent_id, agent());
    assert_eq!(
        granted[0].powers,
        [RoomModerationPower::Ban, RoomModerationPower::Mute]
    );
    assert_eq!(granted[0].lifetime.unwrap().value(), 3_600_000);
    assert_eq!(granted[0].rules, "说 spam 的禁言 10 分钟。");
}

#[tokio::test]
async fn 不填时长交给用例层按三十天算() {
    let grants = Arc::new(FakeGrants::default());
    let app = test_router(
        grants.clone(),
        AuthenticationRequirement::RecentAuthentication,
    );
    let body = json!({"agentId": AGENT_UUID, "rules": "规则", "actions": ["kick"]});
    let response = app
        .oneshot(request(Method::POST, &grants_path(), Some(body), true))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(grants.granted.lock().unwrap()[0].lifetime, None);
}

#[tokio::test]
async fn 正文认不出_来源不对的不给() {
    let grants = Arc::new(FakeGrants::default());
    let app = test_router(
        grants.clone(),
        AuthenticationRequirement::RecentAuthentication,
    );
    let valid = json!({"agentId": AGENT_UUID, "rules": "规则", "actions": ["mute"]});
    for (body, origin, status, code) in [
        (
            json!({"agentId": AGENT_UUID, "rules": "规则", "actions": ["hide"]}),
            true,
            StatusCode::BAD_REQUEST,
            "room_moderation_grant.invalid",
        ),
        (
            json!({"agentId": "agent", "rules": "规则", "actions": ["mute"]}),
            true,
            StatusCode::BAD_REQUEST,
            "room_moderation_grant.invalid",
        ),
        (
            json!({"agentId": AGENT_UUID, "rules": "规则", "actions": ["mute"], "lifetimeSeconds": 0}),
            true,
            StatusCode::BAD_REQUEST,
            "room_moderation_grant.invalid",
        ),
        (
            json!({"agentId": AGENT_UUID, "rules": "规则", "actions": ["mute"], "maxActions": 3}),
            true,
            StatusCode::BAD_REQUEST,
            "room_moderation_grant.invalid",
        ),
        (
            valid,
            false,
            StatusCode::FORBIDDEN,
            "room_moderation_grant.invalid_origin",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(request(Method::POST, &grants_path(), Some(body), origin))
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
    assert!(grants.granted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 列出在用的授权() {
    let grants = Arc::new(FakeGrants::default());
    let app = test_router(grants.clone(), AuthenticationRequirement::ActiveSession);
    let response = app
        .oneshot(request(Method::GET, &grants_path(), None, false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["grants"][0]["id"], GRANT_UUID);
    assert_eq!(body["grants"][0]["actions"], json!(["mute"]));
    assert_eq!(grants.listed.lock().unwrap()[0].catalog_id, catalog());
}

#[tokio::test]
async fn 撤掉一份回_204_来源不对的不撤() {
    let grants = Arc::new(FakeGrants::default());
    let app = test_router(grants.clone(), AuthenticationRequirement::ActiveSession);
    let path = format!("{}/{GRANT_UUID}", grants_path());
    let response = app
        .clone()
        .oneshot(request(Method::DELETE, &path, None, false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(grants.revoked.lock().unwrap().is_empty());

    let response = app
        .oneshot(request(Method::DELETE, &path, None, true))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let revoked = grants.revoked.lock().unwrap();
    assert_eq!(revoked[0].catalog_id, catalog());
    assert_eq!(revoked[0].grant_id.to_string(), GRANT_UUID);
}

#[tokio::test]
async fn 失败照种类回答() {
    let grants = Arc::new(FakeGrants::default());
    let app = test_router(grants.clone(), AuthenticationRequirement::ActiveSession);
    for (kind, status, code) in [
        (
            ModerationGrantFailureKind::InvalidRequest,
            StatusCode::BAD_REQUEST,
            "room_moderation_grant.invalid",
        ),
        (
            ModerationGrantFailureKind::Forbidden,
            StatusCode::FORBIDDEN,
            "room_moderation_grant.forbidden",
        ),
        (
            ModerationGrantFailureKind::NotYourAgent,
            StatusCode::FORBIDDEN,
            "room_moderation_grant.not_your_agent",
        ),
        (
            ModerationGrantFailureKind::NotFound,
            StatusCode::NOT_FOUND,
            "room_moderation_grant.not_found",
        ),
        (
            ModerationGrantFailureKind::Conflict,
            StatusCode::CONFLICT,
            "room_moderation_grant.conflict",
        ),
        (
            ModerationGrantFailureKind::DependencyUnavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            "room_moderation_grant.dependency_unavailable",
        ),
        (
            ModerationGrantFailureKind::Internal,
            StatusCode::INTERNAL_SERVER_ERROR,
            "room_moderation_grant.internal",
        ),
    ] {
        *grants.failure.lock().unwrap() = Some(ModerationGrantFailure::new("test", kind));
        let response = app
            .clone()
            .oneshot(request(Method::GET, &grants_path(), None, false))
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
}

fn test_router(grants: Arc<FakeGrants>, requirement: AuthenticationRequirement) -> Router {
    router(RoomModerationGrantHttpState::new(
        grants,
        Arc::new(FakeAuthentication { requirement }),
        &Url::parse(FRONTEND_ORIGIN).unwrap(),
        &crate::config::DesktopOrigins::for_tests(),
    ))
    .layer(middleware::from_fn(crate::correlation::attach))
}

fn grants_path() -> String {
    format!("/private-rooms/{CATALOG_UUID}/moderation-grants")
}

fn request(method: Method, path: &str, body: Option<Value>, origin: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::COOKIE, "__Host-agent-room-session=session-secret");
    if origin {
        request = request.header(header::ORIGIN, FRONTEND_ORIGIN);
    }
    match body {
        Some(body) => request
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
        None => request.body(Body::empty()).unwrap(),
    }
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 64 * 1_024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

/// 只认这一种登录要求：给授权要最近登录过，看和撤只要登录着。
struct FakeAuthentication {
    requirement: AuthenticationRequirement,
}

impl AuthenticationUseCases for FakeAuthentication {
    fn begin_login(
        &self,
        _request: BeginLogin,
    ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>> {
        Box::pin(async { unreachable!("不开始登录") })
    }

    fn complete_login<'a>(
        &'a self,
        _request: CompleteLogin<'a>,
    ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
        Box::pin(async { unreachable!("不完成登录") })
    }

    fn authenticate<'a>(
        &'a self,
        session_secret: &'a SecretValue,
        requirement: AuthenticationRequirement,
    ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>> {
        assert_eq!(session_secret.expose(), "session-secret");
        assert_eq!(requirement, self.requirement);
        Box::pin(async { Ok(actor()) })
    }

    fn logout<'a>(
        &'a self,
        _session_secret: &'a SecretValue,
    ) -> PortFuture<'a, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("不退出登录") })
    }

    fn suspend_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("不停用账户") })
    }
}

fn actor() -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: PrincipalId::from_uuid(Uuid::parse_str(PRINCIPAL_UUID).unwrap()),
        matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
        display_name: "Agent Room User".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(1_700_000_000_000),
        expires_at: time(1_700_000_900_000),
        recently_authenticated: true,
    }
}

fn catalog() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::parse_str(CATALOG_UUID).unwrap())
}

fn agent() -> AgentId {
    AgentId::from_uuid(Uuid::parse_str(AGENT_UUID).unwrap())
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}
