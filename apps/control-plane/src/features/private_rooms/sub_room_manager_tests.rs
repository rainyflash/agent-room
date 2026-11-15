//! 子房间管理员（`specs/redesign/sub-rooms.md` 第 2d-4 步）的接口。规则在用例里，这里看路由把什么
//! 交给了用例、回答长什么样。

use super::*;

fn manager_uri(principal: &str) -> String {
    format!("/private-rooms/{CATALOG_UUID}/sub-rooms/{SUB_ROOM_UUID}/managers/{principal}")
}

#[tokio::test]
async fn 设和撤同一个地址_都不回内容_要不要最近登录由用例定() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let authentication = Arc::new(FakeAuthentication::default());
    let app = sub_room_router(sub_rooms.clone(), authentication.clone());

    for method in [Method::PUT, Method::DELETE] {
        let response = app
            .clone()
            .oneshot(sub_request(
                method.clone(),
                &manager_uri(TARGET_UUID),
                &json!({}),
                true,
                None,
            ))
            .await
            .expect("路由可调用");
        assert_eq!(response.status(), StatusCode::NO_CONTENT, "{method}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    let changed = sub_rooms.managers_changed.lock().unwrap().clone();
    assert_eq!(
        changed
            .iter()
            .map(|request| (
                request.parent_catalog_id,
                request.catalog_id,
                request.target,
                request.grant
            ))
            .collect::<Vec<_>>(),
        [true, false].map(|grant| (
            catalog_id(),
            RoomCatalogId::from_uuid(uuid(SUB_ROOM_UUID)),
            target_id(),
            grant
        ))
    );
    assert_eq!(
        authentication.calls.lock().unwrap().as_slice(),
        &[AuthenticationRequirement::ActiveSession; 2],
        "自己不当了不用重新登录，所以路由只要有效登录"
    );
}

#[tokio::test]
async fn 他还没进这一间_或者本来就管每一间时说清楚() {
    for (refusal, status, code) in [
        (
            SubRoomRefusal::TargetNotInside,
            StatusCode::CONFLICT,
            "sub_room.target_not_inside",
        ),
        (
            SubRoomRefusal::ManagedByParent,
            StatusCode::CONFLICT,
            "sub_room.managed_by_parent",
        ),
        (
            SubRoomRefusal::PeerManager,
            StatusCode::FORBIDDEN,
            "sub_room.peer_manager",
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
                &manager_uri(TARGET_UUID),
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

#[tokio::test]
async fn 不是账户号的不收() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let response = sub_room_router(sub_rooms.clone(), Arc::default())
        .oneshot(sub_request(
            Method::PUT,
            &manager_uri("not-a-person"),
            &json!({}),
            true,
            None,
        ))
        .await
        .expect("路由可调用");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(sub_rooms.managers_changed.lock().unwrap().is_empty());
}
