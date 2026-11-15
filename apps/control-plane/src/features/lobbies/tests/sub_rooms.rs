//! 子房间第 2f-1b 步（`specs/redesign/sub-rooms.md` 的“接口”）：带 `subRooms=1` 列能进的房间时，
//! 每间私人房间后面跟着它下面的子房间，写着在哪间房下面、哪个分类、哪种门；签名签的是带查询的
//! 整个地址。不带查询的照旧只有私人房间，别的查询不收、不去认证设备。

use agent_room_application::{
    ports::SubRoomEntry,
    private_rooms::{AccessibleSubRoom, AccountSubRoomListing},
};
use agent_room_domain::{private_rooms::PrivateRoomMembershipStatus, room_listing::RoomJoinMode};

use super::*;

const SUB_ROOM_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e51";
const OTHER_SUB_ROOM_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e52";

/// 每间父房间下面摆两间子房间，记下问了谁、哪几间父房间。
#[derive(Default)]
pub(super) struct FakeSubRoomListing {
    calls: Mutex<Vec<(PrincipalId, Vec<RoomCatalogId>)>>,
}

impl AccountSubRoomListing for FakeSubRoomListing {
    fn accessible_sub_rooms<'a>(
        &'a self,
        principal_id: PrincipalId,
        parents: &'a [PrivateRoomSnapshot],
    ) -> PortFuture<'a, PrivateRoomResult<Vec<AccessibleSubRoom>>> {
        let parent_ids = parents
            .iter()
            .map(|snapshot| snapshot.catalog().id())
            .collect();
        self.calls
            .lock()
            .expect("子房间记录锁可用")
            .push((principal_id, parent_ids));
        let sub_rooms = parents
            .iter()
            .flat_map(|snapshot| {
                let mut security = sub_room(snapshot, OTHER_SUB_ROOM_UUID, "security", None);
                security.entry.access = RoomJoinMode::Request;
                security.entry.viewer_status = None;
                [
                    sub_room(snapshot, SUB_ROOM_UUID, "release", Some("Ship")),
                    security,
                ]
            })
            .collect();
        Box::pin(async move { Ok(sub_rooms) })
    }
}

fn sub_room(
    parent: &PrivateRoomSnapshot,
    catalog: &str,
    name: &str,
    category: Option<&str>,
) -> AccessibleSubRoom {
    AccessibleSubRoom {
        parent_catalog_id: parent.catalog().id(),
        parent_name: parent.catalog().name().to_owned(),
        entry: SubRoomEntry {
            catalog_id: RoomCatalogId::from_uuid(uuid(catalog)),
            matrix_room: MatrixRoomReference::new(format!("!{name}:matrix.agent-room.test"))
                .expect("Matrix 房间标识有效"),
            name: name.to_owned(),
            description: String::new(),
            access: RoomJoinMode::Open,
            hidden: false,
            category_id: None,
            archived: false,
            archive_record: None,
            created_by: principal_id(),
            created_at: time(1_700_000_000_000),
            managers: Vec::new(),
            viewer_status: Some(PrivateRoomMembershipStatus::Joined),
            member_count: 2,
            agent_count: 1,
            last_message_at: None,
            viewer_requested_at: None,
            pending_requests: 0,
        },
        category: category.map(str::to_owned),
    }
}

/// 照设备签名的样子发一次列房间；`devices` 核对签的目标就是 `uri`。
async fn list_signed(
    uri: &str,
    devices: &Arc<FakeDevices>,
    sub_rooms: &Arc<FakeSubRoomListing>,
) -> axum::response::Response {
    *devices
        .expected_request
        .lock()
        .expect("设备请求目标记录锁可用") = Some(("GET".to_owned(), uri.to_owned()));
    *devices
        .expected_body
        .lock()
        .expect("设备请求正文记录锁可用") = Some(String::new());
    let app = test_router_parts(
        Arc::new(FakeEntries::default()),
        devices.clone(),
        None,
        sub_rooms.clone(),
    );
    let mut request = Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::AUTHORIZATION, "Bearer device-access-token")
        .header("x-agent-room-device-id", DEVICE_UUID)
        .header("x-agent-room-proof-issued-at", "1700000000000")
        .header("x-agent-room-proof-nonce", "nonce-0123456789abcdef")
        .header(
            "x-agent-room-proof-signature",
            URL_SAFE_NO_PAD.encode([9_u8; 64]),
        )
        .body(Body::empty())
        .expect("列房间请求有效");
    request.headers_mut().remove(header::CONTENT_TYPE);
    app.oneshot(request).await.expect("列房间可调用")
}

#[tokio::test]
async fn 带_sub_rooms_的_每间房后面跟着它下面的子房间_写着父房间_分类和门() {
    let devices = Arc::new(FakeDevices::default());
    let sub_rooms = Arc::new(FakeSubRoomListing::default());

    let response = list_signed("/lobbies/accessible?subRooms=1", &devices, &sub_rooms).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(devices.authentications.load(Ordering::SeqCst), 1);
    let parent = json!({ "catalogId": CATALOG_UUID, "name": "Incident Room" });
    assert_eq!(
        response_json(response).await,
        json!({
            "rooms": [
                {
                    "kind": "private_room",
                    "catalogId": CATALOG_UUID,
                    "matrixRoomId": "!private:matrix.agent-room.test",
                    "name": "Incident Room",
                    "membership": "joined"
                },
                {
                    "kind": "private_room",
                    "catalogId": SUB_ROOM_UUID,
                    "matrixRoomId": "!release:matrix.agent-room.test",
                    "name": "release",
                    "membership": "joined",
                    "parent": parent,
                    "category": "Ship",
                    "access": "open"
                },
                {
                    "kind": "private_room",
                    "catalogId": OTHER_SUB_ROOM_UUID,
                    "matrixRoomId": "!security:matrix.agent-room.test",
                    "name": "security",
                    "parent": parent,
                    "access": "request"
                }
            ]
        }),
        "没进去的不写成员状态，未分类的不写分类，子房间没有分享码"
    );
    assert_eq!(
        sub_rooms.calls.lock().expect("子房间记录锁可用").as_slice(),
        &[(principal_id(), vec![catalog_id()])],
        "问的是设备账号进得去的那几间"
    );
}

#[tokio::test]
async fn 不带查询的照旧只有私人房间_不去问子房间() {
    let devices = Arc::new(FakeDevices::default());
    let sub_rooms = Arc::new(FakeSubRoomListing::default());

    let response = list_signed("/lobbies/accessible", &devices, &sub_rooms).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["rooms"].as_array().map(Vec::len), Some(1));
    assert!(body["rooms"][0].get("parent").is_none());
    assert!(sub_rooms.calls.lock().expect("子房间记录锁可用").is_empty());
}

#[tokio::test]
async fn 别的查询不收_也不去认证设备() {
    for uri in [
        "/lobbies/accessible?subRooms=0",
        "/lobbies/accessible?subRooms=true",
        "/lobbies/accessible?subRooms=1&subRooms=1",
        "/lobbies/accessible?other=1",
        "/lobbies/accessible?",
    ] {
        let devices = Arc::new(FakeDevices::default());
        let sub_rooms = Arc::new(FakeSubRoomListing::default());
        let response = list_signed(uri, &devices, &sub_rooms).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(response_json(response).await["code"], "lobby.invalid_query");
        assert_eq!(devices.authentications.load(Ordering::SeqCst), 0, "{uri}");
        assert!(sub_rooms.calls.lock().expect("子房间记录锁可用").is_empty());
    }
}
