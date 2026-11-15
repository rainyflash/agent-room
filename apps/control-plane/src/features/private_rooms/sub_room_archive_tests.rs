//! 子房间的归档和恢复（`specs/redesign/sub-rooms.md` 第 2e-2 步）的接口。规则在用例里，这里看路由
//! 把什么交给了用例、回答长什么样、被拒时回哪个码。

use super::*;

fn sub_room_uri() -> String {
    format!("/private-rooms/{CATALOG_UUID}/sub-rooms/{SUB_ROOM_UUID}")
}

#[tokio::test]
async fn 归档是删这一间_恢复另一个地址_都不回内容_只要有效登录() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let authentication = Arc::new(FakeAuthentication::default());
    let app = sub_room_router(sub_rooms.clone(), authentication.clone());

    for (method, uri) in [
        (Method::DELETE, sub_room_uri()),
        (Method::POST, format!("{}/restore", sub_room_uri())),
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
    let sub_room = RoomCatalogId::from_uuid(uuid(SUB_ROOM_UUID));
    assert_eq!(
        sub_rooms.archival.lock().unwrap().as_slice(),
        &[
            ("archive", catalog_id(), sub_room),
            ("restore", catalog_id(), sub_room),
        ]
    );
    assert_eq!(
        authentication.calls.lock().unwrap().as_slice(),
        &[AuthenticationRequirement::ActiveSession; 2],
        "和归档整间房一样确认一下就做，不要最近登录过"
    );
}

#[tokio::test]
async fn 不是同源_房间号不像样的没到用例() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let app = sub_room_router(sub_rooms.clone(), Arc::default());
    for (request, status, code) in [
        (
            sub_request(Method::DELETE, &sub_room_uri(), &json!({}), false, None),
            StatusCode::FORBIDDEN,
            "private_room.invalid_origin",
        ),
        (
            sub_request(
                Method::POST,
                &format!("/private-rooms/{CATALOG_UUID}/sub-rooms/not-a-room/restore"),
                &json!({}),
                true,
                None,
            ),
            StatusCode::BAD_REQUEST,
            "private_room.invalid_resource_id",
        ),
    ] {
        let response = app.clone().oneshot(request).await.expect("路由可调用");
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
    assert!(sub_rooms.archival.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 管不了_归档了_撞了名字_满了_别人刚改过时说清楚() {
    for (kind, status, code) in [
        (
            PrivateRoomFailureKind::SubRoom(SubRoomRefusal::ManagersOnly),
            StatusCode::FORBIDDEN,
            "sub_room.managers_only",
        ),
        (
            PrivateRoomFailureKind::SubRoom(SubRoomRefusal::Archived),
            StatusCode::CONFLICT,
            "sub_room.archived",
        ),
        (
            PrivateRoomFailureKind::SubRoom(SubRoomRefusal::NameTaken),
            StatusCode::CONFLICT,
            "sub_room.name_taken",
        ),
        (
            PrivateRoomFailureKind::SubRoom(SubRoomRefusal::LimitReached),
            StatusCode::CONFLICT,
            "sub_room.limit_reached",
        ),
        (
            PrivateRoomFailureKind::Conflict,
            StatusCode::CONFLICT,
            "private_room.conflict",
        ),
    ] {
        let sub_rooms = Arc::new(FakeSubRooms::default());
        *sub_rooms.failure.lock().unwrap() = Some(PrivateRoomFailure::new(
            "test",
            PrivateRoomFailureStage::Persistence,
            kind,
        ));
        let response = sub_room_router(sub_rooms, Arc::default())
            .oneshot(sub_request(
                Method::POST,
                &format!("{}/restore", sub_room_uri()),
                &json!({}),
                true,
                None,
            ))
            .await
            .expect("路由可调用");
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
}
