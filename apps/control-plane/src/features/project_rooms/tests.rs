use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    ports::{PortFuture, RepositoryBindingRecord, RepositoryCheckRecord, SecretValue},
    private_rooms::PrivateRoomActor,
    project_rooms::{
        BindingCheckResult, BindingCheckView, BindingView, ClaimRepository, ClaimStatus, ClaimView,
        InspectClaim, InspectRoomRepository, ProjectRoomFailure, ProjectRoomFailureKind,
        ProjectRoomResult, ProjectRoomUseCases,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RepositoryClaimId, RoomCatalogId},
    project_rooms::{ClaimCheckResult, GithubRepository, RepositoryFile},
    room_listing::RoomShareCode,
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

use super::{ProjectRoomHttpDependencies, ProjectRoomHttpState, router};

mod device;

const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const CLAIM_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e49";
const FRONTEND_ORIGIN: &str = "https://app.agent-room.test";

/// 记下每次调用；`failure` 设上时下一次调用回答它。认领默认还在等，`bound` 设上时回认上了。
#[derive(Default)]
struct FakeProjectRooms {
    claims: Mutex<Vec<ClaimRepository>>,
    inspected: Mutex<Vec<(&'static str, InspectClaim)>>,
    rooms: Mutex<Vec<(&'static str, InspectRoomRepository)>>,
    failure: Mutex<Option<ProjectRoomFailure>>,
    bound: Mutex<bool>,
    unbound_room: Mutex<bool>,
}

impl FakeProjectRooms {
    fn failing(kind: ProjectRoomFailureKind) -> Arc<Self> {
        let rooms = Arc::new(Self::default());
        *rooms.failure.lock().unwrap() = Some(ProjectRoomFailure::new("test", kind));
        rooms
    }

    fn answer<T: Send + 'static>(
        &self,
        value: impl FnOnce() -> T + Send + 'static,
    ) -> PortFuture<'_, ProjectRoomResult<T>> {
        let failure = self.failure.lock().unwrap().take();
        Box::pin(async move { failure.map_or_else(|| Ok(value()), Err) })
    }

    fn claim_view(&self) -> ClaimView {
        if *self.bound.lock().unwrap() {
            ClaimView {
                claim_id: None,
                repository: repository(),
                share_code: share_code(),
                status: ClaimStatus::Bound,
                catalog_id: Some(catalog()),
                claimed_at: None,
                last_check: None,
            }
        } else {
            ClaimView {
                claim_id: Some(claim()),
                repository: repository(),
                share_code: share_code(),
                status: ClaimStatus::Pending,
                catalog_id: None,
                claimed_at: Some(time(1_000)),
                last_check: Some(RepositoryCheckRecord {
                    at: time(2_000),
                    result: ClaimCheckResult::NotInRepository,
                }),
            }
        }
    }
}

impl ProjectRoomUseCases for FakeProjectRooms {
    fn claim(&self, request: ClaimRepository) -> PortFuture<'_, ProjectRoomResult<ClaimView>> {
        self.claims.lock().unwrap().push(request);
        let view = self.claim_view();
        self.answer(move || view)
    }

    fn inspect_claim(&self, request: InspectClaim) -> PortFuture<'_, ProjectRoomResult<ClaimView>> {
        self.inspected.lock().unwrap().push(("inspect", request));
        let view = self.claim_view();
        self.answer(move || view)
    }

    fn check_claim(&self, request: InspectClaim) -> PortFuture<'_, ProjectRoomResult<ClaimView>> {
        self.inspected.lock().unwrap().push(("check", request));
        let view = self.claim_view();
        self.answer(move || view)
    }

    fn cancel_claim(&self, request: InspectClaim) -> PortFuture<'_, ProjectRoomResult<()>> {
        self.inspected.lock().unwrap().push(("cancel", request));
        self.answer(|| ())
    }

    fn binding(
        &self,
        request: InspectRoomRepository,
    ) -> PortFuture<'_, ProjectRoomResult<Option<BindingView>>> {
        self.rooms.lock().unwrap().push(("binding", request));
        let unbound = *self.unbound_room.lock().unwrap();
        self.answer(move || (!unbound).then(binding_view))
    }

    fn unbind(&self, request: InspectRoomRepository) -> PortFuture<'_, ProjectRoomResult<()>> {
        self.rooms.lock().unwrap().push(("unbind", request));
        self.answer(|| ())
    }

    fn check_binding(
        &self,
        request: InspectRoomRepository,
    ) -> PortFuture<'_, ProjectRoomResult<BindingCheckView>> {
        self.rooms.lock().unwrap().push(("check", request));
        self.answer(|| BindingCheckView {
            binding: binding_view(),
            checked_at: time(9_000),
            result: BindingCheckResult::Found(RepositoryFile::ReadmeMd),
        })
    }
}

