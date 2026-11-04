use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationFailure, AuthenticationFailureKind,
        AuthenticationRequirement, AuthenticationResult, AuthenticationUseCases, BeginLogin,
        CompleteLogin, LoginCompletion, LoginRedirect,
    },
    ports::{
        Clock, PortFuture, PublicRoomRecord, PublicRoomSort, PublicRoomViewerState, SecretValue,
    },
    public_room_directory::{
        BrowsePublicRooms, PublicDirectoryFailure, PublicDirectoryFailureKind,
        PublicDirectoryResult, PublicRoomDetailView, PublicRoomDirectoryUseCases, PublicRoomOnline,
        PublicRoomPage, PublicRoomView,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    room_listing::{RoomJoinMode, RoomShareCode},
    time::UtcMillis,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderValue, Request, StatusCode, header},
    middleware,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::{PublicRoomsHttpState, router};
use crate::features::anonymous_limits::{AnonymousLimiter, PER_MINUTE};

const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";

/// 记下每次翻目录的请求；登录的人看到自己已加入。
#[derive(Default)]
struct FakeDirectory {
    browsed: Mutex<Vec<BrowsePublicRooms>>,
    details: Mutex<Vec<(Option<PrincipalId>, String)>>,
}

impl PublicRoomDirectoryUseCases for FakeDirectory {
    fn browse(
        &self,
        request: BrowsePublicRooms,
    ) -> PortFuture<'_, PublicDirectoryResult<PublicRoomPage>> {
        let viewer = request
            .viewer
            .as_ref()
            .map(|_| PublicRoomViewerState::Joined);
        self.browsed.lock().unwrap().push(request);
        Box::pin(async move {
            Ok(PublicRoomPage {
                rooms: vec![PublicRoomView {
                    room: room(),
                    viewer,
                }],
                next_offset: Some(24),
            })
        })
    }

    fn detail(
        &self,
        viewer: Option<AuthenticatedPrincipal>,
        share_code: String,
    ) -> PortFuture<'_, PublicDirectoryResult<PublicRoomDetailView>> {
        let found = share_code == "Club01";
        self.details
            .lock()
            .unwrap()
            .push((viewer.map(|viewer| viewer.principal_id), share_code));
        Box::pin(async move {
            if found {
                Ok(PublicRoomDetailView {
                    room: PublicRoomView {
                        room: room(),
                        viewer: None,
                    },
                    admin_display_names: vec!["小林".to_owned()],
                    online: Some(PublicRoomOnline {
                        count: 200,
                        at_least: true,
                    }),
                })
            } else {
                Err(PublicDirectoryFailure::new(
                    "test",
                    PublicDirectoryFailureKind::NotFound,
                ))
            }
        })
    }
}

#[tokio::test]
async fn 不登录也能翻目录_带着登录的多一个处境() {
    let directory = Arc::new(FakeDirectory::default());
    let app = test_router(directory.clone());
    let response = app
        .clone()
        .oneshot(get("/public-rooms", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response_json(response).await,
        json!({
            "rooms": [{
                "catalogId": CATALOG_UUID,
                "shareCode": "Club01",
                "shareUrl": "https://agent-room.test/r/Club01",
                "name": "读书会",
                "description": "每周读一本",
                "ownerName": "阿明",
                "memberCount": 12,
                "agentCount": 3,
                "joinMode": "open",
                "endToEndEncrypted": false,
                "createdAtUnixMs": 1_000,
                "lastMessageAtUnixMs": null,
                "viewer": null,
            }],
            "nextCursor": "24",
        })
    );

    for (cookie, expected) in [("session-secret", json!("joined")), ("stale", Value::Null)] {
        let response = app
            .clone()
            .oneshot(get("/public-rooms", Some(cookie)))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "登录过期的照样回答");
        assert_eq!(
            response_json(response).await["rooms"][0]["viewer"],
            expected
        );
    }
    let browsed = directory.browsed.lock().unwrap();
    assert!(browsed[0].viewer.is_none());
    assert_eq!(
        browsed[1].viewer.as_ref().unwrap().principal_id,
        principal()
    );
    assert!(browsed[2].viewer.is_none());
}

