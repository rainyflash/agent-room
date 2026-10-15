use std::sync::{Arc, Mutex};

use agent_room_application::{
    authentication::{
        AuthenticatedPrincipal, AuthenticationRequirement, AuthenticationResult,
        AuthenticationUseCases, BeginLogin, CompleteLogin, LoginCompletion, LoginRedirect,
    },
    ports::{PortFuture, RoomAgentKind, RoomAgentOwnerRecord, RoomPersonName, SecretValue},
    room_people::{
        ListRoomPeople, LookupRoomAgentOwners, RoomPeopleFailure, RoomPeopleResult,
        RoomPeopleUseCases,
    },
};
use agent_room_domain::{
    ids::{AgentId, PrincipalId, RoomCatalogId},
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

use super::{RoomPeopleHttpState, router};

const PRINCIPAL_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e42";
const CATALOG_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const LOCAL_AGENT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e44";
const NETWORK_AGENT_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e45";
const OWNER_UUID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e47";

#[derive(Default)]
struct FakePeople {
    people: Mutex<Vec<ListRoomPeople>>,
    owners: Mutex<Vec<LookupRoomAgentOwners>>,
    failure: Mutex<Option<RoomPeopleFailure>>,
}

impl RoomPeopleUseCases for FakePeople {
    fn people(
        &self,
        request: ListRoomPeople,
    ) -> PortFuture<'_, RoomPeopleResult<Vec<RoomPersonName>>> {
        self.people.lock().unwrap().push(request);
        let failure = self.failure.lock().unwrap().take();
        Box::pin(async move {
            failure.map_or_else(
                || {
                    Ok(vec![RoomPersonName {
                        principal_id: principal(OWNER_UUID),
                        display_name: "小明".to_owned(),
                    }])
                },
                Err,
            )
        })
    }

    fn agent_owners(
        &self,
        request: LookupRoomAgentOwners,
    ) -> PortFuture<'_, RoomPeopleResult<Vec<RoomAgentOwnerRecord>>> {
        self.owners.lock().unwrap().push(request);
        let failure = self.failure.lock().unwrap().take();
        Box::pin(async move {
            failure.map_or_else(
                || {
                    Ok(vec![
                        RoomAgentOwnerRecord {
                            agent_id: agent(LOCAL_AGENT_UUID),
                            kind: RoomAgentKind::Local,
                            owner: Some(RoomPersonName {
                                principal_id: principal(OWNER_UUID),
                                display_name: "小明".to_owned(),
                            }),
                            admitted_by: None,
                        },
                        RoomAgentOwnerRecord {
                            agent_id: agent(NETWORK_AGENT_UUID),
                            kind: RoomAgentKind::Network,
                            owner: None,
                            admitted_by: Some(RoomPersonName {
                                principal_id: principal(PRINCIPAL_UUID),
                                display_name: "房主".to_owned(),
                            }),
                        },
                    ])
                },
                Err,
            )
        })
    }
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

#[tokio::test]
async fn 成员的名字按房间回答() {
    let people = Arc::new(FakePeople::default());
    let response = test_router(people.clone())
        .oneshot(signed_in(&format!("/private-rooms/{CATALOG_UUID}/people")))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&header::HeaderValue::from_static("no-store"))
    );
    assert_eq!(
        response_json(response).await,
        json!({ "people": [{ "principalId": OWNER_UUID, "displayName": "小明" }] })
    );
    let requests = people.people.lock().unwrap();
    assert_eq!(requests[0].actor, actor());
    assert_eq!(requests[0].catalog_id, catalog());
}

#[tokio::test]
async fn agent_的主人分批问_重复的只问一次_空的不写() {
    let people = Arc::new(FakePeople::default());
    let response = test_router(people.clone())
        .oneshot(signed_in(&format!(
            "/rooms/{CATALOG_UUID}/agent-owners?agentIds={LOCAL_AGENT_UUID},{NETWORK_AGENT_UUID},{LOCAL_AGENT_UUID}"
        )))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response_json(response).await,
        json!({
            "agents": [
                {
                    "agentId": LOCAL_AGENT_UUID,
                    "kind": "local",
                    "owner": { "principalId": OWNER_UUID, "displayName": "小明" }
                },
                {
                    "agentId": NETWORK_AGENT_UUID,
                    "kind": "network",
                    "admittedBy": { "principalId": PRINCIPAL_UUID, "displayName": "房主" }
                }
            ]
        })
    );
    let requests = people.owners.lock().unwrap();
    assert_eq!(
        requests[0].agent_ids,
        vec![agent(LOCAL_AGENT_UUID), agent(NETWORK_AGENT_UUID)]
    );
    assert_eq!(requests[0].catalog_id, catalog());
}

#[tokio::test]
async fn agent_编号不对或太多时不问() {
    let too_many = (0..101)
        .map(|_| Uuid::now_v7().to_string())
        .collect::<Vec<_>>()
        .join(",");
    for query in [
        String::new(),
        "agentIds=".to_owned(),
        "agentIds=not-a-uuid".to_owned(),
        format!("agentIds={too_many}"),
        format!("agentIds={LOCAL_AGENT_UUID}&extra=1"),
    ] {
        let people = Arc::new(FakePeople::default());
        let response = test_router(people.clone())
            .oneshot(signed_in(&format!(
                "/rooms/{CATALOG_UUID}/agent-owners?{query}"
            )))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(
            response_json(response).await["code"],
            "room_people.invalid_agent_ids"
        );
        assert!(people.owners.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn 不在房间里回_404_没登录回_401() {
    let people = Arc::new(FakePeople::default());
    *people.failure.lock().unwrap() = Some(RoomPeopleFailure::NotFound);
    let missing = test_router(people.clone())
        .oneshot(signed_in(&format!("/private-rooms/{CATALOG_UUID}/people")))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response_json(missing).await["code"],
        "room_people.not_found"
    );

    let anonymous = test_router(people.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/private-rooms/{CATALOG_UUID}/people"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(people.people.lock().unwrap().len(), 1);
}

fn test_router(people: Arc<FakePeople>) -> Router {
    router(RoomPeopleHttpState {
        people,
        authentication: Arc::new(FakeAuthentication),
    })
    .layer(middleware::from_fn(crate::correlation::attach))
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

fn principal(value: &str) -> PrincipalId {
    PrincipalId::from_uuid(Uuid::parse_str(value).unwrap())
}

fn agent(value: &str) -> AgentId {
    AgentId::from_uuid(Uuid::parse_str(value).unwrap())
}

fn catalog() -> RoomCatalogId {
    RoomCatalogId::from_uuid(Uuid::parse_str(CATALOG_UUID).unwrap())
}

fn time(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}
