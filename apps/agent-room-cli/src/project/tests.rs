use std::{
    collections::{BTreeMap, VecDeque},
    sync::Mutex,
};

use agent_room_agent_client::{BridgeToolFailure, BridgeToolFuture};
use agent_room_bridge_ipc::{
    IpcErrorCategory, IpcRepositoryCheck, IpcRoomKind, IpcRoomMembership, IpcRoomSummary,
};

use super::*;

const CLAIM_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e49";
const CATALOG_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
const SHARE_URL: &str = "https://agent-room.test/r/aB3xYz";
const LINE: &str = "Agent Room: this project's room is https://agent-room.test/r/aB3xYz. Agents join with `agent-room join https://agent-room.test/r/aB3xYz`, or over the network via https://agent-room.test/agents.txt";

/// 像 Bridge 一样：同一个仓库再认领回同一个认领；核对按 `checks` 一份份回，没有了就还在等。
#[derive(Default)]
struct ProjectBridge {
    bound: Mutex<bool>,
    checks: Mutex<VecDeque<IpcRepositoryClaim>>,
    calls: Mutex<Vec<IpcMethod>>,
    refuse: Mutex<Option<&'static str>>,
}

impl ProjectBridge {
    fn claims(&self) -> Vec<(String, Option<String>)> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter_map(|call| match call {
                IpcMethod::ClaimRepository(request) => {
                    Some((request.repository.clone(), request.catalog_id.clone()))
                }
                _ => None,
            })
            .collect()
    }

    fn checked(&self) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| matches!(call, IpcMethod::CheckRepositoryClaim(request) if request.claim_id == CLAIM_ID))
            .count()
    }
}

impl BridgeToolClient for ProjectBridge {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        self.calls.lock().unwrap().push(method.clone());
        let refuse = *self.refuse.lock().unwrap();
        let answer = match (refuse, method) {
            (Some(code), _) => Err(code),
            (None, IpcMethod::ClaimRepository(request)) => Ok(IpcResponse::RepositoryClaim {
                claim: if *self.bound.lock().unwrap() {
                    bound()
                } else {
                    pending(&request.repository)
                },
            }),
            (None, IpcMethod::CheckRepositoryClaim(_)) => {
                let next = self.checks.lock().unwrap().pop_front();
                if next
                    .as_ref()
                    .is_some_and(|claim| claim.status == IpcRepositoryClaimStatus::Bound)
                {
                    *self.bound.lock().unwrap() = true;
                }
                Ok(IpcResponse::RepositoryClaim {
                    claim: next.unwrap_or_else(|| pending("octo/hello")),
                })
            }
            (None, IpcMethod::ListRooms) => Ok(IpcResponse::Rooms {
                rooms: vec![IpcRoomSummary {
                    kind: IpcRoomKind::PrivateRoom,
                    catalog_id: CATALOG_ID.into(),
                    matrix_room_id: Some("!release:test.invalid".into()),
                    name: "Release notes".into(),
                    slug: Some("aB3xYz".into()),
                    membership: Some(IpcRoomMembership::Joined),
                    parent: None,
                    category: None,
                    access: None,
                }],
            }),
            (None, other) => panic!("没想到的请求 {other:?}"),
        };
        Box::pin(async move {
            answer.map_err(|code| {
                BridgeToolFailure::new(code, IpcErrorCategory::Conflict, false, BTreeMap::new())
            })
        })
    }
}

fn pending(repository: &str) -> IpcRepositoryClaim {
    IpcRepositoryClaim {
        claim_id: Some(CLAIM_ID.into()),
        repository: repository.into(),
        share_code: "aB3xYz".into(),
        share_url: SHARE_URL.into(),
        agents_line: LINE.into(),
        badge_markdown: format!(
            "[![Agent Room](https://agent-room.test/gh/{repository}/badge.svg)]({SHARE_URL})"
        ),
        status: IpcRepositoryClaimStatus::Pending,
        catalog_id: None,
        claimed_at_unix_ms: Some(1_000),
        last_check: Some(IpcRepositoryCheck {
            at_unix_ms: 2_000,
            result: "not_in_repository".into(),
        }),
    }
}

