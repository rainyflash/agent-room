//! 子房间的接口（`specs/redesign/sub-rooms.md`）：建、读名单、三项设置（2a），加入、退出、改名字
//! 和用途（2b），门和“不在名单里显示”（2d-1），申请和批准（2d-2，在 `sub_room_request_tests.rs`），
//! 请进来、移出和封禁（2d-3，在 `sub_room_member_tests.rs`）。
//! 规则在用例里，这里看路由把什么交给了用例、回答长什么样、被拒时回哪个码。

use agent_room_application::{
    ports::{JoinRequestRecord, PendingJoinRequest, SubRoomEntry},
    private_rooms::{
        AnswerSubRoomJoinRequest, ChangeSubRoomPolicy, CreateSubRoom, GovernSubRoomMember,
        InspectSubRoomPolicy, ListSubRooms, RequestSubRoomJoin, ReviseSubRoom, SubRoomJoinState,
        SubRoomList, SubRoomMembershipAction, SubRoomPolicyView, SubRoomRefusal, SubRoomUseCases,
        SubRoomView,
    },
};
use agent_room_domain::{
    private_rooms::PrivateRoomMembershipStatus,
    room_join_requests::{JoinRequestMessage, JoinRequestStatus},
    sub_rooms::{SubRoomCreators, SubRoomPolicy},
};

use super::*;

#[path = "sub_room_member_tests.rs"]
mod members;
#[path = "sub_room_request_tests.rs"]
mod requests;

const SUB_ROOM_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e50";
const OTHER_SUB_ROOM_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e51";
const THIRD_SUB_ROOM_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e52";
/// 申请记下的时间。
const REQUESTED_AT: i64 = 1_700_000_200_000;

/// 记下交过来的请求，照设好的回答。
#[derive(Default)]
pub(super) struct FakeSubRooms {
    created: Mutex<Vec<CreateSubRoom>>,
    listed: Mutex<Vec<RoomCatalogId>>,
    inspected: Mutex<Vec<RoomCatalogId>>,
    changed: Mutex<Vec<(RoomCatalogId, SubRoomPolicy)>>,
    /// 加入和退出：父房间、子房间。
    joined: Mutex<Vec<(RoomCatalogId, RoomCatalogId)>>,
    left: Mutex<Vec<(RoomCatalogId, RoomCatalogId)>>,
    revised: Mutex<Vec<ReviseSubRoom>>,
    /// 申请、撤回（父房间、子房间）、看申请（父房间、子房间）、处理申请。
    requested: Mutex<Vec<RequestSubRoomJoin>>,
    withdrawn: Mutex<Vec<(RoomCatalogId, RoomCatalogId)>>,
    pending_listed: Mutex<Vec<(RoomCatalogId, RoomCatalogId)>>,
    answered: Mutex<Vec<AnswerSubRoomJoinRequest>>,
    /// 请进来、移出、封禁、解封：哪一样，交过来的请求。
    governed: Mutex<Vec<(&'static str, GovernSubRoomMember)>>,
    /// 设上时申请不用等，直接记成已加入（没有门了的、管这间房的）。
    joins_directly: Mutex<bool>,
    /// 设上时每个用例都回答它。
    failure: Mutex<Option<PrivateRoomFailure>>,
}

impl FakeSubRooms {
    fn refusal(&self) -> Option<PrivateRoomFailure> {
        *self.failure.lock().unwrap()
    }

