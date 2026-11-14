//! 子房间的接口（`specs/redesign/sub-rooms.md` 第 2a 步）：建、读名单、三项设置。规则在用例里，
//! 这里看路由把什么交给了用例、回答长什么样、被拒时回哪个码。

use agent_room_application::{
    ports::SubRoomEntry,
    private_rooms::{
        ChangeSubRoomPolicy, CreateSubRoom, InspectSubRoomPolicy, ListSubRooms, SubRoomList,
        SubRoomPolicyView, SubRoomRefusal, SubRoomUseCases, SubRoomView,
    },
};
use agent_room_domain::{
    private_rooms::PrivateRoomMembershipStatus,
    sub_rooms::{SubRoomCreators, SubRoomPolicy},
};

use super::*;

const SUB_ROOM_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e50";
const OTHER_SUB_ROOM_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e51";

/// 记下交过来的请求，照设好的回答。
#[derive(Default)]
pub(super) struct FakeSubRooms {
    created: Mutex<Vec<CreateSubRoom>>,
    listed: Mutex<Vec<RoomCatalogId>>,
    inspected: Mutex<Vec<RoomCatalogId>>,
    changed: Mutex<Vec<(RoomCatalogId, SubRoomPolicy)>>,
    /// 设上时每个用例都回答它。
    failure: Mutex<Option<PrivateRoomFailure>>,
}

impl FakeSubRooms {
    fn refusal(&self) -> Option<PrivateRoomFailure> {
        *self.failure.lock().unwrap()
    }
}

impl SubRoomUseCases for FakeSubRooms {
    fn create_sub_room(
        &self,
        request: CreateSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        let result = self.refusal().map_or_else(|| Ok(snapshot()), Err);
        self.created.lock().unwrap().push(request);
        Box::pin(async move { result })
    }

    fn list_sub_rooms(
        &self,
        request: ListSubRooms,
    ) -> PortFuture<'_, PrivateRoomResult<SubRoomList>> {
        self.listed.lock().unwrap().push(request.catalog_id);
        let result = self.refusal().map_or_else(|| Ok(list()), Err);
        Box::pin(async move { result })
    }

    fn sub_room_policy(
        &self,
        request: InspectSubRoomPolicy,
    ) -> PortFuture<'_, PrivateRoomResult<SubRoomPolicyView>> {
        self.inspected
            .lock()
            .unwrap()
            .push(request.parent_catalog_id);
        let result = self.refusal().map_or_else(
            || {
                Ok(SubRoomPolicyView {
                    policy: SubRoomPolicy::DEFAULT,
                    can_change: false,
                })
            },
            Err,
        );
        Box::pin(async move { result })
    }

    fn change_sub_room_policy(
        &self,
        request: ChangeSubRoomPolicy,
    ) -> PortFuture<'_, PrivateRoomResult<SubRoomPolicyView>> {
        self.changed
            .lock()
            .unwrap()
            .push((request.parent_catalog_id, request.policy));
        let result = self.refusal().map_or_else(
            || {
                Ok(SubRoomPolicyView {
                    policy: request.policy,
                    can_change: true,
                })
            },
            Err,
        );
        Box::pin(async move { result })
    }
}

#[tokio::test]
async fn 建子房间要同源会话和幂等键_名字去掉两头的空白_回的是建好的这一间() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let authentication = Arc::new(FakeAuthentication::default());
    let app = sub_room_router(sub_rooms.clone(), authentication.clone());
    let uri = format!("/private-rooms/{CATALOG_UUID}/sub-rooms");
    let body = json!({ "name": "  release ", "description": "Ship Alpha 95" });

    let created = app
        .clone()
        .oneshot(sub_request(
            Method::POST,
            &uri,
            &body,
            true,
            Some(SUB_ROOM_UUID),
        ))
        .await
        .expect("建子房间的路由可调用");
    assert_eq!(created.status(), StatusCode::CREATED);
    assert_eq!(created.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response_json(created).await["name"], "Incident Room");
    let requests = sub_rooms.created.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].parent_catalog_id, catalog_id());
    assert_eq!(
        requests[0].catalog_id,
        RoomCatalogId::from_uuid(uuid(SUB_ROOM_UUID)),
        "幂等键就是子房间的房间号"
    );
    assert_eq!(requests[0].name, "release");
    assert_eq!(requests[0].description, "Ship Alpha 95");
    assert_eq!(requests[0].created_by_agent, None);
    assert!(matches!(requests[0].actor, PrivateRoomActor::Person(_)));
    assert_eq!(
        authentication.calls.lock().unwrap().as_slice(),
        &[AuthenticationRequirement::ActiveSession]
    );

    for (request, status, code) in [
        (
            sub_request(Method::POST, &uri, &body, false, Some(SUB_ROOM_UUID)),
            StatusCode::FORBIDDEN,
            "private_room.invalid_origin",
        ),
        (
            sub_request(Method::POST, &uri, &body, true, None),
            StatusCode::BAD_REQUEST,
            "private_room.invalid_idempotency_key",
        ),
        (
            // 门和分类还没有：带了不认识的字段就拒，免得人以为装上了门。
            sub_request(
                Method::POST,
                &uri,
                &json!({ "name": "release", "access": "request" }),
                true,
                Some(SUB_ROOM_UUID),
            ),
            StatusCode::BAD_REQUEST,
            "sub_room.invalid_creation_body",
        ),
    ] {
        let response = app.clone().oneshot(request).await.expect("路由可调用");
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response_json(response).await["code"], code);
    }
    assert_eq!(sub_rooms.created.lock().unwrap().len(), 1, "被拒的没到用例");
}

