//! `agent_room_claim_repository`：只认领、核对，回那一节和徽章；绑不上的房间只回那一节说明。不开会话、不碰文件。

use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};

use agent_room_agent_client::{BridgeToolClient, BridgeToolFuture};
use agent_room_bridge_ipc::{
    IpcClaimRepositoryRequest, IpcMethod, IpcRepositoryCheck, IpcResponse, IpcRoomKind,
    IpcRoomSummary,
};
use tokio::time::Instant;

use super::*;

const CLAIM_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e49";
const LOUNGE: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";

/// 认领回 `claims` 里的下一份，核对按 `checks` 一份份回；房间名单里只有“Octo lounge”。
#[derive(Default)]
struct ClaimsBridge {
    claims: Mutex<VecDeque<Result<IpcRepositoryClaim, BridgeToolFailure>>>,
    checks: Mutex<VecDeque<IpcRepositoryClaim>>,
    calls: Mutex<Vec<IpcMethod>>,
}

impl ClaimsBridge {
    fn claiming(
        claims: impl IntoIterator<Item = Result<IpcRepositoryClaim, &'static str>>,
    ) -> Arc<Self> {
        let bridge = Self::default();
        bridge
            .claims
            .lock()
            .unwrap()
            .extend(claims.into_iter().map(|claim| claim.map_err(refusal)));
        Arc::new(bridge)
    }

    fn claim_requests(&self) -> Vec<IpcClaimRepositoryRequest> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter_map(|call| match call {
                IpcMethod::ClaimRepository(request) => Some(request.clone()),
                _ => None,
            })
            .collect()
    }

    fn checks_made(&self) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| matches!(call, IpcMethod::CheckRepositoryClaim(_)))
            .count()
    }
}

impl BridgeToolClient for ClaimsBridge {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        self.calls.lock().unwrap().push(method.clone());
        let answer = match method {
            IpcMethod::ListRooms => Ok(IpcResponse::Rooms {
                rooms: vec![IpcRoomSummary {
                    kind: IpcRoomKind::PrivateRoom,
                    catalog_id: LOUNGE.into(),
                    matrix_room_id: Some("!lounge:test.invalid".into()),
                    name: "Octo lounge".into(),
                    slug: None,
                    membership: None,
                    parent: None,
                    category: None,
                    access: None,
                }],
            }),
            IpcMethod::ClaimRepository(_) => self
                .claims
                .lock()
                .unwrap()
                .pop_front()
                .expect("认领的回答备好了")
                .map(|claim| IpcResponse::RepositoryClaim { claim }),
            IpcMethod::CheckRepositoryClaim(_) => Ok(IpcResponse::RepositoryClaim {
                claim: self
                    .checks
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("核对的回答备好了"),
            }),
            other => panic!("认领仓库不该调用 {other:?}"),
        };
        Box::pin(async move { answer })
    }
}

fn refusal(code: &str) -> BridgeToolFailure {
    BridgeToolFailure::new(code, IpcErrorCategory::Conflict, false, BTreeMap::new())
}

fn pending() -> IpcRepositoryClaim {
    IpcRepositoryClaim {
        claim_id: Some(CLAIM_ID.to_owned()),
        repository: "octo/hello".to_owned(),
        share_code: "aB3xYz".to_owned(),
        share_url: "https://agent-room.test/r/aB3xYz".to_owned(),
        agents_line: "Agent Room: this project's room is https://agent-room.test/r/aB3xYz."
            .to_owned(),
        badge_markdown: "[![Agent Room](b)](https://agent-room.test/r/aB3xYz)".to_owned(),
        status: IpcRepositoryClaimStatus::Pending,
        catalog_id: None,
        claimed_at_unix_ms: Some(1_000),
        last_check: Some(IpcRepositoryCheck {
            at_unix_ms: 2_000,
            result: "not_in_repository".to_owned(),
        }),
    }
}

fn bound() -> IpcRepositoryClaim {
    IpcRepositoryClaim {
        claim_id: None,
        status: IpcRepositoryClaimStatus::Bound,
        catalog_id: Some(LOUNGE.to_owned()),
        claimed_at_unix_ms: None,
        last_check: None,
        ..pending()
    }
}