    fn govern(
        &self,
        action: &'static str,
        request: GovernSubRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<()>> {
        let result = self.refusal().map_or(Ok(()), Err);
        self.governed.lock().unwrap().push((action, request));
        Box::pin(async move { result })
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

    fn join_sub_room(
        &self,
        request: SubRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        self.joined
            .lock()
            .unwrap()
            .push((request.parent_catalog_id, request.catalog_id));
        let result = self.refusal().map_or_else(|| Ok(snapshot()), Err);
        Box::pin(async move { result })
    }

    fn leave_sub_room(
        &self,
        request: SubRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<()>> {
        self.left
            .lock()
            .unwrap()
            .push((request.parent_catalog_id, request.catalog_id));
        let result = self.refusal().map_or(Ok(()), Err);
        Box::pin(async move { result })
    }

    fn revise_sub_room(
        &self,
        request: ReviseSubRoom,
    ) -> PortFuture<'_, PrivateRoomResult<PrivateRoomSnapshot>> {
        let result = self.refusal().map_or_else(|| Ok(snapshot()), Err);
        self.revised.lock().unwrap().push(request);
        Box::pin(async move { result })
    }

    fn request_sub_room_join(
        &self,
        request: RequestSubRoomJoin,
    ) -> PortFuture<'_, PrivateRoomResult<SubRoomJoinState>> {
        let state = if *self.joins_directly.lock().unwrap() {
            SubRoomJoinState::Joined
        } else {
            SubRoomJoinState::Pending {
                requested_at: time(REQUESTED_AT),
            }
        };
        let result = self.refusal().map_or(Ok(state), Err);
        self.requested.lock().unwrap().push(request);
        Box::pin(async move { result })
    }

    fn withdraw_sub_room_join_request(
        &self,
        request: SubRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<()>> {
        self.withdrawn
            .lock()
            .unwrap()
            .push((request.parent_catalog_id, request.catalog_id));
        let result = self.refusal().map_or(Ok(()), Err);
        Box::pin(async move { result })
    }

    fn sub_room_join_requests(
        &self,
        request: SubRoomMembershipAction,
    ) -> PortFuture<'_, PrivateRoomResult<Vec<PendingJoinRequest>>> {
        self.pending_listed
            .lock()
            .unwrap()
            .push((request.parent_catalog_id, request.catalog_id));
        let pending = PendingJoinRequest {
            record: JoinRequestRecord {
                catalog_id: request.catalog_id,
                requester: target_id(),
                message: JoinRequestMessage::parse("想来帮忙").expect("留言有效"),
                status: JoinRequestStatus::Pending,
                requested_at: time(REQUESTED_AT),
                decided_at: None,
                decided_by: None,
            },
            requester_display_name: "riley".to_owned(),
        };
        let result = self.refusal().map_or_else(|| Ok(vec![pending]), Err);
        Box::pin(async move { result })
    }

    fn answer_sub_room_join_request(
        &self,
        request: AnswerSubRoomJoinRequest,
    ) -> PortFuture<'_, PrivateRoomResult<JoinRequestStatus>> {
        let status = if request.approve {
            JoinRequestStatus::Approved
        } else {
            JoinRequestStatus::Rejected
        };
        let result = self.refusal().map_or(Ok(status), Err);
        self.answered.lock().unwrap().push(request);
        Box::pin(async move { result })
    }

