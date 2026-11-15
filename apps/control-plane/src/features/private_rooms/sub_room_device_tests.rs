//! 子房间第 2g-1 步：本机 Bridge 替 Agent 建子房间，还是那个设备签名的建房地址，带上 `parentRoomId`。
//! 门和分类的名字只有建子房间时能带；规矩在用例里，这里看路由交了什么、回答多了哪几项。

use super::*;

const TARGET: &str = "/devices/current/private-rooms";
const PARENT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e70";

fn device_sub_room_router(sub_rooms: Arc<FakeSubRooms>, body: &str) -> axum::Router {
    let state = PrivateRoomHttpState::new(
        PrivateRoomHttpDependencies {
            rooms: Arc::new(FakeRooms::new()),
            sub_rooms,
            authentication: Arc::<FakeAuthentication>::default(),
            devices: FakeDevices::expecting("POST", TARGET, body),
            secrets: Arc::new(SecureSecretFactory),
        },
        &Url::parse(FRONTEND_ORIGIN).expect("前端地址有效"),
        &crate::config::DesktopOrigins::for_tests(),
    );
    router(state).layer(middleware::from_fn(crate::correlation::attach))
}

async fn send(sub_rooms: &Arc<FakeSubRooms>, body: &Value) -> axum::response::Response {
    let body = body.to_string();
    device_sub_room_router(sub_rooms.clone(), &body)
        .oneshot(device_request(TARGET, &body, true))
        .await
        .expect("路由可调用")
}

#[tokio::test]
async fn 带上父房间建的是子房间_门和分类的名字交给用例_回答带着父房间和分类() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let response = send(
        &sub_rooms,
        &json!({
            "name": "Ship it",
            "agentId": AGENT_UUID,
            "parentRoomId": PARENT_UUID,
            "access": "request",
            "category": "release",
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response_json(response).await,
        json!({
            "catalogId": CATALOG_UUID,
            "matrixRoomId": "!private:matrix.agent-room.test",
            "name": "Incident Room",
            "parent": { "catalogId": PARENT_UUID, "name": "Weekly" },
            "category": "Release",
        })
    );
    let created = sub_rooms.agent_created.lock().unwrap();
    assert_eq!(created.len(), 1);
    let request = &created[0];
    assert!(matches!(request.actor, PrivateRoomActor::Device(_)));
    assert_eq!(
        request.parent_catalog_id,
        RoomCatalogId::from_uuid(uuid(PARENT_UUID))
    );
    assert_eq!(request.catalog_id, catalog_id());
    assert_eq!(request.name, "Ship it");
    assert_eq!(request.access, RoomJoinMode::Request);
    assert_eq!(request.category.as_deref(), Some("release"));
    assert_eq!(
        request.created_by_agent,
        AgentId::from_uuid(uuid(AGENT_UUID))
    );
    assert!(sub_rooms.created.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 不带门是没有门_没有这个分类时回答说一声() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let response = send(
        &sub_rooms,
        &json!({
            "name": "Ship it",
            "agentId": AGENT_UUID,
            "parentRoomId": PARENT_UUID,
            "category": "Design",
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = response_json(response).await;
    assert_eq!(body["categoryMissing"], true);
    assert!(body.get("category").is_none());
    assert_eq!(
        sub_rooms.agent_created.lock().unwrap()[0].access,
        RoomJoinMode::Open
    );
}

#[tokio::test]
async fn 门和分类只有建子房间时能带_父房间号和分类的名字不对的不收() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    for body in [
        json!({ "name": "Ship it", "agentId": AGENT_UUID, "access": "request" }),
        json!({ "name": "Ship it", "agentId": AGENT_UUID, "category": "Release" }),
        json!({ "name": "Ship it", "agentId": AGENT_UUID, "parentRoomId": "not-a-room" }),
        json!({ "name": "Ship it", "agentId": AGENT_UUID, "parentRoomId": PARENT_UUID, "category": " Release" }),
        json!({ "name": "Ship it", "agentId": AGENT_UUID, "parentRoomId": PARENT_UUID, "category": "" }),
        json!({ "name": "Ship it", "agentId": AGENT_UUID, "parentRoomId": PARENT_UUID, "access": "closed" }),
        json!({ "name": "Ship it", "agentId": AGENT_UUID, "parentRoomId": PARENT_UUID, "hidden": true }),
    ] {
        let refused = send(&sub_rooms, &body).await;
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(
            response_json(refused).await["code"],
            "private_room.invalid_creation_body",
            "{body}"
        );
    }
    assert!(sub_rooms.agent_created.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 子房间建不了时照子房间的码说() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    *sub_rooms.failure.lock().unwrap() = Some(PrivateRoomFailure::new(
        "test",
        PrivateRoomFailureStage::Validation,
        PrivateRoomFailureKind::SubRoom(SubRoomRefusal::AgentsMayNotCreate),
    ));
    let refused = send(
        &sub_rooms,
        &json!({ "name": "Ship it", "agentId": AGENT_UUID, "parentRoomId": PARENT_UUID }),
    )
    .await;

    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(refused).await["code"],
        "sub_room.agents_may_not_create"
    );
}