fn input(repo: &str, room: Option<&str>, wait: bool) -> Parameters<ClaimRepositoryInput> {
    Parameters(ClaimRepositoryInput {
        repo: (!repo.is_empty()).then(|| repo.to_owned()),
        room: room.map(str::to_owned),
        wait,
    })
}

fn structured(result: &CallToolResult) -> &Value {
    result.structured_content.as_ref().expect("有结构化的回答")
}

#[tokio::test]
async fn 认领回那一节和徽章_由_agent_自己写进文件() {
    let bridge = ClaimsBridge::claiming([Ok(pending())]);
    let server = AgentRoomMcpServer::new(bridge.clone());

    let result = server
        .claim_repository(input(" octo/hello ", None, false))
        .await;

    assert_eq!(result.is_error, Some(false));
    let value = structured(&result);
    assert_eq!(value["status"], "pending");
    assert_eq!(value["repository"], "octo/hello");
    assert_eq!(value["claimId"], CLAIM_ID);
    assert_eq!(
        value["agentsSection"],
        project_rooms::agents_section(SectionRoom::Shared {
            share_url: &pending().share_url
        })
    );
    assert!(value.get("agentsLine").is_none(), "只给那一节，免得写两份");
    assert_eq!(value["readmeBadge"], pending().badge_markdown);
    assert_eq!(value["shareUrl"], pending().share_url);
    assert_eq!(value["lastCheck"]["result"], "not_in_repository");
    assert_eq!(value["next"], PUSH);
    assert!(value.get("agentsFile").is_none(), "不碰文件");
    assert_eq!(
        bridge.claim_requests(),
        [IpcClaimRepositoryRequest {
            repository: "octo/hello".into(),
            catalog_id: None,
        }]
    );
    assert_eq!(bridge.checks_made(), 0, "不带 wait 不核对");
}

