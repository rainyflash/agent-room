use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    ports::{PortFuture, SecretValue},
    private_rooms::{
        ChangeShareCode, InspectListing, ListingFailure, ListingFailureKind, ListingResult,
        ListingView, PlatformHoldNotice, PrivateRoomListingUseCases, UpdateListing,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::PrivateRoomPermissions,
    room_listing::{RoomJoinMode, RoomListingSettings, RoomShareCode},
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

use super::{PrivateRoomListingHttpState, router};

const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";

/// 回答公开着、要申请的设置；`failure` 设上时下一次调用回答它。
#[derive(Default)]
struct FakeListings {
    inspected: Mutex<Vec<InspectListing>>,
    updated: Mutex<Vec<UpdateListing>>,
    changed: Mutex<Vec<ChangeShareCode>>,
    failure: Mutex<Option<ListingFailure>>,
}

impl FakeListings {
    fn answer(&self) -> PortFuture<'_, ListingResult<ListingView>> {
        let failure = self.failure.lock().unwrap().take();
        Box::pin(async move { failure.map_or_else(|| Ok(view()), Err) })
    }
}

impl PrivateRoomListingUseCases for FakeListings {
    fn inspect(&self, request: InspectListing) -> PortFuture<'_, ListingResult<ListingView>> {
        self.inspected.lock().unwrap().push(request);
        self.answer()
    }

    fn update(&self, request: UpdateListing) -> PortFuture<'_, ListingResult<ListingView>> {
        self.updated.lock().unwrap().push(request);
        self.answer()
    }

    fn change_share_code(
        &self,
        request: ChangeShareCode,
    ) -> PortFuture<'_, ListingResult<ListingView>> {
        self.changed.lock().unwrap().push(request);
        self.answer()
    }
}

