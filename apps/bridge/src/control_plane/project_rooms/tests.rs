use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_room_application::{
    devices::{DeviceRequestProof, DeviceRequestProofPayload},
    ports::{DeviceSignature, PortFuture, SecretFactory, SecretValue},
};
use agent_room_bridge_core::{
    project_rooms::{
        ControlPlaneProjectRoomGateway, ProjectRoomFailureKind, RepositoryClaimStatus,
    },
    session::{AuthorizedControlPlaneRequest, BridgeSessionResult, ControlPlaneRequestAuthorizer},
};
use agent_room_domain::{
    ids::{DeviceId, RepositoryClaimId, RoomCatalogId},
    project_rooms::GithubRepository,
    time::UtcMillis,
};
use agent_room_identity_adapter::SecureSecretFactory;
use axum::{
    Json, Router,
    extract::Path,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use uuid::Uuid;

use super::{ControlPlaneHttpConfig, ReqwestControlPlaneProjectRoomGateway};

const CATALOG_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const CLAIM_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e49";

#[derive(Default)]
struct RecordingAuthorizer {
    requests: Mutex<Vec<(String, String, String)>>,
}

impl ControlPlaneRequestAuthorizer for RecordingAuthorizer {
    fn authorize<'a>(
        &'a self,
        method: &'a str,
        request_target: &'a str,
        body: &'a str,
    ) -> PortFuture<'a, BridgeSessionResult<AuthorizedControlPlaneRequest>> {
        self.requests.lock().unwrap().push((
            method.to_owned(),
            request_target.to_owned(),
            body.to_owned(),
        ));
        let payload = DeviceRequestProofPayload::new(
            DeviceId::from_uuid(Uuid::from_u128(1)),
            UtcMillis::new(1_000).unwrap(),
            secret("0123456789abcdef"),
            method.to_owned(),
            request_target.to_owned(),
            SecureSecretFactory.digest(body),
        )
        .unwrap();
        Box::pin(async move {
            Ok(AuthorizedControlPlaneRequest {
                access_token: secret("access-token"),
                proof: DeviceRequestProof::new(payload, DeviceSignature::new(vec![5; 64]).unwrap()),
            })
        })
    }
}

#[tokio::test]
async fn 认领和核对各走设备签名的路径_回答原样转交() {
    let app = Router::new()
        .route(
            "/devices/current/project-rooms/claims",
            post(|headers: HeaderMap, body: String| async move {
                let signed = header(&headers, "authorization") == Some("Bearer access-token")
                    && header(&headers, "x-agent-room-device-id")
                        == Some("00000000-0000-0000-0000-000000000001");
                if signed
                    && serde_json::from_str::<Value>(&body).ok()
                        == Some(json!({"repository": "Octo/Hello", "catalogId": CATALOG_ID}))
                {
                    (StatusCode::OK, Json(pending_response())).into_response()
                } else {
                    StatusCode::BAD_REQUEST.into_response()
                }
            }),
        )
        .route(
            "/devices/current/project-rooms/claims/{claim_id}/check",
            post(|Path(claim_id): Path<String>, body: String| async move {
                if claim_id == CLAIM_ID && body.is_empty() {
                    (StatusCode::OK, Json(bound_response())).into_response()
                } else {
                    StatusCode::BAD_REQUEST.into_response()
                }
            }),
        );
    let authorizer = Arc::new(RecordingAuthorizer::default());
    let gateway = gateway(spawn_server(app).await, authorizer.clone());

    let pending = gateway
        .claim(
            &GithubRepository::parse("Octo/Hello").unwrap(),
            Some(RoomCatalogId::from_uuid(
                Uuid::parse_str(CATALOG_ID).unwrap(),
            )),
        )
        .await
        .expect("认领了");
    assert_eq!(pending.status, RepositoryClaimStatus::Pending);
    assert_eq!(pending.claim_id.unwrap().as_uuid().to_string(), CLAIM_ID);
    assert_eq!(pending.share_code.as_str(), "aB3xYz");
    assert_eq!(
        pending.last_check.as_ref().unwrap().result,
        "not_in_repository"
    );
    assert_eq!(
        pending.agents_line,
        pending_response()["agentsLine"].as_str().unwrap()
    );

    let bound = gateway.check(claim_id()).await.expect("核对了");
    assert_eq!(bound.status, RepositoryClaimStatus::Bound);
    assert_eq!(bound.claim_id, None);
    assert_eq!(bound.catalog_id.unwrap().to_string(), CATALOG_ID);

    let requests = authorizer.requests.lock().unwrap();
    let targets: Vec<_> = requests
        .iter()
        .map(|(method, target, _)| (method.as_str(), target.as_str()))
        .collect();
    assert_eq!(
        targets,
        [
            ("POST", "/devices/current/project-rooms/claims"),
            (
                "POST",
                format!("/devices/current/project-rooms/claims/{CLAIM_ID}/check").as_str()
            ),
        ]
    );
    assert_eq!(requests[1].2, "", "核对没有正文，签空串");
}

