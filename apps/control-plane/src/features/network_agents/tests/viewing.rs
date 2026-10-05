//! 按需查看的 HTTP 接口：按 ID 取、看前后、往前翻。

use std::sync::Arc;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use tower::ServiceExt;

use super::{FakeAgents, FakeMessaging, TOKEN, app_with, body_json};
use crate::network_gateway::{NetworkAgentRoomQuery, NetworkGatewayFailure};

fn get(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap()
}

async fn call(messaging: &Arc<FakeMessaging>, path: &str) -> axum::response::Response {
    app_with(
        Arc::new(FakeAgents::default()),
        messaging.clone(),
        1_758_600_000_000,
    )
    .oneshot(get(path))
    .await
    .unwrap()
}

#[tokio::test]
async fn 按_id_取_逗号隔开_找不到的放进_missing() {
    let messaging = Arc::new(FakeMessaging::default());
    let response = call(
        &messaging,
        "/v1/network-agents/me/messages/lookup?ids=$hello:matrix.test,%200198b601-77a1-7bb8-83eb-a8fe68c97e99,",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "no-store",
        "消息内容不进缓存"
    );
    let body = body_json(response).await;
    assert_eq!(body["schemaVersion"], 1);
    assert_eq!(body["messages"][0]["eventId"], "$hello:matrix.test");
    assert_eq!(
        body["missing"],
        serde_json::json!(["0198b601-77a1-7bb8-83eb-a8fe68c97e99"])
    );
    let lookups = messaging.lookups.lock().unwrap();
    assert_eq!(lookups[0].0, TOKEN);
    assert_eq!(
        lookups[0].1,
        ["$hello:matrix.test", "0198b601-77a1-7bb8-83eb-a8fe68c97e99"]
    );
}

#[tokio::test]
async fn 都找到了就不写_missing_没给_ids_时说明该怎么写() {
    let messaging = Arc::new(FakeMessaging::default());
    let response = call(
        &messaging,
        "/v1/network-agents/me/messages/lookup?ids=$hello:matrix.test",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_json(response).await.get("missing").is_none());

    let response = call(&messaging, "/v1/network-agents/me/messages/lookup").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_json(response).await;
    assert_eq!(body["code"], "network_agent.invalid_request");
    assert_eq!(body["details"]["field"], "ids");
    assert_eq!(messaging.lookups.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn 看前后_limit_条前后各一半_前面多给一条() {
    let messaging = Arc::new(FakeMessaging::default());
    let response = call(
        &messaging,
        "/v1/network-agents/me/rooms/!lobby:matrix.test/messages?around=$hello:matrix.test&limit=5",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["messages"][0]["eventId"], "$earlier:matrix.test");
    assert_eq!(body["nextCursor"], "$earlier:matrix.test");
    let views = messaging.views.lock().unwrap();
    assert_eq!(views[0].1.room.as_deref(), Some("!lobby:matrix.test"));
    assert_eq!(
        views[0].1.query,
        NetworkAgentRoomQuery::Around {
            id: "$hello:matrix.test".to_owned(),
            before: 3,
            after: 2,
        }
    );
}

#[tokio::test]
async fn 往前翻默认二十条_可以只看某个人_只看提到我的() {
    let messaging = Arc::new(FakeMessaging::default());
    for query in [
        "",
        "?before=$hello:matrix.test&limit=50&from=Ada&mentionsMe=true",
        "?after=0198b601-77a1-7bb8-83eb-a8fe68c97e99",
    ] {
        let response = call(
            &messaging,
            &format!("/v1/network-agents/me/rooms/!lobby:matrix.test/messages{query}"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK, "{query}");
    }
    let views = messaging.views.lock().unwrap();
    assert_eq!(
        views[0].1.query,
        NetworkAgentRoomQuery::History {
            before: None,
            after: None,
            limit: 20,
            from: None,
            mentions_me: false,
        }
    );
    assert_eq!(
        views[1].1.query,
        NetworkAgentRoomQuery::History {
            before: Some("$hello:matrix.test".to_owned()),
            after: None,
            limit: 50,
            from: Some("Ada".to_owned()),
            mentions_me: true,
        }
    );
    assert_eq!(
        views[2].1.query,
        NetworkAgentRoomQuery::History {
            before: None,
            after: Some("0198b601-77a1-7bb8-83eb-a8fe68c97e99".to_owned()),
            limit: 20,
            from: None,
            mentions_me: false,
        }
    );
}

#[tokio::test]
async fn 翻看的参数不对时指出是哪一项_不问网关() {
    let messaging = Arc::new(FakeMessaging::default());
    for (query, field) in [
        ("?limit=0", "limit"),
        ("?limit=51", "limit"),
        ("?around=$hello:matrix.test&from=Ada", "around"),
        ("?around=$hello:matrix.test&mentionsMe=true", "around"),
        (
            "?before=$hello:matrix.test&after=$hello:matrix.test",
            "after",
        ),
        ("?mentionsMe=maybe", "query"),
        ("?unknown=1", "query"),
    ] {
        let response = call(
            &messaging,
            &format!("/v1/network-agents/me/rooms/!lobby:matrix.test/messages{query}"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
        let body = body_json(response).await;
        assert_eq!(body["code"], "network_agent.invalid_request", "{query}");
        assert_eq!(body["details"]["field"], field, "{query}");
    }
    assert!(messaging.views.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 房间里找不到那条_参数不对_不在房间里都按稳定错误码回答() {
    for (failure, status, code) in [
        (
            NetworkGatewayFailure::MessageNotFound,
            StatusCode::NOT_FOUND,
            "network_agent.message_not_found",
        ),
        (
            NetworkGatewayFailure::InvalidLookup("from"),
            StatusCode::BAD_REQUEST,
            "network_agent.invalid_request",
        ),
        (
            NetworkGatewayFailure::RoomNotJoined,
            StatusCode::NOT_FOUND,
            "network_agent.room_not_joined",
        ),
    ] {
        let messaging = FakeMessaging::failing(failure);
        let response = call(
            &messaging,
            "/v1/network-agents/me/rooms/!lobby:matrix.test/messages?around=$hello:matrix.test",
        )
        .await;
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(body_json(response).await["code"], code);
    }
}
