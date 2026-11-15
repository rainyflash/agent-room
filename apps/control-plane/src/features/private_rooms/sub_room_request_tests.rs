//! 子房间的申请和批准（`specs/redesign/sub-rooms.md` 第 2d-2 步）的接口：申请、撤回、看申请、
//! 批准或拒绝。规则在用例里，这里看路由把什么交给了用例、回答长什么样。

use super::*;

fn requests_uri() -> String {
    format!("/private-rooms/{CATALOG_UUID}/sub-rooms/{SUB_ROOM_UUID}/join-requests")
}

fn pair() -> (RoomCatalogId, RoomCatalogId) {
    (catalog_id(), RoomCatalogId::from_uuid(uuid(SUB_ROOM_UUID)))
}

/// 不带正文的申请（网页上不留话时就是这样）。
fn bare_request(origin: bool) -> Request<Body> {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri(requests_uri())
        .header(header::COOKIE, "__Host-agent-room-session=session-secret");
    if origin {
        request = request.header(header::ORIGIN, FRONTEND_ORIGIN);
    }
    request.body(Body::empty()).expect("HTTP 请求有效")
}

#[tokio::test]
async fn 申请可以不带正文_留的话原样交给用例_在等的回申请的时间_不用等的回已加入() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let authentication = Arc::new(FakeAuthentication::default());
    let app = sub_room_router(sub_rooms.clone(), authentication.clone());

    let bare = app
        .clone()
        .oneshot(bare_request(true))
        .await
        .expect("申请的路由可调用");
    assert_eq!(bare.status(), StatusCode::OK);
    assert_eq!(bare.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response_json(bare).await,
        json!({
            "catalogId": SUB_ROOM_UUID,
            "membership": "pending",
            "requestedAtUnixMs": REQUESTED_AT
        })
    );
    let note = json!({ "message": "想来帮忙" });
    let noted = app
        .clone()
        .oneshot(sub_request(
            Method::POST,
            &requests_uri(),
            &note,
            true,
            None,
        ))
        .await
        .expect("申请的路由可调用");
    assert_eq!(noted.status(), StatusCode::OK);
    let requests = sub_rooms.requested.lock().unwrap().clone();
    assert_eq!(
        requests
            .iter()
            .map(|request| (
                request.parent_catalog_id,
                request.catalog_id,
                request.message.as_str()
            ))
            .collect::<Vec<_>>(),
        [(pair().0, pair().1, ""), (pair().0, pair().1, "想来帮忙")],
        "留的话原样交给用例，长短和换行由它管"
    );
    assert_eq!(
        authentication.calls.lock().unwrap().as_slice(),
        &[AuthenticationRequirement::ActiveSession; 2]
    );

    *sub_rooms.joins_directly.lock().unwrap() = true;
    let joined = app
        .clone()
        .oneshot(bare_request(true))
        .await
        .expect("申请的路由可调用");
    assert_eq!(
        response_json(joined).await,
        json!({ "catalogId": SUB_ROOM_UUID, "membership": "joined" }),
        "不用等的不写申请的时间"
    );

    for (request, status, code) in [
        (
            bare_request(false),
            StatusCode::FORBIDDEN,
            "private_room.invalid_origin",
        ),
        (
            sub_request(
                Method::POST,
                &requests_uri(),
                &json!({ "message": "hi", "note": "extra" }),
                true,
                None,
            ),
            StatusCode::BAD_REQUEST,
            "sub_room.invalid_join_request_body",
        ),
        (
            sub_request(
                Method::POST,
                &requests_uri(),
                &json!({ "message": 3 }),
                true,
                None,
            ),
            StatusCode::BAD_REQUEST,
            "sub_room.invalid_join_request_body",
        ),
    ] {
        let response = app.clone().oneshot(request).await.expect("路由可调用");
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
    assert_eq!(
        sub_rooms.requested.lock().unwrap().len(),
        3,
        "被拒的没到用例"
    );
}