    fn admit_sub_room_member(
        &self,
        request: GovernSubRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<()>> {
        self.govern("admit", request)
    }

    fn remove_sub_room_member(
        &self,
        request: GovernSubRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<()>> {
        self.govern("remove", request)
    }

    fn ban_sub_room_member(
        &self,
        request: GovernSubRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<()>> {
        self.govern("ban", request)
    }

    fn unban_sub_room_member(
        &self,
        request: GovernSubRoomMember,
    ) -> PortFuture<'_, PrivateRoomResult<()>> {
        self.govern("unban", request)
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
    assert_eq!(
        (requests[0].access, requests[0].hidden),
        (RoomJoinMode::Open, false),
        "没带门就是没有门、在名单里显示"
    );
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
            // 分类还没有：带了不认识的字段就拒，免得人以为放进了分类。
            sub_request(
                Method::POST,
                &uri,
                &json!({ "name": "release", "category": "launch" }),
                true,
                Some(SUB_ROOM_UUID),
            ),
            StatusCode::BAD_REQUEST,
            "sub_room.invalid_creation_body",
        ),
        (
            sub_request(
                Method::POST,
                &uri,
                &json!({ "name": "release", "access": "secret" }),
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

    // 建的时候就带门：三种门是公开房间加入方式的那一套词；藏不藏原样交给用例，规则由它管。
    for (access, hidden, expected) in [
        ("request", true, RoomJoinMode::Request),
        ("invite_only", false, RoomJoinMode::InviteOnly),
        ("open", true, RoomJoinMode::Open),
    ] {
        let body = json!({ "name": "security", "access": access, "hidden": hidden });
        let request = sub_request(Method::POST, &uri, &body, true, Some(SUB_ROOM_UUID));
        let created = app.clone().oneshot(request).await.expect("路由可调用");
        assert_eq!(created.status(), StatusCode::CREATED, "{access}");
        let requests = sub_rooms.created.lock().unwrap().clone();
        let last = requests.last().expect("到了用例");
        assert_eq!((last.access, last.hidden), (expected, hidden), "{access}");
    }
}

#[tokio::test]
async fn 名单只要有效登录_有门又没进的不给人数和最近说话的时间_在等几个申请只给管这一间的() {
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
                    "lastMessageAtUnixMs": 1_700_000_100_000_i64,
                    "pendingRequestCount": 2
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
                },
                {
                    "catalogId": THIRD_SUB_ROOM_UUID,
                    "name": "design",
                    "description": "",
                    "access": "request",
                    "hidden": false,
                    "createdByPrincipalId": OWNER_UUID,
                    "createdAtUnixMs": 1_700_000_000_000_i64,
                    "managerPrincipalIds": [],
                    "joined": false,
                    "banned": false,
                    "requestedAtUnixMs": REQUESTED_AT
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
async fn 加入和退出各一个请求_要同源和有效登录_退出不回内容() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let authentication = Arc::new(FakeAuthentication::default());
    let app = sub_room_router(sub_rooms.clone(), authentication.clone());
    let uri = format!("/private-rooms/{CATALOG_UUID}/sub-rooms/{SUB_ROOM_UUID}/membership");
    let pair = (catalog_id(), RoomCatalogId::from_uuid(uuid(SUB_ROOM_UUID)));

    let joined = app
        .clone()
        .oneshot(sub_request(Method::POST, &uri, &json!({}), true, None))
        .await
        .expect("加入的路由可调用");
    assert_eq!(joined.status(), StatusCode::OK);
    assert_eq!(joined.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response_json(joined).await["name"], "Incident Room");
    assert_eq!(sub_rooms.joined.lock().unwrap().as_slice(), &[pair]);

    let left = app
        .clone()
        .oneshot(sub_request(Method::DELETE, &uri, &json!({}), true, None))
        .await
        .expect("退出的路由可调用");
    assert_eq!(left.status(), StatusCode::NO_CONTENT);
    assert_eq!(left.headers()[header::CACHE_CONTROL], "no-store");
    assert!(
        to_bytes(left.into_body(), 64)
            .await
            .expect("读得出回答")
            .is_empty()
    );
    assert_eq!(sub_rooms.left.lock().unwrap().as_slice(), &[pair]);
    assert_eq!(
        authentication.calls.lock().unwrap().as_slice(),
        &[AuthenticationRequirement::ActiveSession; 2],
        "加入、退出只要有效登录"
    );

    let malformed = format!("/private-rooms/{CATALOG_UUID}/sub-rooms/not-a-room/membership");
    for method in [Method::POST, Method::DELETE] {
        for (uri, origin, status, code) in [
            (
                &uri,
                false,
                StatusCode::FORBIDDEN,
                "private_room.invalid_origin",
            ),
            (
                &malformed,
                true,
                StatusCode::BAD_REQUEST,
                "private_room.invalid_resource_id",
            ),
        ] {
            let response = app
                .clone()
                .oneshot(sub_request(method.clone(), uri, &json!({}), origin, None))
                .await
                .expect("路由可调用");
            assert_eq!(response.status(), status, "{method} {code}");
            assert_eq!(response_json(response).await["code"], code);
        }
    }
    assert_eq!(sub_rooms.joined.lock().unwrap().len(), 1, "被拒的没到用例");
    assert_eq!(sub_rooms.left.lock().unwrap().len(), 1, "被拒的没到用例");
}

#[tokio::test]
async fn 改名字和用途_没带的那一项不动_名字去掉两头的空白_多余的字段和空请求不收() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let app = sub_room_router(sub_rooms.clone(), Arc::default());
    let uri = format!("/private-rooms/{CATALOG_UUID}/sub-rooms/{SUB_ROOM_UUID}");

    for body in [
        json!({ "name": "  launch ", "description": " Ship it " }),
        json!({ "description": "" }),
        json!({ "name": "launch" }),
    ] {
        let revised = app
            .clone()
            .oneshot(sub_request(Method::PUT, &uri, &body, true, None))
            .await
            .expect("改名的路由可调用");
        assert_eq!(revised.status(), StatusCode::OK, "{body}");
        assert_eq!(revised.headers()[header::CACHE_CONTROL], "no-store");
    }
    let requests = sub_rooms.revised.lock().unwrap().clone();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].parent_catalog_id, catalog_id());
    assert_eq!(
        requests[0].catalog_id,
        RoomCatalogId::from_uuid(uuid(SUB_ROOM_UUID))
    );
    assert_eq!(requests[0].name.as_deref(), Some("launch"));
    assert_eq!(
        requests[0].description.as_deref(),
        Some(" Ship it "),
        "用途原样交给用例，由它去掉两头的空白"
    );
    assert_eq!(
        (
            requests[1].name.as_deref(),
            requests[1].description.as_deref()
        ),
        (None, Some("")),
        "只改用途，空的就是清掉"
    );
    assert_eq!(
        (
            requests[2].name.as_deref(),
            requests[2].description.as_deref()
        ),
        (Some("launch"), None)
    );

    for (body, origin, status, code) in [
        (
            json!({ "name": "launch" }),
            false,
            StatusCode::FORBIDDEN,
            "private_room.invalid_origin",
        ),
        (
            json!({}),
            true,
            StatusCode::BAD_REQUEST,
            "sub_room.invalid_revision_body",
        ),
        (
            // 分类还没有：带了不认识的字段就拒，免得人以为改上了。
            json!({ "name": "launch", "category": "launch" }),
            true,
            StatusCode::BAD_REQUEST,
            "sub_room.invalid_revision_body",
        ),
        (
            json!({ "name": 7 }),
            true,
            StatusCode::BAD_REQUEST,
            "sub_room.invalid_revision_body",
        ),
        (
            json!({ "access": "secret" }),
            true,
            StatusCode::BAD_REQUEST,
            "sub_room.invalid_revision_body",
        ),
        (
            json!({ "hidden": "yes" }),
            true,
            StatusCode::BAD_REQUEST,
            "sub_room.invalid_revision_body",
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
    assert_eq!(sub_rooms.revised.lock().unwrap().len(), 3, "被拒的没到用例");
}

#[tokio::test]
async fn 换门和隐藏_只带哪一项就只交哪一项_名字和用途不带也行() {
    let sub_rooms = Arc::new(FakeSubRooms::default());
    let app = sub_room_router(sub_rooms.clone(), Arc::default());
    let uri = format!("/private-rooms/{CATALOG_UUID}/sub-rooms/{SUB_ROOM_UUID}");

    for body in [
        json!({ "access": "request", "hidden": true }),
        json!({ "access": "invite_only" }),
        json!({ "hidden": false }),
        json!({ "name": "launch", "access": "open" }),
    ] {
        let revised = app
            .clone()
            .oneshot(sub_request(Method::PUT, &uri, &body, true, None))
            .await
            .expect("换门的路由可调用");
        assert_eq!(revised.status(), StatusCode::OK, "{body}");
    }
    let asked: Vec<_> = sub_rooms
        .revised
        .lock()
        .unwrap()
        .iter()
        .map(|request| (request.name.clone(), request.access, request.hidden))
        .collect();
    assert_eq!(
        asked,
        [
            (None, Some(RoomJoinMode::Request), Some(true)),
            (None, Some(RoomJoinMode::InviteOnly), None),
            (None, None, Some(false)),
            (Some("launch".to_owned()), Some(RoomJoinMode::Open), None),
        ],
        "没带的那一项是空的，由用例照原来的留着"
    );
}

/// 子房间这边每一种拒绝回的状态和码。
fn refusal_codes() -> [(SubRoomRefusal, StatusCode, &'static str); 18] {
    [
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
        (
            SubRoomRefusal::Archived,
            StatusCode::CONFLICT,
            "sub_room.archived",
        ),
        (
            SubRoomRefusal::Banned,
            StatusCode::FORBIDDEN,
            "sub_room.banned",
        ),
        (
            SubRoomRefusal::ApprovalRequired,
            StatusCode::FORBIDDEN,
            "sub_room.approval_required",
        ),
        (
            SubRoomRefusal::InviteOnly,
            StatusCode::FORBIDDEN,
            "sub_room.invite_only",
        ),
        (
            SubRoomRefusal::OwnerStays,
            StatusCode::CONFLICT,
            "sub_room.owner_stays",
        ),
        (
            SubRoomRefusal::ManagersOnly,
            StatusCode::FORBIDDEN,
            "sub_room.managers_only",
        ),
        (
            SubRoomRefusal::RecentlyRejected,
            StatusCode::CONFLICT,
            "sub_room.recently_rejected",
        ),
        (
            SubRoomRefusal::NotInParent,
            StatusCode::CONFLICT,
            "sub_room.not_in_parent",
        ),
        (
            SubRoomRefusal::TargetBanned,
            StatusCode::CONFLICT,
            "sub_room.target_banned",
        ),
        (
            SubRoomRefusal::PeerManager,
            StatusCode::FORBIDDEN,
            "sub_room.peer_manager",
        ),
    ]
}

#[tokio::test]
async fn 子房间这边被拒各有各的码_网页好说清楚() {
    let uri = format!("/private-rooms/{CATALOG_UUID}/sub-rooms");
    for (refusal, status, code) in refusal_codes() {
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

/// 三间子房间：一间没有门、看的人在里面管着它；一间只能请进来、不在名单里显示、看的人被封禁了；
/// 一间要批准、看的人在等。每一间都有两个在等的申请，只有管着的那一间写出来。
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
        viewer_requested_at: None,
        pending_requests: 2,
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
                manages: true,
            },
            SubRoomView {
                entry: SubRoomEntry {
                    access: RoomJoinMode::InviteOnly,
                    hidden: true,
                    viewer_status: Some(PrivateRoomMembershipStatus::Banned),
                    ..entry(OTHER_SUB_ROOM_UUID, "security")
                },
                inside_visible: false,
                manages: false,
            },
            SubRoomView {
                entry: SubRoomEntry {
                    access: RoomJoinMode::Request,
                    viewer_requested_at: Some(time(REQUESTED_AT)),
                    ..entry(THIRD_SUB_ROOM_UUID, "design")
                },
                inside_visible: false,
                manages: false,
            },
        ],
    }
}
