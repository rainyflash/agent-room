use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    ports::{PortFuture, PrivateRoomInviteLinkRecord, SecretValue},
    private_rooms::{
        CreateInviteLink, InviteLink, InviteLinkFailure, InviteLinkFailureKind, InviteLinkResult,
        InvitePreview, InviteeMembership, ManageInviteLink, PrivateRoomInviteLinkUseCases,
        RedeemedInvite, UseInviteLink,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::PrivateRoomPermissions,
    room_invites::PrivateRoomInviteSecret,
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

use super::{PrivateRoomInviteHttpState, router};

const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";

/// 记下每次调用；`failure` 设上时下一次调用回答它。
#[derive(Default)]
struct FakeLinks {
    calls: Mutex<Vec<&'static str>>,
    created: Mutex<Vec<CreateInviteLink>>,
    used: Mutex<Vec<UseInviteLink>>,
    failure: Mutex<Option<InviteLinkFailure>>,
}

impl FakeLinks {
    fn answer<T: Send + 'static>(
        &self,
        call: &'static str,
        value: T,
    ) -> PortFuture<'_, InviteLinkResult<T>> {
        self.calls.lock().unwrap().push(call);
        let failure = self.failure.lock().unwrap().take();
        Box::pin(async move { failure.map_or(Ok(value), Err) })
    }
}

impl PrivateRoomInviteLinkUseCases for FakeLinks {
    fn create(&self, request: CreateInviteLink) -> PortFuture<'_, InviteLinkResult<InviteLink>> {
        let permissions = request
            .permissions
            .unwrap_or(PrivateRoomPermissions::INVITED_BY_AGENT);
        self.created.lock().unwrap().push(request);
        self.answer("create", link(permissions))
    }

    fn reveal(&self, request: ManageInviteLink) -> PortFuture<'_, InviteLinkResult<InviteLink>> {
        assert_eq!(request.catalog_id, catalog());
        self.answer("reveal", link(PrivateRoomPermissions::INVITED_BY_AGENT))
    }

    fn disable(&self, request: ManageInviteLink) -> PortFuture<'_, InviteLinkResult<()>> {
        assert_eq!(request.actor, actor());
        self.answer("disable", ())
    }

    fn preview(&self, request: UseInviteLink) -> PortFuture<'_, InviteLinkResult<InvitePreview>> {
        self.used.lock().unwrap().push(request);
        self.answer(
            "preview",
            InvitePreview {
                catalog_id: catalog(),
                room_name: "周报".to_owned(),
                inviter_display_name: "小明".to_owned(),
                permissions: PrivateRoomPermissions::INVITED_BY_AGENT,
                expires_at: time(2_000),
                membership: InviteeMembership::Absent,
            },
        )
    }

    fn redeem(&self, request: UseInviteLink) -> PortFuture<'_, InviteLinkResult<RedeemedInvite>> {
        self.used.lock().unwrap().push(request);
        self.answer(
            "redeem",
            RedeemedInvite {
                catalog_id: catalog(),
                membership: InviteeMembership::Invited,
            },
        )
    }
}

#[tokio::test]
async fn 生成链接回答能直接发出去的网页地址_正文可以不带() {
    let links = Arc::new(FakeLinks::default());
    let response = test_router(links.clone())
        .oneshot(signed_in(Method::POST, "/invite-link", None, true))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&header::HeaderValue::from_static("no-store"))
    );
    let secret = secret();
    assert_eq!(
        response_json(response).await,
        json!({
            "url": format!("{FRONTEND_ORIGIN}/join#{}", secret.as_str()),
            "secret": secret.as_str(),
            "permissions": { "capabilities": ["view", "speak", "automate"] },
            "createdAtUnixMs": 1_000,
            "expiresAtUnixMs": 2_000,
        })
    );
    let created = links.created.lock().unwrap();
    assert_eq!(created[0].actor, actor());
    assert_eq!(created[0].catalog_id, catalog());
    assert_eq!(created[0].permissions, None);
}