#[tokio::test]
async fn 筛选排序翻页照查询参数_认不出的值回_400() {
    let directory = Arc::new(FakeDirectory::default());
    let app = test_router(directory.clone());
    let response = app
        .clone()
        .oneshot(get(
            "/public-rooms?q=%E8%AF%BB%E4%B9%A6&join=request&active=true&encrypted=false&sort=members&cursor=48",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    {
        let browsed = directory.browsed.lock().unwrap();
        let request = &browsed[0];
        assert_eq!(request.text, "读书");
        assert_eq!(request.join_mode, Some(RoomJoinMode::Request));
        assert!(request.recently_active);
        assert!(request.unencrypted_only);
        assert_eq!(request.sort, PublicRoomSort::Members);
        assert_eq!(request.offset, 48);
    }
    for query in [
        "join=invite_only",
        "active=yes",
        "encrypted=true",
        "sort=random",
        "cursor=-1",
        "cursor=%2B5",
        "cursor=abc",
    ] {
        let response = app
            .clone()
            .oneshot(get(&format!("/public-rooms?{query}"), None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(
            response_json(response).await["code"],
            "public_room.invalid_query"
        );
    }
    assert_eq!(directory.browsed.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn 一间房的详情多几位管理员_没有的回_404_不写房间名() {
    let directory = Arc::new(FakeDirectory::default());
    let app = test_router(directory.clone());
    let response = app
        .clone()
        .oneshot(get("/public-rooms/Club01", Some("session-secret")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["name"], "读书会");
    assert_eq!(body["admins"], json!(["小林"]));
    assert_eq!(body["online"], json!({"count": 200, "atLeast": true}));

    let response = app
        .oneshot(get("/public-rooms/Gone99", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = response_json(response).await;
    assert_eq!(body["code"], "public_room.not_found");
    assert!(!body.to_string().contains("读书会"));
    assert_eq!(
        *directory.details.lock().unwrap(),
        [
            (Some(principal()), "Club01".to_owned()),
            (None, "Gone99".to_owned()),
        ]
    );
}

#[tokio::test]
async fn 两个接口合起来按来源限流_超了回_429_带重试时间() {
    let directory = Arc::new(FakeDirectory::default());
    let app = test_router(directory.clone());
    let from = |uri: &str, address: &str| {
        let mut request = get(uri, None);
        request
            .headers_mut()
            .insert("x-forwarded-for", HeaderValue::from_str(address).unwrap());
        request
    };
    for index in 0..PER_MINUTE {
        let uri = if index % 2 == 0 {
            "/public-rooms"
        } else {
            "/public-rooms/Club01"
        };
        let response = app.clone().oneshot(from(uri, "203.0.113.7")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "第 {index} 次");
    }

    let response = app
        .clone()
        .oneshot(from("/public-rooms/Club01", "203.0.113.7"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()[header::RETRY_AFTER], "60");
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body = response_json(response).await;
    assert_eq!(body["code"], "public_rooms.rate_limited");
    assert_eq!(body["retryAfterSeconds"], 60);
    assert_eq!(
        directory.details.lock().unwrap().len(),
        usize::try_from(PER_MINUTE / 2).unwrap(),
        "超了的那次不去查"
    );

    let response = app
        .oneshot(from("/public-rooms", "203.0.113.8"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "别的来源照常");
}

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> UtcMillis {
        UtcMillis::new(1_790_000_000_000).unwrap()
    }
}

fn test_router(directory: Arc<FakeDirectory>) -> Router {
    router(PublicRoomsHttpState {
        directory,
        authentication: Arc::new(FakeAuthentication),
        limits: Arc::new(AnonymousLimiter::new(Vec::new(), Arc::new(FixedClock))),
        site_origin: "https://agent-room.test".into(),
    })
    .layer(middleware::from_fn(crate::correlation::attach))
}

fn get(uri: &str, session: Option<&str>) -> Request<Body> {
    let mut request = Request::builder().uri(uri);
    if let Some(session) = session {
        request = request.header(
            header::COOKIE,
            format!("__Host-agent-room-session={session}"),
        );
    }
    request.body(Body::empty()).unwrap()
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 64 * 1_024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn room() -> PublicRoomRecord {
    PublicRoomRecord {
        catalog_id: RoomCatalogId::from_uuid(Uuid::parse_str(CATALOG_UUID).unwrap()),
        name: "读书会".to_owned(),
        description: "每周读一本".to_owned(),
        owner_display_name: "阿明".to_owned(),
        member_count: 12,
        agent_count: 3,
        join_mode: RoomJoinMode::Open,
        end_to_end_encrypted: false,
        share_code: RoomShareCode::parse("Club01").unwrap(),
        created_at: UtcMillis::new(1_000).unwrap(),
        last_message_at: None,
    }
}

fn principal() -> PrincipalId {
    PrincipalId::from_uuid(Uuid::parse_str(PRINCIPAL_UUID).unwrap())
}

/// 只认 `session-secret`，别的都当登录失效。
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
        assert_eq!(requirement, AuthenticationRequirement::ActiveSession);
        let valid = session_secret.expose() == "session-secret";
        Box::pin(async move {
            if valid {
                Ok(AuthenticatedPrincipal {
                    principal_id: principal(),
                    matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
                    display_name: "Agent Room User".to_owned(),
                    locale: "zh-CN".to_owned(),
                    authenticated_at: UtcMillis::new(1_700_000_000_000).unwrap(),
                    expires_at: UtcMillis::new(1_700_000_900_000).unwrap(),
                    recently_authenticated: false,
                })
            } else {
                Err(AuthenticationFailure::new(
                    "test",
                    AuthenticationFailureKind::InvalidSession,
                ))
            }
        })
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
