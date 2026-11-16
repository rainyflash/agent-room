//! 两个只读的：自己在等的认领（网页的设置拿它找这间房在等的那一个），和谁登录了都能问的“房间绑着哪个
//! 仓库”（房间名旁边的勾，只有仓库和地址）。

use super::*;

#[tokio::test]
async fn 列出自己在等的认领_认上了就不在里面() {
    let rooms = Arc::new(FakeProjectRooms::default());
    let app = test_router(rooms.clone());

    let response = app
        .clone()
        .oneshot(request(Method::GET, "/project-rooms/claims", false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body = response_json(response).await;
    let claims = body["claims"].as_array().expect("claims 是一列");
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0]["claimId"], CLAIM_UUID);
    assert_eq!(claims[0]["status"], "pending");
    assert_eq!(claims[0]["repository"], "octo-org/hello.world");
    assert_eq!(claims[0]["shareUrl"], "https://agent-room.test/r/aB3xYz");

    *rooms.bound.lock().unwrap() = true;
    let response = app
        .oneshot(request(Method::GET, "/project-rooms/claims", false))
        .await
        .unwrap();
    assert_eq!(response_json(response).await, json!({ "claims": [] }));
    assert_eq!(
        *rooms.listed_for.lock().unwrap(),
        [
            PrivateRoomActor::Person(actor()),
            PrivateRoomActor::Person(actor())
        ]
    );
}

#[tokio::test]
async fn 谁都能问房间绑着哪个仓库_只给仓库和地址() {
    let rooms = Arc::new(FakeProjectRooms::default());
    let app = test_router(rooms.clone());
    let path = format!("/rooms/{CATALOG_UUID}/repository");

    let response = app
        .clone()
        .oneshot(request(Method::GET, &path, false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response_json(response).await,
        json!({
            "repository": "octo-org/hello.world",
            "repositoryUrl": "https://github.com/octo-org/hello.world",
        }),
        "谁核对上的、在哪个文件里只有房主那个接口给"
    );

    *rooms.unbound_room.lock().unwrap() = true;
    let response = app
        .clone()
        .oneshot(request(Method::GET, &path, false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await, Value::Null);
    {
        let calls = rooms.rooms.lock().unwrap();
        assert!(calls.iter().all(|(call, request)| *call == "public"
            && request.catalog_id == catalog()
            && request.actor == PrivateRoomActor::Person(actor())));
    }

    let response = app
        .oneshot(request(Method::GET, "/rooms/abc/repository", false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn 不登录问不到() {
    let rooms = Arc::new(FakeProjectRooms::default());
    let app = test_router(rooms.clone());
    for path in [
        "/project-rooms/claims".to_owned(),
        format!("/rooms/{CATALOG_UUID}/repository"),
    ] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    assert!(rooms.listed_for.lock().unwrap().is_empty());
    assert!(rooms.rooms.lock().unwrap().is_empty());
}