#[tokio::test]
async fn 不像_github_仓库名的在碰_bridge_之前就拒绝() {
    let bridge = ClaimsBridge::claiming([]);
    let server = AgentRoomMcpServer::new(bridge.clone());

    for repo in ["octo", "https://github.com/octo/hello", "octo/hello/x"] {
        let result = server.claim_repository(input(repo, None, false)).await;
        assert_eq!(result.is_error, Some(true), "{repo}");
        assert_eq!(structured(&result)["code"], "bridge.ipc.repository_invalid");
        assert_eq!(structured(&result)["message"], INVALID_REPOSITORY);
    }
    assert!(bridge.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 给了_room_按名单换成房间号再认领() {
    let bridge = ClaimsBridge::claiming([Ok(pending())]);
    let server = AgentRoomMcpServer::new(bridge.clone());

    let result = server
        .claim_repository(input("octo/hello", Some("Octo lounge"), false))
        .await;

    assert_eq!(result.is_error, Some(false));
    assert_eq!(
        bridge.claim_requests()[0].catalog_id.as_deref(),
        Some(LOUNGE)
    );

    let missing = server
        .claim_repository(input("octo/hello", Some("Nowhere"), false))
        .await;
    assert_eq!(missing.is_error, Some(true));
    assert_eq!(structured(&missing)["code"], "agent.join.room_not_found");
    assert_eq!(bridge.claim_requests().len(), 1, "找不到房间就不认领");
}

#[tokio::test(start_paused = true)]
async fn 带_wait_每分钟核对一次_认上了说怎么进房间() {
    let bridge = ClaimsBridge::claiming([Ok(pending())]);
    bridge.checks.lock().unwrap().extend([pending(), bound()]);
    let server = AgentRoomMcpServer::new(bridge.clone());
    let started = Instant::now();

    let result = server
        .claim_repository(input("octo/hello", None, true))
        .await;

    let value = structured(&result);
    assert_eq!(value["status"], "bound");
    assert_eq!(value["catalogId"], LOUNGE);
    assert!(value.get("claimId").is_none());
    assert_eq!(value["next"], BOUND);
    assert_eq!(bridge.checks_made(), 2);
    assert_eq!(started.elapsed(), project_rooms::CHECK_INTERVAL);
}

#[tokio::test(start_paused = true)]
async fn 带_wait_等满还没认上_说再调一次接着等() {
    let bridge = ClaimsBridge::claiming([Ok(pending())]);
    bridge
        .checks
        .lock()
        .unwrap()
        .extend((0..20).map(|_| pending()));
    let server = AgentRoomMcpServer::new(bridge.clone());

    let result = server
        .claim_repository(input("octo/hello", None, true))
        .await;

    assert_eq!(structured(&result)["status"], "pending");
    assert_eq!(structured(&result)["next"], STILL_WAITING);
    assert_eq!(bridge.checks_made(), 15);
}

#[tokio::test]
async fn 仓库已经绑了房间_直接回那间房_不再核对() {
    let bridge = ClaimsBridge::claiming([Ok(bound())]);
    let server = AgentRoomMcpServer::new(bridge.clone());

    let result = server
        .claim_repository(input("octo/hello", None, true))
        .await;

    assert_eq!(structured(&result)["status"], "bound");
    assert_eq!(structured(&result)["shareUrl"], bound().share_url);
    assert_eq!(structured(&result)["next"], BOUND);
    assert_eq!(bridge.checks_made(), 0);
}

#[tokio::test]
async fn 控制面拒绝时说清楚怎么办_旧版_bridge_说要更新() {
    let bridge = ClaimsBridge::claiming([
        Err("bridge.project_room.too_many_claims"),
        Err("bridge.ipc.protocol_failed"),
    ]);
    let server = AgentRoomMcpServer::new(bridge);

    let refused = server
        .claim_repository(input("octo/hello", None, false))
        .await;
    assert_eq!(refused.is_error, Some(true));
    let value = structured(&refused);
    assert_eq!(value["code"], "bridge.project_room.too_many_claims");
    assert!(
        value["message"]
            .as_str()
            .unwrap()
            .contains("3 claims waiting"),
        "{value}"
    );

    let outdated = server
        .claim_repository(input("octo/hello", None, false))
        .await;
    assert_eq!(structured(&outdated)["code"], BRIDGE_OUTDATED);
    assert_eq!(structured(&outdated)["message"], OUTDATED_BRIDGE);
}

#[test]
fn 认领的每种拒绝都有说法() {
    for code in [
        "bridge.ipc.repository_invalid",
        "bridge.project_room.invalid_repository",
        "bridge.project_room.too_many_claims",
        "bridge.project_room.repository_taken",
        "bridge.project_room.room_not_eligible",
        "bridge.project_room.forbidden",
        "bridge.project_room.room_already_bound",
        "bridge.project_room.check_too_soon",
        "bridge.project_room.not_found",
        "bridge.project_room.unsupported",
        "bridge.project_room.unavailable",
        "bridge.project_room.conflict",
    ] {
        assert!(recovery_for(code).is_some(), "{code}");
    }
}

#[tokio::test]
async fn 绑不上的房间只回那一节说明_不认领() {
    for code in [
        "bridge.project_room.room_not_eligible",
        "bridge.project_room.forbidden",
    ] {
        let bridge = ClaimsBridge::claiming([Err(code)]);
        let server = AgentRoomMcpServer::new(bridge.clone());

        let result = server
            .claim_repository(input("octo/hello", Some("Octo lounge"), false))
            .await;

        assert_eq!(result.is_error, Some(false), "{code}");
        let value = structured(&result);
        assert_eq!(value["status"], "guide");
        assert_eq!(value["room"], "Octo lounge");
        assert_eq!(value["next"], GUIDE_NOT_BINDABLE);
        let section = value["agentsSection"].as_str().unwrap();
        assert!(section.contains("`agent-room join --room \"Octo lounge\"`"));
        assert!(!section.contains("/r/"), "不写链接");
        assert_eq!(bridge.checks_made(), 0);
    }
}

#[tokio::test]
async fn 不在_github_上的仓库带了_room_也回说明_没带_room_照旧要_repo() {
    let bridge = ClaimsBridge::claiming([]);
    let server = AgentRoomMcpServer::new(bridge.clone());

    let result = server
        .claim_repository(input("", Some("Octo lounge"), false))
        .await;
    assert_eq!(result.is_error, Some(false));
    assert_eq!(structured(&result)["status"], "guide");
    assert_eq!(structured(&result)["next"], GUIDE_NOT_GITHUB);
    assert!(bridge.claim_requests().is_empty(), "不认领");

    let missing = server.claim_repository(input("", None, false)).await;
    assert_eq!(missing.is_error, Some(true));
    assert_eq!(
        structured(&missing)["code"],
        "bridge.ipc.repository_invalid"
    );
}