#[tokio::test]
async fn 名单只要有效登录_有门又没进的不给人数和最近说话的时间() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let authentication = Arc::new(FakeAuthentication::default());
    let response = sub_room_router(sub_rooms.clone(), authentication.clone())
        .oneshot(sub_request(
            Method::GET,
            &format!("/private-rooms/{SUB_ROOM_UUID}/sub-rooms"),
            &json!({}),
            false,
            None,
        ))
        .await
        .expect("名单路由可调用");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        response_json(response).await,
        json!({
            "parent": { "catalogId": CATALOG_UUID, "name": "Incident Room" },
            "canCreate": true,
            "subRooms": [
                {
                    "catalogId": SUB_ROOM_UUID,
                    "name": "release",
                    "description": "Ship Alpha 95",
                    "access": "open",
                    "hidden": false,
                    "createdByPrincipalId": TARGET_UUID,
                    "createdAtUnixMs": 1_700_000_000_000_i64,
                    "managerPrincipalIds": [TARGET_UUID],
                    "joined": true,
                    "banned": false,
                    "memberCount": 3,
                    "agentCount": 2,
                    "lastMessageAtUnixMs": 1_700_000_100_000_i64
                },
                {
                    "catalogId": OTHER_SUB_ROOM_UUID,
                    "name": "security",
                    "description": "",
                    "access": "invite_only",
                    "hidden": true,
                    "createdByPrincipalId": OWNER_UUID,
                    "createdAtUnixMs": 1_700_000_000_000_i64,
                    "managerPrincipalIds": [],
                    "joined": false,
                    "banned": true
                }
            ]
        })
    );
    assert_eq!(
        sub_rooms.listed.lock().unwrap().as_slice(),
        &[RoomCatalogId::from_uuid(uuid(SUB_ROOM_UUID))],
        "地址里是子房间的房间号也行，原样交给用例"
    );
    assert_eq!(
        authentication.calls.lock().unwrap().as_slice(),
        &[AuthenticationRequirement::ActiveSession]
    );
}

#[tokio::test]
async fn 三项设置能读能改_改要同源_多余的字段和不认识的值不收() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let app = sub_room_router(sub_rooms.clone(), Arc::default());
    let uri = format!("/private-rooms/{CATALOG_UUID}/sub-room-policy");

    let read = app
        .clone()
        .oneshot(sub_request(Method::GET, &uri, &json!({}), false, None))
        .await
        .expect("读设置的路由可调用");
    assert_eq!(read.status(), StatusCode::OK);
    assert_eq!(
        response_json(read).await,
        json!({
            "creators": "members",
            "agentsMayCreate": true,
            "autoArchive": true,
            "canChange": false
        })
    );

    let strict = json!({ "creators": "admins", "agentsMayCreate": false, "autoArchive": false });
    let changed = app
        .clone()
        .oneshot(sub_request(Method::PUT, &uri, &strict, true, None))
        .await
        .expect("改设置的路由可调用");
    assert_eq!(changed.status(), StatusCode::OK);
    assert_eq!(response_json(changed).await["creators"], "admins");
    assert_eq!(
        sub_rooms.changed.lock().unwrap().as_slice(),
        &[(
            catalog_id(),
            SubRoomPolicy {
                creators: SubRoomCreators::Admins,
                agents_may_create: false,
                auto_archive: false,
            }
        )]
    );

    for (body, origin, status, code) in [
        (
            strict.clone(),
            false,
            StatusCode::FORBIDDEN,
            "private_room.invalid_origin",
        ),
        (
            json!({ "creators": "everyone", "agentsMayCreate": true, "autoArchive": true }),
            true,
            StatusCode::BAD_REQUEST,
            "sub_room.invalid_policy_body",
        ),
        (
            json!({ "creators": "admins", "agentsMayCreate": true }),
            true,
            StatusCode::BAD_REQUEST,
            "sub_room.invalid_policy_body",
        ),
        (
            json!({
                "creators": "admins", "agentsMayCreate": true, "autoArchive": true,
                "canChange": true
            }),
            true,
            StatusCode::BAD_REQUEST,
            "sub_room.invalid_policy_body",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(sub_request(Method::PUT, &uri, &body, origin, None))
            .await
            .expect("路由可调用");
        assert_eq!(response.status(), status, "{body}");
        assert_eq!(response_json(response).await["code"], code, "{body}");
    }
    assert_eq!(sub_rooms.changed.lock().unwrap().len(), 1, "被拒的没到用例");
}