#[tokio::test]
async fn 撤回不回内容_看申请只要有效登录_回账户昵称留话和时间() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let authentication = Arc::new(FakeAuthentication::default());
    let app = sub_room_router(sub_rooms.clone(), authentication.clone());

    let withdrawn = app
        .clone()
        .oneshot(sub_request(
            Method::DELETE,
            &requests_uri(),
            &json!({}),
            true,
            None,
        ))
        .await
        .expect("撤回的路由可调用");
    assert_eq!(withdrawn.status(), StatusCode::NO_CONTENT);
    assert_eq!(withdrawn.headers()[header::CACHE_CONTROL], "no-store");
    assert!(
        to_bytes(withdrawn.into_body(), 64)
            .await
            .expect("读得出回答")
            .is_empty()
    );
    assert_eq!(sub_rooms.withdrawn.lock().unwrap().as_slice(), &[pair()]);
    let refused = app
        .clone()
        .oneshot(sub_request(
            Method::DELETE,
            &requests_uri(),
            &json!({}),
            false,
            None,
        ))
        .await
        .expect("撤回的路由可调用");
    assert_eq!(refused.status(), StatusCode::FORBIDDEN, "撤回也要同源");

    let listed = app
        .clone()
        .oneshot(sub_request(
            Method::GET,
            &requests_uri(),
            &json!({}),
            false,
            None,
        ))
        .await
        .expect("看申请的路由可调用");
    assert_eq!(listed.status(), StatusCode::OK);
    assert_eq!(listed.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response_json(listed).await,
        json!({
            "requests": [{
                "principalId": TARGET_UUID,
                "displayName": "riley",
                "message": "想来帮忙",
                "requestedAtUnixMs": REQUESTED_AT
            }]
        })
    );
    assert_eq!(
        sub_rooms.pending_listed.lock().unwrap().as_slice(),
        &[pair()]
    );
    assert_eq!(
        authentication.calls.lock().unwrap().as_slice(),
        &[AuthenticationRequirement::ActiveSession; 2],
        "被拒的那次撤回没走到认证"
    );
}

#[tokio::test]
async fn 批准和拒绝_路径里是申请人_不认识的决定和多余的字段不收() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let app = sub_room_router(sub_rooms.clone(), Arc::default());
    let uri = format!("{}/{TARGET_UUID}", requests_uri());

    for (decision, status) in [("approve", "approved"), ("reject", "rejected")] {
        let response = app
            .clone()
            .oneshot(sub_request(
                Method::PUT,
                &uri,
                &json!({ "decision": decision }),
                true,
                None,
            ))
            .await
            .expect("处理申请的路由可调用");
        assert_eq!(response.status(), StatusCode::OK, "{decision}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(response_json(response).await, json!({ "status": status }));
    }
    let answered = sub_rooms.answered.lock().unwrap().clone();
    assert_eq!(
        answered
            .iter()
            .map(|answer| (
                answer.parent_catalog_id,
                answer.catalog_id,
                answer.requester,
                answer.approve
            ))
            .collect::<Vec<_>>(),
        [
            (pair().0, pair().1, target_id(), true),
            (pair().0, pair().1, target_id(), false)
        ]
    );

    let malformed = format!("{}/not-a-person", requests_uri());
    for (uri, body, origin, status, code) in [
        (
            &uri,
            json!({ "decision": "maybe" }),
            true,
            StatusCode::BAD_REQUEST,
            "sub_room.invalid_join_request_answer",
        ),
        (
            &uri,
            json!({ "decision": "approve", "message": "welcome" }),
            true,
            StatusCode::BAD_REQUEST,
            "sub_room.invalid_join_request_answer",
        ),
        (
            &malformed,
            json!({ "decision": "approve" }),
            true,
            StatusCode::BAD_REQUEST,
            "private_room.invalid_resource_id",
        ),
        (
            &uri,
            json!({ "decision": "approve" }),
            false,
            StatusCode::FORBIDDEN,
            "private_room.invalid_origin",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(sub_request(Method::PUT, uri, &body, origin, None))
            .await
            .expect("路由可调用");
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
    assert_eq!(
        sub_rooms.answered.lock().unwrap().len(),
        2,
        "被拒的没到用例"
    );
}

#[tokio::test]
async fn 被拒以后24小时内再申请_回冲突带着要等多久() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("时钟在 1970 年以后")
        .as_millis();
    let retry_at = i64::try_from(now).expect("毫秒数放得下") + 2 * 60 * 60 * 1000;
    let sub_rooms = Arc::new(FakeSubRooms::default());
    *sub_rooms.failure.lock().unwrap() = Some(
        PrivateRoomFailure::new(
            "test",
            PrivateRoomFailureStage::Validation,
            PrivateRoomFailureKind::SubRoom(SubRoomRefusal::RecentlyRejected),
        )
        .retrying_at(time(retry_at)),
    );

    let response = sub_room_router(sub_rooms, Arc::default())
        .oneshot(bare_request(true))
        .await
        .expect("申请的路由可调用");

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let seconds: u64 = response.headers()[header::RETRY_AFTER]
        .to_str()
        .expect("是数字")
        .parse()
        .expect("是秒数");
    assert!((7_100..=7_200).contains(&seconds), "{seconds}");
    assert_eq!(
        response_json(response).await["code"],
        "sub_room.recently_rejected"
    );
}