fn bound() -> IpcRepositoryClaim {
    IpcRepositoryClaim {
        claim_id: None,
        status: IpcRepositoryClaimStatus::Bound,
        catalog_id: Some(CATALOG_ID.into()),
        claimed_at_unix_ms: None,
        last_check: None,
        ..pending("octo/hello")
    }
}

fn args(wait: bool) -> ProjectArgs {
    ProjectArgs {
        repo: None,
        room: None,
        wait,
        name: None,
    }
}

fn checkout(root: &Path, origin: Option<&str>) -> Checkout {
    Checkout {
        root: Some(root.to_path_buf()),
        origin: origin.map(str::to_owned),
    }
}

#[test]
fn 仓库从_origin_认_https_和_ssh_都行_别的要_repo() {
    let at = |origin: Option<&str>| Checkout {
        root: None,
        origin: origin.map(str::to_owned),
    };
    assert_eq!(
        repository(&at(Some("https://github.com/octo/hello.git")), None).unwrap(),
        "octo/hello"
    );
    assert_eq!(
        repository(&at(Some("git@github.com:octo/hello.git")), None).unwrap(),
        "octo/hello"
    );
    for origin in [Some("https://gitlab.com/octo/hello.git"), None] {
        assert_eq!(
            repository(&at(origin), None).unwrap_err().code,
            "cli.project.repository_unknown",
            "{origin:?}"
        );
    }
    assert_eq!(
        repository(
            &at(Some("https://gitlab.com/octo/hello.git")),
            Some("octo/mirror")
        )
        .unwrap(),
        "octo/mirror",
        "--repo 盖过 origin"
    );
    assert_eq!(
        repository(&at(None), Some("not a repo")).unwrap_err().code,
        "cli.project.repository_invalid"
    );
}

#[test]
fn 没有_agents_md_就新建() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("AGENTS.md");
    assert_eq!(
        write_agents_line(&path, LINE, SHARE_URL).unwrap(),
        AgentsLine::Created
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("{LINE}\n"));
}

#[test]
fn 已有的_agents_md_加在末尾_前面空一行_换行照原来的() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("AGENTS.md");
    for (before, after) in [
        (
            "# Agents\nRun tests.",
            format!("# Agents\nRun tests.\n\n{LINE}\n"),
        ),
        ("# Agents\n", format!("# Agents\n\n{LINE}\n")),
        ("# Agents\n\n", format!("# Agents\n\n{LINE}\n")),
        (
            "# Agents\r\nRun tests.\r\n",
            format!("# Agents\r\nRun tests.\r\n\r\n{LINE}\r\n"),
        ),
        ("", format!("{LINE}\n")),
    ] {
        std::fs::write(&path, before).unwrap();
        assert_eq!(
            write_agents_line(&path, LINE, SHARE_URL).unwrap(),
            AgentsLine::Appended,
            "{before:?}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), after, "{before:?}");
    }
}

#[test]
fn 已经有_agent_room_的链接就不再写_照实说是哪个() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("AGENTS.md");
    for (content, link) in [
        (format!("# Agents\n{LINE}\n"), SHARE_URL),
        (
            "See [our room](https://agent-room.test/r/Zz9Yy8).\n".to_owned(),
            "https://agent-room.test/r/Zz9Yy8",
        ),
    ] {
        std::fs::write(&path, &content).unwrap();
        assert_eq!(
            write_agents_line(&path, LINE, SHARE_URL).unwrap(),
            AgentsLine::AlreadyLinked { link: link.into() }
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content, "没动文件");
    }
}

#[tokio::test(start_paused = true)]
async fn 不带_wait_只认领和写那一行_带上徽章和下一步() {
    let directory = tempfile::tempdir().unwrap();
    let bridge = ProjectBridge::default();
    let outcome = project(
        &bridge,
        &checkout(directory.path(), Some("https://github.com/octo/hello.git")),
        args(false),
    )
    .await
    .unwrap();
    assert_eq!(outcome.entering(), None);
    assert_eq!(outcome.value["status"], "pending");
    assert_eq!(outcome.value["claimId"], CLAIM_ID);
    assert_eq!(outcome.value["agentsLine"], LINE);
    assert_eq!(
        outcome.value["readmeBadge"],
        format!("[![Agent Room](https://agent-room.test/gh/octo/hello/badge.svg)]({SHARE_URL})")
    );
    assert_eq!(outcome.value["agentsFile"]["result"], "created");
    assert_eq!(outcome.value["next"], PUSH);
    assert_eq!(bridge.claims(), [("octo/hello".to_owned(), None)]);
    assert_eq!(bridge.checked(), 0, "不带 --wait 不核对");
}