#[tokio::test]
async fn 子房间这边被拒各有各的码_网页好说清楚() {
    let uri = format!("/private-rooms/{CATALOG_UUID}/sub-rooms");
    for (refusal, status, code) in [
        (
            SubRoomRefusal::ParentOnly,
            StatusCode::CONFLICT,
            "sub_room.parent_only",
        ),
        (
            SubRoomRefusal::Nested,
            StatusCode::CONFLICT,
            "sub_room.nested",
        ),
        (
            SubRoomRefusal::AdminsOnly,
            StatusCode::FORBIDDEN,
            "sub_room.admins_only",
        ),
        (
            SubRoomRefusal::AgentsMayNotCreate,
            StatusCode::FORBIDDEN,
            "sub_room.agents_may_not_create",
        ),
        (
            SubRoomRefusal::NameTaken,
            StatusCode::CONFLICT,
            "sub_room.name_taken",
        ),
        (
            SubRoomRefusal::LimitReached,
            StatusCode::CONFLICT,
            "sub_room.limit_reached",
        ),
        (
            SubRoomRefusal::RateLimited,
            StatusCode::TOO_MANY_REQUESTS,
            "sub_room.rate_limited",
        ),
        (
            SubRoomRefusal::PolicyOwnerOnly,
            StatusCode::FORBIDDEN,
            "sub_room.owner_only",
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
                Method::POST,
                &uri,
                &json!({ "name": "release" }),
                true,
                Some(SUB_ROOM_UUID),
            ))
            .await
            .expect("路由可调用");
        assert_eq!(response.status(), status, "{code}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(response_json(response).await["code"], code);
    }
}

fn sub_room_router(
    sub_rooms: Arc<FakeSubRooms>,
    authentication: Arc<FakeAuthentication>,
) -> axum::Router {
    let state = PrivateRoomHttpState::new(
        PrivateRoomHttpDependencies {
            rooms: Arc::new(FakeRooms::new()),
            sub_rooms,
            authentication,
            devices: Arc::<FakeDevices>::default(),
            secrets: Arc::new(SecureSecretFactory),
        },
        &Url::parse(FRONTEND_ORIGIN).expect("前端地址有效"),
        &crate::config::DesktopOrigins::for_tests(),
    );
    router(state).layer(middleware::from_fn(crate::correlation::attach))
}

/// 网页发的请求：带着会话；`origin` 是带不带来源，`key` 是幂等键（子房间的房间号）。
fn sub_request(
    method: Method,
    uri: &str,
    body: &Value,
    origin: bool,
    key: Option<&str>,
) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, "__Host-agent-room-session=session-secret");
    if origin {
        request = request.header(header::ORIGIN, FRONTEND_ORIGIN);
    }
    if let Some(key) = key {
        request = request.header("idempotency-key", key);
    }
    request
        .body(Body::from(body.to_string()))
        .expect("HTTP 请求有效")
}

/// 两间子房间：一间没有门、看的人在里面；一间只能请进来、不在名单里显示、看的人被封禁了。
fn list() -> SubRoomList {
    let entry = |catalog: &str, name: &str| SubRoomEntry {
        catalog_id: RoomCatalogId::from_uuid(uuid(catalog)),
        name: name.to_owned(),
        description: String::new(),
        access: RoomJoinMode::Open,
        hidden: false,
        archived: false,
        created_by: owner_id(),
        created_at: time(1_700_000_000_000),
        managers: Vec::new(),
        viewer_status: None,
        member_count: 3,
        agent_count: 2,
        last_message_at: Some(time(1_700_000_100_000)),
    };
    SubRoomList {
        parent_catalog_id: catalog_id(),
        parent_name: "Incident Room".to_owned(),
        can_create: true,
        sub_rooms: vec![
            SubRoomView {
                entry: SubRoomEntry {
                    description: "Ship Alpha 95".to_owned(),
                    created_by: target_id(),
                    managers: vec![target_id()],
                    viewer_status: Some(PrivateRoomMembershipStatus::Joined),
                    ..entry(SUB_ROOM_UUID, "release")
                },
                inside_visible: true,
            },
            SubRoomView {
                entry: SubRoomEntry {
                    access: RoomJoinMode::InviteOnly,
                    hidden: true,
                    viewer_status: Some(PrivateRoomMembershipStatus::Banned),
                    ..entry(OTHER_SUB_ROOM_UUID, "security")
                },
                inside_visible: false,
            },
        ],
    }
}