#[tokio::test]
async fn 读设置回答公开着的样子_分享码和加不加密都在() {
    let listings = Arc::new(FakeListings::default());
    let response = test_router(listings.clone())
        .oneshot(request(Method::GET, None, false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "no-store",
        "设置里有分享码，别缓存"
    );
    assert_eq!(
        response_json(response).await,
        json!({
            "listed": true,
            "joinMode": "request",
            "memberPermissions": { "capabilities": ["view", "speak", "automate"] },
            "allowNetworkAgents": false,
            "shareCode": "aB3xYz",
            "shareUrl": "https://agent-room.test/r/aB3xYz",
            "firstListedAtUnixMs": 1_000,
            "endToEndEncrypted": false,
            "platformHold": { "reason": "冒充官方客服", "heldAtUnixMs": 2_000 },
        })
    );
    let inspected = listings.inspected.lock().unwrap();
    assert_eq!(inspected[0].actor, actor());
    assert_eq!(inspected[0].catalog_id, catalog());
}

#[tokio::test]
async fn 改设置要来自自己的网页_正文要对得上() {
    let listings = Arc::new(FakeListings::default());
    let app = test_router(listings.clone());
    let body = json!({
        "listed": true,
        "joinMode": "invite_only",
        "memberPermissions": { "capabilities": ["view", "speak"] },
        "allowNetworkAgents": true,
    });

    let response = app
        .clone()
        .oneshot(request(Method::PUT, Some(body.clone()), true))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    {
        let updated = listings.updated.lock().unwrap();
        assert_eq!(updated.len(), 1);
        let settings = updated[0].settings;
        assert!(settings.listed());
        assert_eq!(settings.join_mode(), RoomJoinMode::InviteOnly);
        assert!(settings.member_permissions().speak());
        assert!(!settings.member_permissions().automate());
        assert!(settings.allow_network_agents());
    }

    let response = app
        .clone()
        .oneshot(request(Method::PUT, Some(body), false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(response).await["code"],
        "room_listing.invalid_origin"
    );

    for bad in [
        json!({}),
        json!({ "listed": true, "joinMode": "approval", "memberPermissions": { "capabilities": ["view"] }, "allowNetworkAgents": true }),
        json!({ "listed": true, "joinMode": "open", "memberPermissions": { "capabilities": ["view", "manage"] }, "allowNetworkAgents": true }),
        json!({ "listed": true, "joinMode": "open", "memberPermissions": { "capabilities": ["automate"] }, "allowNetworkAgents": true }),
        json!({ "listed": true, "joinMode": "open", "memberPermissions": { "capabilities": ["view"] }, "allowNetworkAgents": true, "extra": 1 }),
    ] {
        let response = app
            .clone()
            .oneshot(request(Method::PUT, Some(bad.clone()), true))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(
            response_json(response).await["code"],
            "room_listing.invalid"
        );
    }
    assert_eq!(listings.updated.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn 换分享码要来自自己的网页_回换好以后的设置() {
    let listings = Arc::new(FakeListings::default());
    let app = test_router(listings.clone());

    let refused = app
        .clone()
        .oneshot(share_code_request(false))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(refused).await["code"],
        "room_listing.invalid_origin"
    );
    assert!(listings.changed.lock().unwrap().is_empty());

    let response = app.oneshot(share_code_request(true)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response_json(response).await["shareCode"], "aB3xYz");
    {
        let changed = listings.changed.lock().unwrap();
        assert_eq!(changed[0].actor, actor());
        assert_eq!(changed[0].catalog_id, catalog());
    }

    let never = Arc::new(FakeListings::default());
    *never.failure.lock().unwrap() =
        Some(ListingFailure::new("test", ListingFailureKind::NeverListed));
    let response = test_router(never)
        .oneshot(share_code_request(true))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await["code"],
        "room_listing.never_listed"
    );
}

fn share_code_request(origin: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri(format!("/private-rooms/{CATALOG_UUID}/listing/share-code"))
        .header(header::COOKIE, "__Host-agent-room-session=session-secret");
    if origin {
        request = request.header(header::ORIGIN, FRONTEND_ORIGIN);
    }
    request.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn 失败按种类回答() {
    for (kind, status, code) in [
        (
            ListingFailureKind::Forbidden,
            StatusCode::FORBIDDEN,
            "room_listing.forbidden",
        ),
        (
            ListingFailureKind::NotFound,
            StatusCode::NOT_FOUND,
            "room_listing.not_found",
        ),
        (
            ListingFailureKind::LimitReached,
            StatusCode::CONFLICT,
            "room_listing.limit_reached",
        ),
        (
            ListingFailureKind::DependencyUnavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            "room_listing.dependency_unavailable",
        ),
    ] {
        let listings = Arc::new(FakeListings::default());
        *listings.failure.lock().unwrap() = Some(ListingFailure::new("test", kind));
        let response = test_router(listings)
            .oneshot(request(Method::GET, None, false))
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
}

fn view() -> ListingView {
    ListingView {
        settings: RoomListingSettings::new(
            true,
            RoomJoinMode::Request,
            PrivateRoomPermissions::INVITED_BY_AGENT,
            false,
        )
        .unwrap(),
        share_code: Some(RoomShareCode::parse("aB3xYz").unwrap()),
        first_listed_at: Some(time(1_000)),
        end_to_end_encrypted: false,
        platform_hold: Some(PlatformHoldNotice {
            reason: "冒充官方客服".to_owned(),
            held_at: time(2_000),
        }),
    }
}

fn test_router(listings: Arc<FakeListings>) -> Router {
    router(PrivateRoomListingHttpState::new(
        listings,
        Arc::new(FakeAuthentication),
        &Url::parse(FRONTEND_ORIGIN).unwrap(),
        &crate::config::DesktopOrigins::for_tests(),
        "agent-room.test",
    ))
    .layer(middleware::from_fn(crate::correlation::attach))
}

fn request(method: Method, body: Option<Value>, origin: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(format!("/private-rooms/{CATALOG_UUID}/listing"))
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
        principal_id: PrincipalId::from_uuid(Uuid::parse_str(PRINCIPAL_UUID).unwrap()),
        matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
        display_name: "Agent Room User".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(1_700_000_000_000),
        expires_at: time(1_700_000_900_000),
        recently_authenticated: false,
    }
}

fn catalog() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::parse_str(CATALOG_UUID).unwrap())
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}