#[tokio::test]
async fn 控制面的错误码换成认领自己的失败_没有接口的控制面报不支持() {
    for (status, code, expected) in [
        (
            StatusCode::BAD_REQUEST,
            Some("project_room.invalid_repository"),
            ProjectRoomFailureKind::InvalidRepository,
        ),
        (
            StatusCode::CONFLICT,
            Some("project_room.too_many_claims"),
            ProjectRoomFailureKind::TooManyClaims,
        ),
        (
            StatusCode::CONFLICT,
            Some("project_room.repository_taken"),
            ProjectRoomFailureKind::RepositoryTaken,
        ),
        (
            StatusCode::CONFLICT,
            Some("project_room.room_not_eligible"),
            ProjectRoomFailureKind::RoomNotEligible,
        ),
        (
            StatusCode::CONFLICT,
            Some("room_listing.repository_bound"),
            ProjectRoomFailureKind::RoomAlreadyBound,
        ),
        (
            StatusCode::TOO_MANY_REQUESTS,
            Some("project_room.check_too_soon"),
            ProjectRoomFailureKind::CheckTooSoon,
        ),
        (
            StatusCode::NOT_FOUND,
            Some("project_room.not_found"),
            ProjectRoomFailureKind::NotFound,
        ),
        (
            StatusCode::FORBIDDEN,
            Some("project_room.forbidden"),
            ProjectRoomFailureKind::Forbidden,
        ),
        (
            StatusCode::UNAUTHORIZED,
            Some("device.invalid_access_token"),
            ProjectRoomFailureKind::NotAuthorized,
        ),
        (
            StatusCode::NOT_FOUND,
            None,
            ProjectRoomFailureKind::Unsupported,
        ),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Some("project_room.dependency_unavailable"),
            ProjectRoomFailureKind::ControlPlaneUnavailable,
        ),
    ] {
        let app = Router::new().route(
            "/devices/current/project-rooms/claims",
            post(move || async move {
                let body = code.map_or_else(|| json!({}), |code| json!({"code": code}));
                (status, Json(body)).into_response()
            }),
        );
        let gateway = gateway(spawn_server(app).await, Arc::default());
        let failure = gateway
            .claim(&GithubRepository::parse("octo/hello").unwrap(), None)
            .await
            .unwrap_err();
        assert_eq!(failure.kind(), expected, "{status} {code:?}");
    }
}

#[tokio::test]
async fn 回答不像样就不转交() {
    let mut bound_with_claim = bound_response();
    bound_with_claim["claimId"] = json!(CLAIM_ID);
    let mut pending_without_claim = pending_response();
    pending_without_claim["claimId"] = Value::Null;
    let mut odd_status = pending_response();
    odd_status["status"] = json!("done");
    let mut line_with_newline = pending_response();
    line_with_newline["agentsLine"] = json!("Agent Room:\nhttps://agent-room.test/r/aB3xYz");
    let mut other_code = pending_response();
    other_code["shareUrl"] = json!("https://agent-room.test/r/zzzzzz");
    let mut odd_result = pending_response();
    odd_result["lastCheck"]["result"] = json!("Not Found");
    for body in [
        bound_with_claim,
        pending_without_claim,
        odd_status,
        line_with_newline,
        other_code,
        odd_result,
    ] {
        let app = Router::new().route(
            "/devices/current/project-rooms/claims",
            post(move || {
                let body = body.clone();
                async move { (StatusCode::OK, Json(body)).into_response() }
            }),
        );
        let gateway = gateway(spawn_server(app).await, Arc::default());
        let failure = gateway
            .claim(&GithubRepository::parse("octo/hello").unwrap(), None)
            .await
            .unwrap_err();
        assert_eq!(
            failure.kind(),
            ProjectRoomFailureKind::InvalidControlPlaneResponse
        );
    }
}

fn pending_response() -> Value {
    json!({
        "claimId": CLAIM_ID,
        "repository": "Octo/Hello",
        "shareCode": "aB3xYz",
        "shareUrl": "https://agent-room.test/r/aB3xYz",
        "agentsLine": "Agent Room: this project's room is https://agent-room.test/r/aB3xYz. Agents join with `agent-room join https://agent-room.test/r/aB3xYz`, or over the network via https://agent-room.test/agents.txt",
        "badgeMarkdown": "[![Agent Room](https://agent-room.test/gh/Octo/Hello/badge.svg)](https://agent-room.test/r/aB3xYz)",
        "status": "pending",
        "catalogId": CATALOG_ID,
        "claimedAtUnixMs": 1_000,
        "lastCheck": { "atUnixMs": 2_000, "result": "not_in_repository" },
    })
}

fn bound_response() -> Value {
    let mut bound = pending_response();
    bound["claimId"] = Value::Null;
    bound["status"] = json!("bound");
    bound["claimedAtUnixMs"] = Value::Null;
    bound["lastCheck"] = Value::Null;
    bound
}

fn gateway(
    base_url: String,
    authorizer: Arc<RecordingAuthorizer>,
) -> ReqwestControlPlaneProjectRoomGateway {
    ReqwestControlPlaneProjectRoomGateway::new(
        &ControlPlaneHttpConfig {
            base_url,
            request_timeout: Duration::from_secs(2),
        },
        authorizer,
    )
    .unwrap()
}

async fn spawn_server(app: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{address}/")
}

fn claim_id() -> RepositoryClaimId {
    RepositoryClaimId::from_uuid(Uuid::parse_str(CLAIM_ID).unwrap())
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn secret(value: &str) -> SecretValue {
    SecretValue::new(value).unwrap()
}
