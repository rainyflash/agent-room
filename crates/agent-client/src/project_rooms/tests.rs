use std::{
    collections::{BTreeMap, VecDeque},
    sync::Mutex,
};

use agent_room_bridge_ipc::{IpcErrorCategory, IpcMethod, IpcRepositoryCheck, IpcResponse};

use super::*;
use crate::BridgeToolFuture;

const CLAIM_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e49";
const CATALOG_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";

#[test]
fn 认得出_github_的_https_和_ssh_两种写法() {
    for (remote, expected) in [
        (
            "https://github.com/octo-org/hello.world.git",
            "octo-org/hello.world",
        ),
        ("https://github.com/octo/hello", "octo/hello"),
        ("https://github.com/octo/hello/\n", "octo/hello"),
        ("HTTPS://GitHub.com/Octo/Hello.git", "Octo/Hello"),
        (
            "https://x-access-token:secret@github.com/octo/hello.git",
            "octo/hello",
        ),
        ("https://github.com:443/octo/hello.git", "octo/hello"),
        ("git@github.com:octo/hello.git", "octo/hello"),
        ("git@github.com:octo/hello", "octo/hello"),
        ("ssh://git@github.com/octo/hello.git", "octo/hello"),
        ("ssh://git@github.com:22/octo/hello", "octo/hello"),
    ] {
        assert_eq!(
            repository_from_remote(remote).as_deref(),
            Some(expected),
            "{remote}"
        );
    }
}

#[test]
fn 别的托管和别的写法认不出() {
    for remote in [
        "",
        "https://gitlab.com/octo/hello.git",
        "https://github.example.com/octo/hello.git",
        "https://github.com.evil.test/octo/hello.git",
        "http://github.com/octo/hello.git",
        "git://github.com/octo/hello.git",
        "git@gitlab.com:octo/hello.git",
        "https://github.com/octo",
        "https://github.com/octo/hello/tree/main",
        "git@github.com:octo/hello/extra.git",
        "C:\\work\\hello",
        "../hello",
        "file:///srv/hello.git",
    ] {
        assert_eq!(repository_from_remote(remote), None, "{remote}");
    }
}

/// 像 Bridge 一样：认领回 `claims` 里的下一份（没有了就回最后一份），核对按 `checks` 一份份回。
#[derive(Default)]
struct ClaimsBridge {
    claims: Mutex<VecDeque<IpcRepositoryClaim>>,
    checks: Mutex<VecDeque<Result<IpcRepositoryClaim, &'static str>>>,
    calls: Mutex<Vec<IpcMethod>>,
}

impl BridgeToolClient for ClaimsBridge {
    fn invoke(&self, method: IpcMethod) -> BridgeToolFuture<'_> {
        self.calls.lock().unwrap().push(method.clone());
        let answer = match method {
            IpcMethod::ClaimRepository(_) => {
                let mut claims = self.claims.lock().unwrap();
                let claim = if claims.len() > 1 {
                    claims.pop_front().unwrap()
                } else {
                    claims.front().cloned().unwrap()
                };
                Ok(claim)
            }
            IpcMethod::CheckRepositoryClaim(_) => self.checks.lock().unwrap().pop_front().unwrap(),
            other => panic!("没想到的请求 {other:?}"),
        };
        Box::pin(async move {
            answer
                .map(|claim| IpcResponse::RepositoryClaim { claim })
                .map_err(|code| {
                    BridgeToolFailure::new(code, IpcErrorCategory::Conflict, false, BTreeMap::new())
                })
        })
    }
}

fn pending(code: &str) -> IpcRepositoryClaim {
    IpcRepositoryClaim {
        claim_id: Some(CLAIM_ID.to_owned()),
        repository: "octo/hello".to_owned(),
        share_code: code.to_owned(),
        share_url: format!("https://agent-room.test/r/{code}"),
        agents_line: format!(
            "Agent Room: this project's room is https://agent-room.test/r/{code}."
        ),
        badge_markdown: format!("[![Agent Room](b)](https://agent-room.test/r/{code})"),
        status: IpcRepositoryClaimStatus::Pending,
        catalog_id: None,
        claimed_at_unix_ms: Some(1_000),
        last_check: Some(IpcRepositoryCheck {
            at_unix_ms: 2_000,
            result: "not_in_repository".to_owned(),
        }),
    }
}

fn bound(code: &str) -> IpcRepositoryClaim {
    IpcRepositoryClaim {
        claim_id: None,
        status: IpcRepositoryClaimStatus::Bound,
        catalog_id: Some(CATALOG_ID.to_owned()),
        claimed_at_unix_ms: None,
        last_check: None,
        ..pending(code)
    }
}

fn checks(bridge: &ClaimsBridge) -> usize {
    bridge
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|call| matches!(call, IpcMethod::CheckRepositoryClaim(request) if request.claim_id == CLAIM_ID))
        .count()
}

#[tokio::test(start_paused = true)]
async fn 每分钟核对一次_太快就等下一分钟_认上就停() {
    let bridge = ClaimsBridge::default();
    bridge.checks.lock().unwrap().extend([
        Err(CHECK_TOO_SOON),
        Ok(pending("aB3xYz")),
        Ok(bound("aB3xYz")),
    ]);
    let started = Instant::now();
    let outcome = wait_until_bound(&bridge, pending("aB3xYz"), None, WAIT_LIMIT)
        .await
        .unwrap();
    assert_eq!(outcome, ClaimWait::Bound(bound("aB3xYz")));
    assert_eq!(checks(&bridge), 3);
    assert_eq!(started.elapsed(), CHECK_INTERVAL * 2, "核对之间隔一分钟");
}

