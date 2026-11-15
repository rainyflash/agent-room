use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    ports::{PortFuture, RoomRoleChangeRecord, RoomRoleChangeSubRoom, SecretValue},
    room_role_changes::{
        ListRoomRoleChanges, RoomRoleChangeFailure, RoomRoleChangeResult, RoomRoleChangeUseCases,
    },
};
use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    private_rooms::{PrivateRoomCapability, PrivateRoomMembershipStatus, PrivateRoomPermissions},
    room_role_changes::RoomStanding,
    time::UtcMillis,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    middleware,
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use super::{RoomRoleChangeHttpState, router};

const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const MEMBER_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e47";
const INVITED_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e48";
const SUB_ROOM_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e49";

#[derive(Default)]
struct FakeRoleChanges {
    requests: Mutex<Vec<ListRoomRoleChanges>>,
    failure: Mutex<Option<RoomRoleChangeFailure>>,
}

impl RoomRoleChangeUseCases for FakeRoleChanges {
    fn recent(
        &self,
        request: ListRoomRoleChanges,
    ) -> PortFuture<'_, RoomRoleChangeResult<Vec<RoomRoleChangeRecord>>> {
        self.requests.lock().unwrap().push(request);
        let failure = self.failure.lock().unwrap().take();
        Box::pin(async move { failure.map_or_else(|| Ok(records()), Err) })
    }
}

#[tokio::test]
async fn 记录按房间回答_两头给状态房主和权限_空的不写_子房间里的写着是哪一间() {
    let role_changes = Arc::new(FakeRoleChanges::default());
    let response = test_router(role_changes.clone())
        .oneshot(signed_in(&path()))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&header::HeaderValue::from_static("no-store"))
    );
    assert_eq!(
        response_json(response).await,
        json!({
            "changes": [
                {
                    "id": 12,
                    "targetPrincipalId": INVITED_UUID,
                    "next": {
                        "status": "invited",
                        "owner": false,
                        "permissions": { "capabilities": ["view", "speak", "invite", "manage"] }
                    },
                    "changedAtUnixMs": 1_700_000_300_000_i64
                },
                {
                    "id": 11,
                    "actorPrincipalId": PRINCIPAL_UUID,
                    "targetPrincipalId": MEMBER_UUID,
                    "previous": {
                        "status": "joined",
                        "owner": false,
                        "permissions": { "capabilities": ["view", "speak"] }
                    },
                    "next": {
                        "status": "joined",
                        "owner": true,
                        "permissions": {
                            "capabilities": ["view", "speak", "invite", "manage", "automate"]
                        }
                    },
                    "changedAtUnixMs": 1_700_000_200_000_i64
                },
                {
                    "id": 10,
                    "actorPrincipalId": PRINCIPAL_UUID,
                    "targetPrincipalId": MEMBER_UUID,
                    "previous": {
                        "status": "joined",
                        "owner": false,
                        "permissions": { "capabilities": ["view", "speak"] }
                    },
                    "next": {
                        "status": "joined",
                        "owner": false,
                        "permissions": { "capabilities": ["view", "speak", "manage"] }
                    },
                    "changedAtUnixMs": 1_700_000_100_000_i64,
                    "subRoom": { "catalogId": SUB_ROOM_UUID, "name": "release" }
                }
            ]
        })
    );
    let requests = role_changes.requests.lock().unwrap();
    assert_eq!(requests[0].actor, actor());
    assert_eq!(requests[0].catalog_id, catalog());
}

#[tokio::test]
async fn 成员回_403_不在房间里回_404_没登录回_401_房间号不对不问() {
    for (failure, status, code) in [
        (
            RoomRoleChangeFailure::ManagersOnly,
            StatusCode::FORBIDDEN,
            "room_role_changes.managers_only",
        ),
        (
            RoomRoleChangeFailure::NotFound,
            StatusCode::NOT_FOUND,
            "room_role_changes.not_found",
        ),
        (
            RoomRoleChangeFailure::Forbidden,
            StatusCode::FORBIDDEN,
            "room_role_changes.forbidden",
        ),
        (
            RoomRoleChangeFailure::DependencyUnavailable,
            StatusCode::SERVICE_UNAVAILABLE,
            "room_role_changes.dependency_unavailable",
        ),
    ] {
        let role_changes = Arc::new(FakeRoleChanges::default());
        *role_changes.failure.lock().unwrap() = Some(failure);
        let response = test_router(role_changes)
            .oneshot(signed_in(&path()))
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{failure:?}");
        assert_eq!(response_json(response).await["code"], code);
    }

    let role_changes = Arc::new(FakeRoleChanges::default());
    let anonymous = test_router(role_changes.clone())
        .oneshot(Request::builder().uri(path()).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let invalid = test_router(role_changes.clone())
        .oneshot(signed_in("/private-rooms/not-a-uuid/role-changes"))
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(invalid).await["code"],
        "room_role_changes.invalid_catalog"
    );
    assert!(role_changes.requests.lock().unwrap().is_empty());
}