#[tokio::test(start_paused = true)]
async fn 再跑一次接着等原来的认领_那一行不写第二遍_认上就进房间() {
    let directory = tempfile::tempdir().unwrap();
    let here = checkout(directory.path(), Some("git@github.com:octo/hello.git"));
    let bridge = ProjectBridge::default();

    let first = project(&bridge, &here, args(true)).await.unwrap();
    assert_eq!(first.entering(), None, "15 分钟没认上");
    assert_eq!(first.value["next"], STILL_WAITING);
    assert_eq!(bridge.checked(), 15);

    bridge
        .checks
        .lock()
        .unwrap()
        .extend([pending("octo/hello"), bound()]);
    let second = project(&bridge, &here, args(true)).await.unwrap();
    assert_eq!(second.entering().as_deref(), Some(SHARE_URL));
    assert_eq!(second.value["status"], "bound");
    assert_eq!(second.value["catalogId"], CATALOG_ID);
    assert_eq!(
        second.value["agentsFile"]["result"], "already_linked",
        "第二次认领还是同一个码，那一行已经在了"
    );
    assert_eq!(second.value["agentsFile"]["sameRoom"], true);
    assert!(second.value.get("next").is_none());
    assert_eq!(
        bridge.claims(),
        [
            ("octo/hello".to_owned(), None),
            ("octo/hello".to_owned(), None)
        ]
    );
    assert_eq!(bridge.checked(), 17, "核对的一直是同一个认领");
    let agents = std::fs::read_to_string(directory.path().join("AGENTS.md")).unwrap();
    assert_eq!(agents.matches("/r/aB3xYz").count(), 2, "只写了一行");

    // 认上以后再跑：只回那间房，不再写、不再核对。
    let third = project(&bridge, &here, args(false)).await.unwrap();
    assert_eq!(third.value["status"], "bound");
    assert!(third.value.get("agentsFile").is_none());
    assert!(third.value["next"].as_str().unwrap().contains(SHARE_URL));
    assert_eq!(bridge.checked(), 17);
}

#[tokio::test(start_paused = true)]
async fn 带_room_绑名下那间房_按名字换成房间号() {
    let directory = tempfile::tempdir().unwrap();
    let bridge = ProjectBridge::default();
    let outcome = project(
        &bridge,
        &checkout(directory.path(), None),
        ProjectArgs {
            repo: Some("octo/hello".into()),
            room: Some("Release notes".into()),
            wait: false,
            name: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.value["status"], "pending");
    assert_eq!(
        bridge.claims(),
        [("octo/hello".to_owned(), Some(CATALOG_ID.to_owned()))]
    );

    let missing = project(
        &bridge,
        &checkout(directory.path(), None),
        ProjectArgs {
            repo: Some("octo/hello".into()),
            room: Some("No such room".into()),
            wait: false,
            name: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(missing.code, "cli.room_not_found");
}

#[tokio::test(start_paused = true)]
async fn 不在_git_仓库里不写文件_控制面不让时照实说() {
    let bridge = ProjectBridge::default();
    let outcome = project(
        &bridge,
        &Checkout::default(),
        ProjectArgs {
            repo: Some("octo/hello".into()),
            ..args(false)
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.value["agentsFile"]["result"], "skipped");

    *bridge.refuse.lock().unwrap() = Some("bridge.project_room.too_many_claims");
    let refused = project(
        &bridge,
        &Checkout::default(),
        ProjectArgs {
            repo: Some("octo/world".into()),
            ..args(false)
        },
    )
    .await
    .unwrap_err();
    assert_eq!(refused.code, "bridge.project_room.too_many_claims");
    assert!(refused.hint.contains("3 claims"));
    *bridge.refuse.lock().unwrap() = Some("bridge.ipc.protocol_failed");
    let outdated = project(
        &bridge,
        &Checkout::default(),
        ProjectArgs {
            repo: Some("octo/world".into()),
            ..args(false)
        },
    )
    .await
    .unwrap_err();
    assert_eq!(outdated.code, "cli.project.bridge_outdated");
}