#[tokio::test(start_paused = true)]
async fn 等满十五分钟还没认上_回原来的认领() {
    let bridge = ClaimsBridge::default();
    bridge
        .checks
        .lock()
        .unwrap()
        .extend((0..20).map(|_| Ok(pending("aB3xYz"))));
    let started = Instant::now();
    let outcome = wait_until_bound(&bridge, pending("aB3xYz"), None, WAIT_LIMIT)
        .await
        .unwrap();
    assert_eq!(outcome, ClaimWait::Pending(pending("aB3xYz")));
    assert!(started.elapsed() <= WAIT_LIMIT);
    assert_eq!(checks(&bridge), 15, "第一次马上核对，之后每分钟一次");
}

#[tokio::test(start_paused = true)]
async fn 认领被后台认上删掉了_再认领拿到绑上的那间房() {
    let bridge = ClaimsBridge::default();
    bridge.claims.lock().unwrap().push_back(bound("aB3xYz"));
    bridge
        .checks
        .lock()
        .unwrap()
        .push_back(Err(CLAIM_NOT_FOUND));
    let outcome = wait_until_bound(
        &bridge,
        pending("aB3xYz"),
        Some(CATALOG_ID.to_owned()),
        WAIT_LIMIT,
    )
    .await
    .unwrap();
    assert_eq!(outcome, ClaimWait::Bound(bound("aB3xYz")));
    let calls = bridge.calls.lock().unwrap();
    assert!(
        matches!(&calls[1], IpcMethod::ClaimRepository(request)
            if request.repository == "octo/hello" && request.catalog_id.as_deref() == Some(CATALOG_ID)),
        "用同样的仓库和房间再认领"
    );
}

#[tokio::test(start_paused = true)]
async fn 认领作废了_再认领是新码就停下说清楚() {
    let bridge = ClaimsBridge::default();
    bridge.claims.lock().unwrap().push_back(pending("Zz9Yy8"));
    bridge
        .checks
        .lock()
        .unwrap()
        .push_back(Err(CLAIM_NOT_FOUND));
    let outcome = wait_until_bound(&bridge, pending("aB3xYz"), None, WAIT_LIMIT)
        .await
        .unwrap();
    assert_eq!(outcome, ClaimWait::Replaced(pending("Zz9Yy8")));
}

#[tokio::test(start_paused = true)]
async fn 别的拒绝原样返回() {
    let bridge = ClaimsBridge::default();
    bridge
        .checks
        .lock()
        .unwrap()
        .push_back(Err("bridge.project_room.unavailable"));
    let failure = wait_until_bound(&bridge, pending("aB3xYz"), None, WAIT_LIMIT)
        .await
        .unwrap_err();
    assert_eq!(failure.code(), "bridge.project_room.unavailable");
}

#[test]
fn 仓库名照_github_的规矩() {
    assert!(valid_repository("octo-org/hello.world"));
    for bad in [
        "octo",
        "octo/",
        "/hello",
        "octo/hello/x",
        "-octo/hello",
        "octo/..",
    ] {
        assert!(!valid_repository(bad), "{bad}");
    }
}

#[test]
fn 公开房间那一节写分享链接和网络的入口_服务器认得出那一串() {
    let section = agents_section(SectionRoom::Shared {
        share_url: "https://agent-room.test/r/aB3xYz",
    });
    assert!(section.starts_with(SECTION_START));
    assert!(section.ends_with(SECTION_END));
    assert!(section.contains("\n## Agent Room\n"));
    assert!(section.contains("`agent-room join --room https://agent-room.test/r/aB3xYz`"));
    assert!(section.contains("https://agent-room.test/agents.txt"));
    assert!(section.contains("agent-room listen"));
    // 服务器核对时找 `<站点>/r/<码>` 后面不紧跟字母数字的那一串。
    for (index, _) in section.match_indices("/r/aB3xYz") {
        let next = section[index + "/r/aB3xYz".len()..].chars().next();
        assert!(!next.is_some_and(char::is_alphanumeric), "{next:?}");
    }
    assert!(!section.ends_with('\n'));
}

#[test]
fn 绑不上的房间只写名字_名字写进命令会坏时用房间号() {
    let named = agents_section(SectionRoom::Named {
        address: "Release notes/Design",
        catalog_id: CATALOG_ID,
    });
    assert!(named.contains("in the room “Release notes/Design”."));
    assert!(named.contains("`agent-room join --room \"Release notes/Design\"`"));
    assert!(named.contains("ask the room's owner for an invite"));
    assert!(!named.contains("/r/"), "不写分享链接");
    assert!(!named.contains("agents.txt"), "私人房间没有网络那半句");

    for address in [
        "Say \"hi\"",
        "a`b",
        "$HOME",
        r"back\slash",
        "two\nlines",
        "  ",
    ] {
        let section = agents_section(SectionRoom::Named {
            address,
            catalog_id: CATALOG_ID,
        });
        assert!(
            section.contains(&format!("`agent-room join --room {CATALOG_ID}`")),
            "{address:?}"
        );
        assert_eq!(
            section.lines().count(),
            10,
            "名字里的换行不进文件：{address:?}"
        );
    }
}

#[test]
fn 只有不是你的和私人房间算绑不了() {
    let failure = |code: &str| {
        BridgeToolFailure::new(code, IpcErrorCategory::Conflict, false, BTreeMap::new())
    };
    assert!(room_not_bindable(&failure("bridge.project_room.forbidden")));
    assert!(room_not_bindable(&failure(
        "bridge.project_room.room_not_eligible"
    )));
    for code in [
        "bridge.project_room.room_already_bound",
        "bridge.project_room.too_many_claims",
        "bridge.project_room.not_found",
    ] {
        assert!(!room_not_bindable(&failure(code)), "{code}");
    }
}
