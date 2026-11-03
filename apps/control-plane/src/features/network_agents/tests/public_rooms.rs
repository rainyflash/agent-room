//! 找公开房间：只要网络 Agent 进得去的、不要令牌、和发现页一起按来源限流；总开关关着时说已关闭。

use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::AuthenticatedPrincipal,
    ports::{PortFuture, PublicRoomRecord, PublicRoomSort},
    public_room_directory::{
        BrowsePublicRooms, PublicDirectoryFailure, PublicDirectoryFailureKind,
        PublicDirectoryResult, PublicRoomDetailView, PublicRoomDirectoryUseCases, PublicRoomPage,
        PublicRoomView,
    },
};
use agent_room_domain::{
    ids::RoomCatalogId,
    room_listing::{RoomJoinMode, RoomShareCode},
    time::UtcMillis,
};
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
    middleware,
};
use tower::ServiceExt;
use uuid::Uuid;

use super::{FakeAgents, FakeMessaging, FixedClock, body_json, state_with};
use crate::features::{
    anonymous_limits::{AnonymousLimiter, PER_MINUTE},
    network_agents::{NetworkAgentPublicRooms, router},
};

pub(in super::super) const PUBLIC_ROOM_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e5a";
const NOW: i64 = 1_758_600_000_000;

/// 记下每次翻目录的请求，回一间直接进的房间，还有下一页。
#[derive(Default)]
pub(in super::super) struct FakeDirectory {
    pub(in super::super) browsed: Mutex<Vec<BrowsePublicRooms>>,
}

impl PublicRoomDirectoryUseCases for FakeDirectory {
    fn browse(
        &self,
        request: BrowsePublicRooms,
    ) -> PortFuture<'_, PublicDirectoryResult<PublicRoomPage>> {
        self.browsed.lock().unwrap().push(request);
        Box::pin(async {
            Ok(PublicRoomPage {
                rooms: vec![PublicRoomView {
                    room: room(),
                    viewer: None,
                }],
                next_offset: Some(24),
            })
        })
    }

    fn detail(
        &self,
        _viewer: Option<AuthenticatedPrincipal>,
        _share_code: String,
    ) -> PortFuture<'_, PublicDirectoryResult<PublicRoomDetailView>> {
        Box::pin(async {
            Err(PublicDirectoryFailure::new(
                "public_room.detail",
                PublicDirectoryFailureKind::NotFound,
            ))
        })
    }
}

fn room() -> PublicRoomRecord {
    PublicRoomRecord {
        catalog_id: RoomCatalogId::from_uuid(Uuid::parse_str(PUBLIC_ROOM_UUID).unwrap()),
        name: "Book club".to_owned(),
        description: "One book a week".to_owned(),
        owner_display_name: "Ming".to_owned(),
        member_count: 12,
        agent_count: 3,
        join_mode: RoomJoinMode::Open,
        end_to_end_encrypted: false,
        share_code: RoomShareCode::parse("Club01").unwrap(),
        created_at: UtcMillis::new(1_000).unwrap(),
        last_message_at: Some(UtcMillis::new(2_000).unwrap()),
    }
}

/// 总开关开着、找公开房间用这个目录的应用。
pub(in super::super) fn app_with_directory(directory: Arc<FakeDirectory>) -> axum::Router {
    let mut state = state_with(
        Arc::new(FakeAgents::default()),
        Arc::new(FakeMessaging::default()),
        NOW,
    );
    state.public_rooms = Some(NetworkAgentPublicRooms {
        directory,
        limits: Arc::new(AnonymousLimiter::new(Vec::new(), Arc::new(FixedClock(NOW)))),
        site_origin: "https://agent-room.test".into(),
    });
    router(state).layer(middleware::from_fn(crate::correlation::attach))
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header("x-forwarded-for", "198.51.100.7")
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn 不要令牌_只列网络_agent_进得去的_带房间号和分享链接() {
    let directory = Arc::new(FakeDirectory::default());
    let response = app_with_directory(directory.clone())
        .oneshot(get("/v1/network-agents/public-rooms?q=book&cursor=24"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body = body_json(response).await;
    assert_eq!(body["schemaVersion"], 1);
    assert_eq!(body["nextCursor"], "24");
    let room = &body["rooms"][0];
    assert_eq!(room["room"], PUBLIC_ROOM_UUID);
    assert_eq!(room["shareUrl"], "https://agent-room.test/r/Club01");
    assert_eq!(room["name"], "Book club");
    assert_eq!(room["description"], "One book a week");
    assert_eq!(room["ownerName"], "Ming");
    assert_eq!(room["joinMode"], "open");
    assert_eq!(room["endToEndEncrypted"], false);
    assert_eq!(
        (room["memberCount"].clone(), room["agentCount"].clone()),
        (12.into(), 3.into())
    );
    assert_eq!(room["lastMessageAtUnixMs"], 2_000);

    let browsed = directory.browsed.lock().unwrap();
    let request = &browsed[0];
    assert!(request.network_agents_only);
    assert_eq!(request.viewer, None);
    assert_eq!(request.text, "book");
    assert_eq!(request.offset, 24);
    assert_eq!(request.sort, PublicRoomSort::Active);
    assert_eq!(request.join_mode, None);
}

#[tokio::test]
async fn 翻页的记号认不出说清楚_总开关关着说已关闭() {
    let app = app_with_directory(Arc::new(FakeDirectory::default()));
    for uri in [
        "/v1/network-agents/public-rooms?cursor=-1",
        "/v1/network-agents/public-rooms?cursor=abc",
    ] {
        let response = app.clone().oneshot(get(uri)).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(
            body_json(response).await["code"],
            "network_agent.invalid_request"
        );
    }

    let response = super::app_at(Arc::new(FakeAgents::default()), NOW)
        .oneshot(get("/v1/network-agents/public-rooms"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body_json(response).await["code"], "network_agent.disabled");
}

#[tokio::test]
async fn 同一个来源一分钟问太多回_429_带_retry_after() {
    let app = app_with_directory(Arc::new(FakeDirectory::default()));
    for _ in 0..PER_MINUTE {
        let response = app
            .clone()
            .oneshot(get("/v1/network-agents/public-rooms"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    let response = app
        .oneshot(get("/v1/network-agents/public-rooms"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(response.headers().contains_key(header::RETRY_AFTER));
    assert_eq!(
        body_json(response).await["code"],
        "network_agent.rate_limited"
    );
}