#[tokio::test]
async fn 生成时可以给权限_权限不对或来源不对都不调用() {
    let links = Arc::new(FakeLinks::default());
    let app = test_router(links.clone());
    let response = app
        .clone()
        .oneshot(signed_in(
            Method::POST,
            "/invite-link",
            Some(json!({ "permissions": { "capabilities": ["view"] } })),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await["permissions"],
        json!({ "capabilities": ["view"] })
    );

    for (body, origin, code) in [
        (
            Some(json!({ "permissions": { "capabilities": ["automate"] } })),
            true,
            "invite_link.invalid_permissions",
        ),
        (
            Some(json!({ "permissions": null, "extra": 1 })),
            true,
            "invite_link.invalid_permissions",
        ),
        (None, false, "invite_link.invalid_origin"),
    ] {
        let response = app
            .clone()
            .oneshot(signed_in(Method::POST, "/invite-link", body, origin))
            .await
            .unwrap();
        assert_eq!(response_json(response).await["code"], code);
    }
    assert_eq!(links.created.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn 再看和停用_停用要来源对() {
    let links = Arc::new(FakeLinks::default());
    let app = test_router(links.clone());
    let revealed = app
        .clone()
        .oneshot(signed_in(Method::GET, "/invite-link", None, false))
        .await
        .unwrap();
    assert_eq!(revealed.status(), StatusCode::OK);
    assert_eq!(response_json(revealed).await["secret"], secret().as_str());

    let rejected = app
        .clone()
        .oneshot(signed_in(Method::DELETE, "/invite-link", None, false))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    let disabled = app
        .oneshot(signed_in(Method::DELETE, "/invite-link", None, true))
        .await
        .unwrap();
    assert_eq!(disabled.status(), StatusCode::NO_CONTENT);
    assert_eq!(*links.calls.lock().unwrap(), ["reveal", "disable"]);
}

#[tokio::test]
async fn 拿着链接的人看一眼再进来_秘密只放在正文里() {
    let links = Arc::new(FakeLinks::default());
    let app = test_router(links.clone());
    let body = json!({ "secret": secret().as_str() });
    let preview = app
        .clone()
        .oneshot(using("/private-room-invites/preview", &body))
        .await
        .unwrap();
    assert_eq!(preview.status(), StatusCode::OK);
    assert_eq!(
        response_json(preview).await,
        json!({
            "catalogId": CATALOG_UUID,
            "roomName": "周报",
            "inviterDisplayName": "小明",
            "permissions": { "capabilities": ["view", "speak", "automate"] },
            "expiresAtUnixMs": 2_000,
            "membership": "absent",
        })
    );
    let redeemed = app
        .clone()
        .oneshot(using("/private-room-invites/redeem", &body))
        .await
        .unwrap();
    assert_eq!(
        response_json(redeemed).await,
        json!({ "catalogId": CATALOG_UUID, "membership": "invited" })
    );
    {
        let used = links.used.lock().unwrap();
        assert_eq!(used.len(), 2);
        assert_eq!(used[0].secret, secret().as_str());
        assert_eq!(used[0].actor, actor());
    }

    for bad in [
        json!({}),
        json!({ "secret": 1 }),
        json!({ "secret": "a", "x": 1 }),
    ] {
        let response = app
            .clone()
            .oneshot(using("/private-room-invites/redeem", &bad))
            .await
            .unwrap();
        assert_eq!(response_json(response).await["code"], "invite_link.invalid");
    }
}

#[tokio::test]
async fn 失败按种类回答_猜太多次带上要等多久() {
    for (failure, status, code) in [
        (
            InviteLinkFailure::new("test", InviteLinkFailureKind::NotFound),
            StatusCode::NOT_FOUND,
            "invite_link.not_found",
        ),
        (
            InviteLinkFailure::new("test", InviteLinkFailureKind::Banned),
            StatusCode::FORBIDDEN,
            "invite_link.banned",
        ),
        (
            InviteLinkFailure::new("test", InviteLinkFailureKind::NotRevealable),
            StatusCode::NOT_FOUND,
            "invite_link.not_revealable",
        ),
        (
            InviteLinkFailure::rate_limited("test", time(i64::MAX / 2)),
            StatusCode::TOO_MANY_REQUESTS,
            "invite_link.rate_limited",
        ),
    ] {
        let links = Arc::new(FakeLinks::default());
        *links.failure.lock().unwrap() = Some(failure);
        let response = test_router(links)
            .oneshot(using(
                "/private-room-invites/redeem",
                &json!({ "secret": secret().as_str() }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{code}");
        let rate_limited = response.headers().contains_key(header::RETRY_AFTER);
        assert_eq!(rate_limited, status == StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response_json(response).await["code"], code);
    }
}

fn test_router(links: Arc<FakeLinks>) -> Router {
    router(PrivateRoomInviteHttpState::new(
        links,
        Arc::new(FakeAuthentication),
        &Url::parse(FRONTEND_ORIGIN).unwrap(),
        &crate::config::DesktopOrigins::for_tests(),
    ))
    .layer(middleware::from_fn(crate::correlation::attach))
}

fn signed_in(method: Method, suffix: &str, body: Option<Value>, origin: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(format!("/private-rooms/{CATALOG_UUID}{suffix}"))
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

fn using(path: &str, body: &Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::COOKIE, "__Host-agent-room-session=session-secret")
        .header(header::ORIGIN, FRONTEND_ORIGIN)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 64 * 1_024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn link(permissions: PrivateRoomPermissions) -> InviteLink {
    InviteLink {
        secret: secret(),
        record: PrivateRoomInviteLinkRecord {
            catalog_id: catalog(),
            permissions,
            created_by: principal(),
            created_at: time(1_000),
            expires_at: time(2_000),
        },
    }
}

fn secret() -> PrivateRoomInviteSecret {
    PrivateRoomInviteSecret::from_entropy([0x42; 16])
}

struct FakeAuthentication;

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
        assert_eq!(requirement, AuthenticationRequirement::ActiveSession);
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
        principal_id: principal(),
        matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
        display_name: "Agent Room User".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(1_700_000_000_000),
        expires_at: time(1_700_000_900_000),
        recently_authenticated: false,
    }
}

fn principal() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::parse_str(PRINCIPAL_UUID).unwrap())
}

fn catalog() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::parse_str(CATALOG_UUID).unwrap())
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}
