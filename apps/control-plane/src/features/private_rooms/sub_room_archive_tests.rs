//! 子房间的归档和恢复（`specs/redesign/sub-rooms.md` 第 2e-2 步）和“已归档”的名单（2e-3）的接口。
//! 规则在用例里，这里看路由把什么交给了用例、回答长什么样、被拒时回哪个码。

use agent_room_application::ports::SubRoomArchiveRecord;
use agent_room_domain::sub_room_archive::SubRoomArchiveReason;

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

/// “已归档”的名单：一间是目标收起来的、看的人管着它；一间是 30 天没人说话自动收起来的、看的人
/// 没进过。
pub(super) fn archived_list() -> SubRoomList {
    let mut listed = list();
    let template = listed.sub_rooms.remove(0).entry;
    listed.sub_rooms = vec![
        SubRoomView {
            entry: SubRoomEntry {
                archived: true,
                archive_record: Some(SubRoomArchiveRecord {
                    at: time(ARCHIVED_AT),
                    by: Some(target_id()),
                    reason: SubRoomArchiveReason::Manual,
                }),
                pending_requests: 0,
                ..template.clone()
            },
            inside_visible: true,
            manages: true,
            restorable: true,
        },
        SubRoomView {
            entry: SubRoomEntry {
                catalog_id: RoomCatalogId::from_uuid(uuid(OTHER_SUB_ROOM_UUID)),
                name: "notes".to_owned(),
                description: String::new(),
                category_id: None,
                archived: true,
                archive_record: Some(SubRoomArchiveRecord {
                    at: time(ARCHIVED_AT - 1_000),
                    by: None,
                    reason: SubRoomArchiveReason::Inactive,
                }),
                managers: Vec::new(),
                viewer_status: None,
                ..template
            },
            inside_visible: true,
            manages: false,
            restorable: false,
        },
    ];
    listed
}

const ARCHIVED_AT: i64 = 1_700_000_200_000;

fn list_uri(query: &str) -> String {
    format!("/private-rooms/{CATALOG_UUID}/sub-rooms{query}")
}

#[tokio::test]
async fn 已归档的名单带着什么时候_谁_为什么归档的_看的人能不能恢复() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let response = sub_room_router(sub_rooms.clone(), Arc::default())
        .oneshot(sub_request(
            Method::GET,
            &list_uri("?archived=1"),
            &json!({}),
            false,
            None,
        ))
        .await
        .expect("名单路由可调用");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(
        body["subRooms"],
        json!([
            {
                "catalogId": SUB_ROOM_UUID,
                "name": "release",
                "description": "Ship Alpha 95",
                "access": "open",
                "hidden": false,
                "categoryId": OTHER_CATEGORY_UUID,
                "createdByPrincipalId": TARGET_UUID,
                "createdAtUnixMs": 1_700_000_000_000_i64,
                "managerPrincipalIds": [TARGET_UUID],
                "joined": true,
                "banned": false,
                "memberCount": 3,
                "agentCount": 2,
                "lastMessageAtUnixMs": 1_700_000_100_000_i64,
                "pendingRequestCount": 0,
                "archivedAtUnixMs": ARCHIVED_AT,
                "archivedByPrincipalId": TARGET_UUID,
                "archivedReason": "manual",
                "canRestore": true
            },
            {
                "catalogId": OTHER_SUB_ROOM_UUID,
                "name": "notes",
                "description": "",
                "access": "open",
                "hidden": false,
                "createdByPrincipalId": TARGET_UUID,
                "createdAtUnixMs": 1_700_000_000_000_i64,
                "managerPrincipalIds": [],
                "joined": false,
                "banned": false,
                "memberCount": 3,
                "agentCount": 2,
                "lastMessageAtUnixMs": 1_700_000_100_000_i64,
                "archivedAtUnixMs": ARCHIVED_AT - 1_000,
                "archivedReason": "inactive",
                "canRestore": false
            }
        ]),
        "自动归档的没有谁归档的"
    );
    assert_eq!(body["parent"]["catalogId"], CATALOG_UUID);
    assert_eq!(
        sub_rooms.listed.lock().unwrap().as_slice(),
        &[(catalog_id(), true)]
    );
}

#[tokio::test]
async fn 名单的_archived_只认_0_和_1_别的不收() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let app = sub_room_router(sub_rooms.clone(), Arc::default());
    let response = app
        .clone()
        .oneshot(sub_request(
            Method::GET,
            &list_uri("?archived=0"),
            &json!({}),
            false,
            None,
        ))
        .await
        .expect("名单路由可调用");
    assert_eq!(response.status(), StatusCode::OK);
    for query in ["?archived=yes", "?archived=", "?archived=1&archived=0"] {
        let response = app
            .clone()
            .oneshot(sub_request(
                Method::GET,
                &list_uri(query),
                &json!({}),
                false,
                None,
            ))
            .await
            .expect("名单路由可调用");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(
            response_json(response).await["code"],
            "private_room.invalid_request"
        );
    }
    assert_eq!(
        sub_rooms.listed.lock().unwrap().as_slice(),
        &[(catalog_id(), false)],
        "不收的没到用例"
    );
}