#[tokio::test]
async fn 认领一个仓库_回答带着那一行和徽章() {
    let rooms = Arc::new(FakeProjectRooms::default());
    let response = test_router(rooms.clone())
        .oneshot(claim_request(
            &json!({ "repository": "octo-org/hello.world" }),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response_json(response).await,
        json!({
            "claimId": CLAIM_UUID,
            "repository": "octo-org/hello.world",
            "shareCode": "aB3xYz",
            "shareUrl": "https://agent-room.test/r/aB3xYz",
            "agentsLine": "Agent Room: this project's room is https://agent-room.test/r/aB3xYz. Agents join with `agent-room join https://agent-room.test/r/aB3xYz`, or over the network via https://agent-room.test/agents.txt",
            "badgeMarkdown": "[![Agent Room](https://agent-room.test/gh/octo-org/hello.world/badge.svg)](https://agent-room.test/r/aB3xYz)",
            "status": "pending",
            "catalogId": null,
            "claimedAtUnixMs": 1_000,
            "lastCheck": { "atUnixMs": 2_000, "result": "not_in_repository" },
        })
    );
    let claims = rooms.claims.lock().unwrap();
    assert_eq!(
        claims[0],
        ClaimRepository {
            actor: PrivateRoomActor::Person(actor()),
            repository: "octo-org/hello.world".to_owned(),
            target: None,
        }
    );
}

#[tokio::test]
async fn 拿已有的房间认领_带上房间号() {
    let rooms = Arc::new(FakeProjectRooms::default());
    let response = test_router(rooms.clone())
        .oneshot(claim_request(
            &json!({ "repository": "octo/hello", "catalogId": CATALOG_UUID }),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(rooms.claims.lock().unwrap()[0].target, Some(catalog()));
}

#[tokio::test]
async fn 认领要来自自己的网页_正文要对得上() {
    let rooms = Arc::new(FakeProjectRooms::default());
    let app = test_router(rooms.clone());

    let refused = app
        .clone()
        .oneshot(claim_request(&json!({ "repository": "octo/hello" }), false))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(refused).await["code"],
        "project_room.invalid_origin"
    );

    for bad in [
        json!({}),
        json!({ "repository": 1 }),
        json!({ "repository": "octo/hello", "extra": true }),
        json!({ "repository": "octo/hello", "catalogId": "not-a-uuid" }),
        json!({ "repository": "octo/hello", "catalogId": "0198b601-77a1-4bb8-83eb-a8fe68c97e46" }),
    ] {
        let response = app
            .clone()
            .oneshot(claim_request(&bad, true))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(
            response_json(response).await["code"],
            "project_room.invalid_request"
        );
    }
    assert!(rooms.claims.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 看认领_现在核对_撤掉() {
    let rooms = Arc::new(FakeProjectRooms::default());
    let app = test_router(rooms.clone());

    let response = app
        .clone()
        .oneshot(request(
            Method::GET,
            &format!("/project-rooms/claims/{CLAIM_UUID}"),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await["status"], "pending");

    *rooms.bound.lock().unwrap() = true;
    let refused = app
        .clone()
        .oneshot(request(
            Method::POST,
            &format!("/project-rooms/claims/{CLAIM_UUID}/check"),
            false,
        ))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let response = app
        .clone()
        .oneshot(request(
            Method::POST,
            &format!("/project-rooms/claims/{CLAIM_UUID}/check"),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["status"], "bound");
    assert_eq!(body["claimId"], Value::Null, "认上以后认领就没了");
    assert_eq!(body["catalogId"], CATALOG_UUID);
    assert_eq!(body["shareUrl"], "https://agent-room.test/r/aB3xYz");

    let response = app
        .clone()
        .oneshot(request(
            Method::DELETE,
            &format!("/project-rooms/claims/{CLAIM_UUID}"),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    {
        let inspected = rooms.inspected.lock().unwrap();
        let calls: Vec<_> = inspected.iter().map(|(call, _)| *call).collect();
        assert_eq!(calls, ["inspect", "check", "cancel"]);
        assert!(
            inspected
                .iter()
                .all(|(_, request)| request.claim_id == claim()
                    && request.actor == PrivateRoomActor::Person(actor()))
        );
    }

    let response = app
        .oneshot(request(Method::GET, "/project-rooms/claims/abc", false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn 核对太快回_429_带着等多久() {
    let rooms = Arc::new(FakeProjectRooms::default());
    let retry_at = UtcMillis::new(now_millis() + 30_000).unwrap();
    *rooms.failure.lock().unwrap() = Some(
        ProjectRoomFailure::new("test", ProjectRoomFailureKind::CheckTooSoon).retrying_at(retry_at),
    );
    let response = test_router(rooms)
        .oneshot(request(
            Method::POST,
            &format!("/project-rooms/claims/{CLAIM_UUID}/check"),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after: u64 = response.headers()[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((29..=31).contains(&retry_after), "{retry_after}");
    assert_eq!(
        response_json(response).await["code"],
        "project_room.check_too_soon"
    );
}

#[tokio::test]
async fn 房主看房间绑的仓库_没绑是_null() {
    let rooms = Arc::new(FakeProjectRooms::default());
    let app = test_router(rooms.clone());
    let path = format!("/private-rooms/{CATALOG_UUID}/repository");

    let response = app
        .clone()
        .oneshot(request(Method::GET, &path, false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response_json(response).await, binding_json());

    *rooms.unbound_room.lock().unwrap() = true;
    let response = app
        .oneshot(request(Method::GET, &path, false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await, Value::Null);
    let calls = rooms.rooms.lock().unwrap();
    assert_eq!(calls[0].1.catalog_id, catalog());
}

#[tokio::test]
async fn 房主再核对一次_回答带着这次的结果() {
    let rooms = Arc::new(FakeProjectRooms::default());
    let response = test_router(rooms.clone())
        .oneshot(request(
            Method::POST,
            &format!("/private-rooms/{CATALOG_UUID}/repository/check"),
            true,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await,
        json!({
            "binding": binding_json(),
            "lastCheck": { "atUnixMs": 9_000, "result": "found" },
        })
    );
    assert_eq!(rooms.rooms.lock().unwrap()[0].0, "check");
}

#[tokio::test]
async fn 解绑要来自自己的网页_另有一间不绑仓库的公开房间时回_409() {
    let path = format!("/private-rooms/{CATALOG_UUID}/repository");
    let rooms = Arc::new(FakeProjectRooms::default());
    let app = test_router(rooms.clone());
    let refused = app
        .clone()
        .oneshot(request(Method::DELETE, &path, false))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert!(rooms.rooms.lock().unwrap().is_empty());
    let response = app
        .oneshot(request(Method::DELETE, &path, true))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let limited = FakeProjectRooms::failing(ProjectRoomFailureKind::LimitReached);
    let response = test_router(limited)
        .oneshot(request(Method::DELETE, &path, true))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await["code"],
        "private_room.public_limit_reached"
    );
}

#[tokio::test]
async fn 失败按种类回答() {
    for (kind, status, code) in [
        (
            ProjectRoomFailureKind::InvalidRepository,
            StatusCode::BAD_REQUEST,
            "project_room.invalid_repository",
        ),
        (
            ProjectRoomFailureKind::Forbidden,
            StatusCode::FORBIDDEN,
            "project_room.forbidden",
        ),
        (
            ProjectRoomFailureKind::NotFound,
            StatusCode::NOT_FOUND,
            "project_room.not_found",
        ),
        (
            ProjectRoomFailureKind::TooManyClaims,
            StatusCode::CONFLICT,
            "project_room.too_many_claims",
        ),
        (
            ProjectRoomFailureKind::RepositoryTaken,
            StatusCode::CONFLICT,
            "project_room.repository_taken",
        ),
        (
            ProjectRoomFailureKind::RoomNotEligible,
            StatusCode::CONFLICT,
            "project_room.room_not_eligible",
        ),
        (
            ProjectRoomFailureKind::RoomAlreadyBound,
            StatusCode::CONFLICT,
            "room_listing.repository_bound",
        ),
        (
            ProjectRoomFailureKind::Conflict,
            StatusCode::CONFLICT,
            "project_room.conflict",
        ),
        (
            ProjectRoomFailureKind::DependencyUnavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            "project_room.dependency_unavailable",
        ),
        (
            ProjectRoomFailureKind::Internal,
            StatusCode::INTERNAL_SERVER_ERROR,
            "project_room.internal",
        ),
    ] {
        let response = test_router(FakeProjectRooms::failing(kind))
            .oneshot(claim_request(&json!({ "repository": "octo/hello" }), true))
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{kind:?}");
        assert_eq!(response_json(response).await["code"], code, "{kind:?}");
    }
}

fn binding_view() -> BindingView {
    BindingView {
        binding: RepositoryBindingRecord {
            catalog_id: catalog(),
            repository: repository(),
            verified_by: actor().principal_id,
            verified_at: time(5_000),
            verified_file: RepositoryFile::AgentsMd,
        },
        share_code: Some(share_code()),
    }
}

fn binding_json() -> Value {
    json!({
        "repository": "octo-org/hello.world",
        "repositoryUrl": "https://github.com/octo-org/hello.world",
        "verifiedByAccountId": PRINCIPAL_UUID,
        "verifiedAtUnixMs": 5_000,
        "verifiedFile": "AGENTS.md",
        "shareCode": "aB3xYz",
        "shareUrl": "https://agent-room.test/r/aB3xYz",
        "agentsLine": "Agent Room: this project's room is https://agent-room.test/r/aB3xYz. Agents join with `agent-room join https://agent-room.test/r/aB3xYz`, or over the network via https://agent-room.test/agents.txt",
        "badgeMarkdown": "[![Agent Room](https://agent-room.test/gh/octo-org/hello.world/badge.svg)](https://agent-room.test/r/aB3xYz)",
    })
}

fn test_router(rooms: Arc<FakeProjectRooms>) -> Router {
    router_with_devices(rooms, Arc::default())
}

fn router_with_devices(rooms: Arc<FakeProjectRooms>, devices: Arc<device::FakeDevices>) -> Router {
    router(ProjectRoomHttpState::new(
        ProjectRoomHttpDependencies {
            rooms,
            authentication: Arc::new(FakeAuthentication),
            devices,
            secrets: Arc::new(agent_room_identity_adapter::SecureSecretFactory),
        },
        &Url::parse(FRONTEND_ORIGIN).unwrap(),
        &crate::config::DesktopOrigins::for_tests(),
        "agent-room.test",
    ))
    .layer(middleware::from_fn(crate::correlation::attach))
}

fn claim_request(body: &Value, origin: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/project-rooms/claims")
        .header(header::COOKIE, "__Host-agent-room-session=session-secret")
        .header(header::CONTENT_TYPE, "application/json");
    if origin {
        request = request.header(header::ORIGIN, FRONTEND_ORIGIN);
    }
    request.body(Body::from(body.to_string())).unwrap()
}

fn request(method: Method, path: &str, origin: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::COOKIE, "__Host-agent-room-session=session-secret");
    if origin {
        request = request.header(header::ORIGIN, FRONTEND_ORIGIN);
    }
    request.body(Body::empty()).unwrap()
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

fn repository() -> GithubRepository {
    GithubRepository::parse("octo-org/hello.world").unwrap()
}

fn share_code() -> RoomShareCode {
    RoomShareCode::parse("aB3xYz").unwrap()
}

fn catalog() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::parse_str(CATALOG_UUID).unwrap())
}

fn claim() -> RepositoryClaimId {
    RepositoryClaimId::from_uuid(Uuid::parse_str(CLAIM_UUID).unwrap())
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}

fn now_millis() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}
