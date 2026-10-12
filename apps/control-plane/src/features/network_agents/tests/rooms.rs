//! 网络 Agent 自己建房间、关房间的 HTTP 接口（`specs/agent-rooms/design.md` 第 4 步）。

use std::sync::Arc;

use agent_room_application::network_agents::{NetworkAgentFailure, NetworkAgentFailureKind};
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use serde_json::json;
use tower::ServiceExt;

use super::{FakeAgents, FakeMessaging, PRIVATE_CATALOG_UUID, TOKEN, app_with, body_json};
use crate::network_gateway::NetworkGatewayFailure;

const ROOMS: &str = "/v1/network-agents/me/private-rooms";

fn request(method: Method, path: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

async fn call(messaging: &Arc<FakeMessaging>, request: Request<Body>) -> axum::response::Response {
    app_with(
        Arc::new(FakeAgents::default()),
        messaging.clone(),
        1_758_600_000_000,
    )
    .oneshot(request)
    .await
    .unwrap()
}

async fn create(messaging: &Arc<FakeMessaging>, body: &str) -> axum::response::Response {
    call(messaging, request(Method::POST, ROOMS, body)).await
}

#[tokio::test]
async fn 建房交给网关_新建的回答_201_带口令和要转给别的_agent_的话() {
    let messaging = Arc::new(FakeMessaging::default());
    let response = create(&messaging, r#"{"name":"周报"}"#).await;

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "no-store",
        "口令不进缓存"
    );
    let body = body_json(response).await;
    assert_eq!(body["schemaVersion"], 1);
    assert_eq!(body["created"], true);
    assert_eq!(
        body["room"],
        json!({
            "catalogId": PRIVATE_CATALOG_UUID,
            "matrixRoomId": "!owned:matrix.test",
            "name": "周报",
        })
    );
    assert_eq!(
        body["code"],
        json!({"state": "visible", "code": "K7P3-Q9XW-2DMA"})
    );
    assert_eq!(
        body["guideUrl"],
        "https://api.agent-room.example/agents.txt"
    );
    for language in ["zh", "en"] {
        let text = body["share"][language].as_str().unwrap();
        for part in [
            "https://api.agent-room.example/agents.txt",
            "K7P3-Q9XW-2DMA",
            PRIVATE_CATALOG_UUID,
            "周报",
        ] {
            assert!(text.contains(part), "{language} 缺 {part}：{text}");
        }
    }
    assert_eq!(
        *messaging.created_rooms.lock().unwrap(),
        [(TOKEN.to_owned(), "周报".to_owned())]
    );
}

#[tokio::test]
async fn 回到同名房间回答_200_口令看不到时只说敲门() {
    let messaging = Arc::new(FakeMessaging::default());
    let response = create(&messaging, r#"{"name":"已有的"}"#).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_json(response).await["created"], false);

    let body = body_json(create(&messaging, r#"{"name":"看不到口令"}"#).await).await;
    assert_eq!(body["code"], json!({"state": "hidden"}));
    let zh = body["share"]["zh"].as_str().unwrap();
    let en = body["share"]["en"].as_str().unwrap();
    assert!(
        zh.contains("敲") && zh.contains(PRIVATE_CATALOG_UUID),
        "{zh}"
    );
    assert!(
        en.contains("knock") && !en.contains("with the code"),
        "{en}"
    );
}

#[tokio::test]
async fn 请求体不对时不交给网关() {
    let messaging = Arc::new(FakeMessaging::default());
    for body in ["", "{}", r#"{"name":"周报","extra":1}"#, r#"{"name":1}"#] {
        let response = create(&messaging, body).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(
            body_json(response).await["code"],
            "network_agent.invalid_request"
        );
    }
    assert!(messaging.created_rooms.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 建房的几种失败各有错误码() {
    for (kind, status, code) in [
        (
            NetworkAgentFailureKind::RoomNameInvalid,
            StatusCode::BAD_REQUEST,
            "network_agent.room_name_invalid",
        ),
        (
            NetworkAgentFailureKind::RoomLimitReached,
            StatusCode::CONFLICT,
            "network_agent.room_limit_reached",
        ),
        (
            NetworkAgentFailureKind::RoomCapacityReached,
            StatusCode::SERVICE_UNAVAILABLE,
            "network_agent.room_capacity_reached",
        ),
        (
            NetworkAgentFailureKind::RoomNotOwned,
            StatusCode::FORBIDDEN,
            "network_agent.room_not_owned",
        ),
    ] {
        let messaging =
            FakeMessaging::failing(NetworkGatewayFailure::Agent(NetworkAgentFailure::new(kind)));
        let response = create(&messaging, r#"{"name":"周报"}"#).await;
        assert_eq!(response.status(), status, "{code}");
        let body = body_json(response).await;
        assert_eq!(body["code"], code);
        assert!(
            body["message"].as_str().unwrap().is_ascii(),
            "给 Agent 的说明是英文：{body}"
        );
    }
}

#[tokio::test]
async fn 关房间按房间号或_room_id_交给网关_成功回答_204() {
    let messaging = Arc::new(FakeMessaging::default());
    for room in [
        PRIVATE_CATALOG_UUID,
        "!owned:matrix.test",
        "%21owned%3Amatrix.test",
    ] {
        let response = call(
            &messaging,
            request(Method::DELETE, &format!("{ROOMS}/{room}"), ""),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT, "{room}");
    }
    assert_eq!(
        *messaging.closed_rooms.lock().unwrap(),
        [
            (TOKEN.to_owned(), PRIVATE_CATALOG_UUID.to_owned()),
            (TOKEN.to_owned(), "!owned:matrix.test".to_owned()),
            (TOKEN.to_owned(), "!owned:matrix.test".to_owned()),
        ]
    );

    let messaging = FakeMessaging::failing(NetworkGatewayFailure::Agent(NetworkAgentFailure::new(
        NetworkAgentFailureKind::RoomNotOwned,
    )));
    let response = call(
        &messaging,
        request(
            Method::DELETE,
            &format!("{ROOMS}/{PRIVATE_CATALOG_UUID}"),
            "",
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        body_json(response).await["code"],
        "network_agent.room_not_owned"
    );
}