/// 新的在前：一位管理员被请进来（之前名单里没有他，这一条也没有动手的人：两个可以为空的字段都空着），
/// 再往前是转让房主，最早的是子房间里设了一位管理员。
fn records() -> Vec<RoomRoleChangeRecord> {
    let member = permissions(&[PrivateRoomCapability::View, PrivateRoomCapability::Speak]);
    let admin = permissions(&[
        PrivateRoomCapability::View,
        PrivateRoomCapability::Speak,
        PrivateRoomCapability::Invite,
        PrivateRoomCapability::Manage,
    ]);
    vec![
        RoomRoleChangeRecord {
            id: 12,
            actor: None,
            target: principal(INVITED_UUID),
            previous: None,
            next: RoomStanding {
                status: PrivateRoomMembershipStatus::Invited,
                owner: false,
                permissions: admin,
            },
            changed_at: time(1_700_000_300_000),
            sub_room: None,
        },
        RoomRoleChangeRecord {
            id: 11,
            actor: Some(principal(PRINCIPAL_UUID)),
            target: principal(MEMBER_UUID),
            previous: Some(RoomStanding {
                status: PrivateRoomMembershipStatus::Joined,
                owner: false,
                permissions: member,
            }),
            next: RoomStanding {
                status: PrivateRoomMembershipStatus::Joined,
                owner: true,
                permissions: PrivateRoomPermissions::ALL,
            },
            changed_at: time(1_700_000_200_000),
            sub_room: None,
        },
        RoomRoleChangeRecord {
            id: 10,
            actor: Some(principal(PRINCIPAL_UUID)),
            target: principal(MEMBER_UUID),
            previous: Some(RoomStanding {
                status: PrivateRoomMembershipStatus::Joined,
                owner: false,
                permissions: member,
            }),
            next: RoomStanding {
                status: PrivateRoomMembershipStatus::Joined,
                owner: false,
                permissions: permissions(&[
                    PrivateRoomCapability::View,
                    PrivateRoomCapability::Speak,
                    PrivateRoomCapability::Manage,
                ]),
            },
            changed_at: time(1_700_000_100_000),
            sub_room: Some(RoomRoleChangeSubRoom {
                catalog_id: RoomCatalogId::from_uuid(Uuid::parse_str(SUB_ROOM_UUID).unwrap()),
                name: "release".to_owned(),
            }),
        },
    ]
}

struct FakeAuthentication;

impl AuthenticationUseCases for FakeAuthentication {
    fn begin_login(
        &self,
        _request: BeginLogin,
    ) -> PortFuture<'_, AuthenticationResult<LoginRedirect>> {
        Box::pin(async { unreachable!("不开始登录") })
    }

    fn complete_login<'a>(
        &'a self,
        _request: CompleteLogin<'a>,
    ) -> PortFuture<'a, AuthenticationResult<LoginCompletion>> {
        Box::pin(async { unreachable!("不完成登录") })
    }

    fn authenticate<'a>(
        &'a self,
        session_secret: &'a SecretValue,
        requirement: AuthenticationRequirement,
    ) -> PortFuture<'a, AuthenticationResult<AuthenticatedPrincipal>> {
        assert_eq!(session_secret.expose(), "session-secret");
        // 看记录不动任何东西，有效登录就够。
        assert_eq!(requirement, AuthenticationRequirement::ActiveSession);
        Box::pin(async { Ok(actor()) })
    }

    fn logout<'a>(
        &'a self,
        _session_secret: &'a SecretValue,
    ) -> PortFuture<'a, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("不退出登录") })
    }

    fn suspend_principal(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, AuthenticationResult<()>> {
        Box::pin(async { unreachable!("不停用账户") })
    }
}

fn test_router(role_changes: Arc<FakeRoleChanges>) -> Router {
    router(RoomRoleChangeHttpState {
        role_changes,
        authentication: Arc::new(FakeAuthentication),
    })
    .layer(middleware::from_fn(crate::correlation::attach))
}

fn path() -> String {
    format!("/private-rooms/{CATALOG_UUID}/role-changes")
}

fn signed_in(path: &str) -> Request<Body> {
    Request::builder()
        .uri(path)
        .header(header::COOKIE, "__Host-agent-room-session=session-secret")
        .body(Body::empty())
        .unwrap()
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 64 * 1_024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn actor() -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        principal_id: principal(PRINCIPAL_UUID),
        matrix_user_id: "@user:matrix.agent-room.test".to_owned(),
        display_name: "Agent Room User".to_owned(),
        locale: "zh-CN".to_owned(),
        authenticated_at: time(1_700_000_000_000),
        expires_at: time(1_700_000_900_000),
        recently_authenticated: false,
    }
}

fn permissions(capabilities: &[PrivateRoomCapability]) -> PrivateRoomPermissions {
    PrivateRoomPermissions::from_capabilities(capabilities.iter().copied()).unwrap()
}

fn principal(value: &str) -> PrincipalId {
    PrincipalId::from_uuid(Uuid::parse_str(value).unwrap())
}

fn catalog() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::parse_str(CATALOG_UUID).unwrap())
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}
