//! 子房间的请进来、移出和封禁（`specs/redesign/sub-rooms.md` 第 2d-3 步）的接口。规则在用例里，这里
//! 看路由把什么交给了用例、回答长什么样。

use super::*;

fn member_uri(principal: &str) -> String {
    format!("/private-rooms/{CATALOG_UUID}/sub-rooms/{SUB_ROOM_UUID}/members/{principal}")
}

#[tokio::test]
async fn 请进来_移出_封禁_解封各一个地址_都不回内容_只要有效登录() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let authentication = Arc::new(FakeAuthentication::default());
    let app = sub_room_router(sub_rooms.clone(), authentication.clone());
    let member = member_uri(TARGET_UUID);

    for (method, uri) in [
        (Method::PUT, member.clone()),
        (Method::DELETE, member.clone()),
        (Method::POST, format!("{member}/ban")),
        (Method::POST, format!("{member}/unban")),
    ] {
        let response = app
            .clone()
            .oneshot(sub_request(method.clone(), &uri, &json!({}), true, None))
            .await
            .expect("路由可调用");
        assert_eq!(response.status(), StatusCode::NO_CONTENT, "{method} {uri}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert!(
            to_bytes(response.into_body(), 64)
                .await
                .expect("读得出回答")
                .is_empty()
        );
    }
    let governed = sub_rooms.governed.lock().unwrap().clone();
    assert_eq!(
        governed
            .iter()
            .map(|(action, request)| (
                *action,
                request.parent_catalog_id,
                request.catalog_id,
                request.target
            ))
            .collect::<Vec<_>>(),
        ["admit", "remove", "ban", "unban"].map(|action| (
            action,
            catalog_id(),
            RoomCatalogId::from_uuid(uuid(SUB_ROOM_UUID)),
            target_id()
        ))
    );
    assert_eq!(
        authentication.calls.lock().unwrap().as_slice(),
        &[AuthenticationRequirement::ActiveSession; 4],
        "和整间房的移出、封禁一样只要有效登录"
    );
}

#[tokio::test]
async fn 要同源_账户和房间号不对的不收() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let app = sub_room_router(sub_rooms.clone(), Arc::default());
    let wrong_room =
        format!("/private-rooms/{CATALOG_UUID}/sub-rooms/not-a-room/members/{TARGET_UUID}");

    for (uri, origin, status, code) in [
        (
            member_uri(TARGET_UUID),
            false,
            StatusCode::FORBIDDEN,
            "private_room.invalid_origin",
        ),
        (
            member_uri("not-a-person"),
            true,
            StatusCode::BAD_REQUEST,
            "private_room.invalid_resource_id",
        ),
        (
            wrong_room,
            true,
            StatusCode::BAD_REQUEST,
            "private_room.invalid_resource_id",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(sub_request(Method::PUT, &uri, &json!({}), origin, None))
            .await
            .expect("路由可调用");
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
    assert!(
        sub_rooms.governed.lock().unwrap().is_empty(),
        "被拒的没到用例"
    );
}

#[tokio::test]
async fn 动不了父房间的管理员时回只有房主能动() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    *sub_rooms.failure.lock().unwrap() = Some(PrivateRoomFailure::new(
        "test",
        PrivateRoomFailureStage::Validation,
        PrivateRoomFailureKind::OwnerOnly,
    ));

    let response = sub_room_router(sub_rooms, Arc::default())
        .oneshot(sub_request(
            Method::DELETE,
            &member_uri(TARGET_UUID),
            &json!({}),
            true,
            None,
        ))
        .await
        .expect("路由可调用");

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(response).await["code"],
        "private_room.owner_only"
    );
}
