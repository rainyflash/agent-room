//! 子房间的分类（`specs/redesign/sub-rooms.md` 第 2e-1 步）的接口。规则在用例里，这里看路由把什么
//! 交给了用例、回答长什么样、被拒时回哪个码。

use super::*;

fn categories_uri() -> String {
    format!("/private-rooms/{CATALOG_UUID}/sub-room-categories")
}

fn category_uri(category: &str) -> String {
    format!("/private-rooms/{CATALOG_UUID}/sub-room-categories/{category}")
}

fn expected_categories() -> Value {
    json!({
        "categories": [
            { "categoryId": CATEGORY_UUID, "name": "设计", "position": 0 },
            { "categoryId": OTHER_CATEGORY_UUID, "name": "Release", "position": 1 }
        ]
    })
}

#[tokio::test]
async fn 建改删都回这间房的分类_建要幂等键_都只要有效登录() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let authentication = Arc::new(FakeAuthentication::default());
    let app = sub_room_router(sub_rooms.clone(), authentication.clone());

    for (request, status) in [
        (
            sub_request(
                Method::POST,
                &categories_uri(),
                &json!({ "name": "设计" }),
                true,
                Some(CATEGORY_UUID),
            ),
            StatusCode::CREATED,
        ),
        (
            sub_request(
                Method::PUT,
                &category_uri(CATEGORY_UUID),
                &json!({ "name": "Design", "position": 1 }),
                true,
                None,
            ),
            StatusCode::OK,
        ),
        (
            sub_request(
                Method::PUT,
                &category_uri(CATEGORY_UUID),
                &json!({ "position": 0 }),
                true,
                None,
            ),
            StatusCode::OK,
        ),
        (
            sub_request(
                Method::DELETE,
                &category_uri(OTHER_CATEGORY_UUID),
                &json!({}),
                true,
                None,
            ),
            StatusCode::OK,
        ),
    ] {
        let response = app.clone().oneshot(request).await.expect("路由可调用");
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(response_json(response).await, expected_categories());
    }

    let category = SubRoomCategoryId::from_uuid(uuid(CATEGORY_UUID));
    let other = SubRoomCategoryId::from_uuid(uuid(OTHER_CATEGORY_UUID));
    assert_eq!(
        sub_rooms.categories_changed.lock().unwrap().as_slice(),
        &[
            (
                "create",
                catalog_id(),
                category,
                Some("设计".to_owned()),
                None
            ),
            (
                "revise",
                catalog_id(),
                category,
                Some("Design".to_owned()),
                Some(1)
            ),
            ("revise", catalog_id(), category, None, Some(0)),
            ("delete", catalog_id(), other, None, None),
        ],
        "名字原样交给用例，去空白、查规矩是用例的事"
    );
    assert_eq!(
        authentication.calls.lock().unwrap().as_slice(),
        &[AuthenticationRequirement::ActiveSession; 4],
        "分类只管怎么摆，不要最近登录过"
    );
}

#[tokio::test]
async fn 不收的请求_没到用例() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let app = sub_room_router(sub_rooms.clone(), Arc::default());
    for (request, status, code) in [
        (
            sub_request(
                Method::POST,
                &categories_uri(),
                &json!({ "name": "设计" }),
                true,
                None,
            ),
            StatusCode::BAD_REQUEST,
            "private_room.invalid_idempotency_key",
        ),
        (
            sub_request(
                Method::POST,
                &categories_uri(),
                &json!({ "name": "设计" }),
                true,
                Some("not-an-id"),
            ),
            StatusCode::BAD_REQUEST,
            "private_room.invalid_idempotency_key",
        ),
        (
            sub_request(
                Method::POST,
                &categories_uri(),
                &json!({ "name": "设计", "position": 0 }),
                true,
                Some(CATEGORY_UUID),
            ),
            StatusCode::BAD_REQUEST,
            "sub_room.invalid_category_body",
        ),
        (
            sub_request(
                Method::PUT,
                &category_uri(CATEGORY_UUID),
                &json!({}),
                true,
                None,
            ),
            StatusCode::BAD_REQUEST,
            "sub_room.invalid_category_body",
        ),
        (
            sub_request(
                Method::PUT,
                &category_uri(CATEGORY_UUID),
                &json!({ "position": 256 }),
                true,
                None,
            ),
            StatusCode::BAD_REQUEST,
            "sub_room.invalid_category_body",
        ),
        (
            sub_request(
                Method::PUT,
                &category_uri("not-a-category"),
                &json!({ "name": "设计" }),
                true,
                None,
            ),
            StatusCode::BAD_REQUEST,
            "private_room.invalid_resource_id",
        ),
        (
            sub_request(
                Method::DELETE,
                &category_uri("not-a-category"),
                &json!({}),
                true,
                None,
            ),
            StatusCode::BAD_REQUEST,
            "private_room.invalid_resource_id",
        ),
        (
            sub_request(
                Method::DELETE,
                &category_uri(CATEGORY_UUID),
                &json!({}),
                false,
                None,
            ),
            StatusCode::FORBIDDEN,
            "private_room.invalid_origin",
        ),
    ] {
        let response = app.clone().oneshot(request).await.expect("路由可调用");
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
    assert!(
        sub_rooms.categories_changed.lock().unwrap().is_empty(),
        "被拒的没到用例"
    );
}

#[tokio::test]
async fn 不是房主和管理员_重名_到上限_分类没了时说清楚() {
    for (refusal, status, code) in [
        (
            SubRoomRefusal::CategoryAdminsOnly,
            StatusCode::FORBIDDEN,
            "sub_room.category_admins_only",
        ),
        (
            SubRoomRefusal::CategoryNameTaken,
            StatusCode::CONFLICT,
            "sub_room.category_name_taken",
        ),
        (
            SubRoomRefusal::CategoryLimitReached,
            StatusCode::CONFLICT,
            "sub_room.category_limit_reached",
        ),
        (
            SubRoomRefusal::CategoryMissing,
            StatusCode::CONFLICT,
            "sub_room.category_missing",
        ),
    ] {
        let sub_rooms = Arc::new(FakeSubRooms::default());
        *sub_rooms.failure.lock().unwrap() = Some(PrivateRoomFailure::new(
            "test",
            PrivateRoomFailureStage::Validation,
            PrivateRoomFailureKind::SubRoom(refusal),
        ));
        let response = sub_room_router(sub_rooms, Arc::default())
            .oneshot(sub_request(
                Method::PUT,
                &category_uri(CATEGORY_UUID),
                &json!({ "name": "设计" }),
                true,
                None,
            ))
            .await
            .expect("路由可调用");
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
}
