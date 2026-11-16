//! `agent-room project`（`specs/redesign/project-rooms.md` 的 3P-4b）：在仓库目录里替这台电脑的账户认领
//! GitHub 仓库，把那一行写进仓库根目录的 `AGENTS.md`，打印 README 徽章。不替人提交、不推送：仓库是人的。
//!
//! 带 `--wait` 每分钟核对一次，最多 15 分钟，认上以后像 `join` 一样进房间。再跑一次同一条命令接着等
//! 原来那个认领（控制面对同一个人同一个仓库回同一个认领）。

use std::{
    io::Write as _,
    path::{Path, PathBuf},
};

use agent_room_agent_client::{
    BridgeToolClient,
    project_rooms::{self, ClaimWait, WAIT_LIMIT},
    rooms::bridge_outdated,
};
use agent_room_bridge_ipc::{IpcMethod, IpcRepositoryClaim, IpcRepositoryClaimStatus, IpcResponse};
use serde_json::{Value, json};

use crate::{
    access::{self, pick_room},
    call,
    cli::{Command, ProjectArgs},
    output::{CliFailure as Failure, CliResult as Result, success},
};

#[cfg(test)]
mod tests;

const AGENTS_FILE: &str = "AGENTS.md";
const PUSH: &str = "Commit and push AGENTS.md (and the README badge if you want it) to the default branch; it's usually recognised within a few minutes. Run agent-room project --wait to wait and enter the room once it is; rerunning picks up the same claim.";
const STILL_WAITING: &str = "Not recognised yet after 15 minutes. Check that the line is pushed to the default branch on github.com, in AGENTS.md or README.md; run agent-room project --wait again to keep waiting with the same claim.";
const REPLACED: &str = "The earlier claim lapsed, so this is a new claim with a new link: replace the Agent Room line in AGENTS.md with agentsLine (and the README badge with readmeBadge), then commit and push again.";

/// 在哪个仓库目录里：根目录（不在 git 里时没有）和 `origin` 的地址。
#[derive(Debug, Clone, Default)]
pub(crate) struct Checkout {
    pub(crate) root: Option<PathBuf>,
    pub(crate) origin: Option<String>,
}

impl Checkout {
    /// 问 git 这个目录在哪个仓库里、`origin` 指向哪里。git 不在、不是仓库时都没有。
    async fn discover(directory: &Path) -> Self {
        Self {
            root: git(directory, &["rev-parse", "--show-toplevel"])
                .await
                .map(PathBuf::from),
            origin: git(directory, &["remote", "get-url", "origin"]).await,
        }
    }
}

async fn git(directory: &Path, args: &[&str]) -> Option<String> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(directory)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub(crate) async fn run(
    backend: &dyn BridgeToolClient,
    root: &Path,
    service: &str,
    selected: Option<String>,
    args: ProjectArgs,
) -> Result<()> {
    let directory =
        std::env::current_dir().map_err(|_| Failure::local("cli.project.directory_unreadable"))?;
    let checkout = Checkout::discover(&directory).await;
    let name = args.name.clone();
    let outcome = project(backend, &checkout, args).await?;
    match outcome.entering() {
        Some(share_url) => {
            let join = Command::Join {
                invite: None,
                room: Some(share_url),
                code: None,
                name,
            };
            access::run_with(
                backend,
                root,
                service,
                selected,
                join,
                Some(("project", outcome.value)),
            )
            .await
        }
        None => success(outcome.value),
    }
}

/// 认领的结果：交出去的 JSON，和要不要进房间。
#[derive(Debug)]
pub(crate) struct Outcome {
    pub(crate) value: Value,
    pub(crate) claim: IpcRepositoryClaim,
    pub(crate) wait: bool,
}

impl Outcome {
    /// 带 `--wait` 且认上了：进它绑上的那间房（按分享链接，和 `join` 一样）。
    fn entering(&self) -> Option<String> {
        (self.wait && self.claim.status == IpcRepositoryClaimStatus::Bound)
            .then(|| self.claim.share_url.clone())
    }
}

/// 认领、写 `AGENTS.md`、带 `--wait` 时等到认上。不进房间，好单独测。
pub(crate) async fn project(
    backend: &dyn BridgeToolClient,
    checkout: &Checkout,
    args: ProjectArgs,
) -> Result<Outcome> {
    let repository = repository(checkout, args.repo.as_deref())?;
    let target = match args.room.as_deref().map(str::trim) {
        Some(room) if !room.is_empty() => Some(room_number(backend, room).await?),
        _ => None,
    };
    let mut claim = project_rooms::claim(backend, &repository, target.clone())
        .await
        .map_err(failure)?;
    // 认上了的仓库只回那间房，不再往仓库里写东西。
    let agents_file = (claim.status == IpcRepositoryClaimStatus::Pending)
        .then(|| agents_file(checkout, &claim))
        .transpose()?;
    let mut next = None;
    if args.wait && claim.status == IpcRepositoryClaimStatus::Pending {
        match project_rooms::wait_until_bound(backend, claim, target, WAIT_LIMIT)
            .await
            .map_err(failure)?
        {
            ClaimWait::Bound(bound) => claim = bound,
            ClaimWait::Pending(pending) => {
                claim = pending;
                next = Some(STILL_WAITING);
            }
            ClaimWait::Replaced(replaced) => {
                claim = replaced;
                next = Some(REPLACED);
            }
        }
    }
    let next = match (claim.status, next) {
        (IpcRepositoryClaimStatus::Bound, _) if args.wait => None,
        (IpcRepositoryClaimStatus::Bound, _) => Some(format!(
            "The repository is bound to the room at {}. Enter it with agent-room join --room {} (or project --wait).",
            claim.share_url, claim.share_url
        )),
        (IpcRepositoryClaimStatus::Pending, Some(next)) => Some(next.to_owned()),
        (IpcRepositoryClaimStatus::Pending, None) => Some(PUSH.to_owned()),
    };
    let mut value = json!({
        "repository": claim.repository,
        "status": claim.status,
        "shareUrl": claim.share_url,
        "agentsLine": claim.agents_line,
        "readmeBadge": claim.badge_markdown,
    });
    for (key, field) in [
        ("claimId", claim.claim_id.clone().map(Value::from)),
        ("catalogId", claim.catalog_id.clone().map(Value::from)),
        (
            "lastCheck",
            claim
                .last_check
                .as_ref()
                .map(|check| json!({"atUnixMs": check.at_unix_ms, "result": check.result})),
        ),
        ("agentsFile", agents_file),
        ("next", next.map(Value::from)),
    ] {
        if let Some(field) = field {
            value[key] = field;
        }
    }
    Ok(Outcome {
        value,
        claim,
        wait: args.wait,
    })
}

/// `--repo` 给了就用它，不然从 `origin` 认；只认 github.com。
fn repository(checkout: &Checkout, given: Option<&str>) -> Result<String> {
    if let Some(given) = given.map(str::trim) {
        return if project_rooms::valid_repository(given) {
            Ok(given.to_owned())
        } else {
            Err(Failure::validation("cli.project.repository_invalid"))
        };
    }
    checkout
        .origin
        .as_deref()
        .and_then(project_rooms::repository_from_remote)
        .ok_or_else(|| Failure::validation("cli.project.repository_unknown"))
}

/// `--room` 的名字或房间号换成房间号；能不能绑（公开着、是你的）由控制面把关。
async fn room_number(backend: &dyn BridgeToolClient, room: &str) -> Result<String> {
    let IpcResponse::Rooms { rooms } = call(backend, IpcMethod::ListRooms).await? else {
        return Err(Failure::local("cli.response_invalid"));
    };
    Ok(pick_room(&rooms, room)?.catalog_id.clone())
}

/// 写那一行；不在 git 仓库里时不写，照实说。
fn agents_file(checkout: &Checkout, claim: &IpcRepositoryClaim) -> Result<Value> {
    let Some(root) = &checkout.root else {
        return Ok(json!({
            "result": "skipped",
            "reason": "Not inside a git checkout: add agentsLine to AGENTS.md at the repository root yourself.",
        }));
    };
    let path = root.join(AGENTS_FILE);
    let written = write_agents_line(&path, &claim.agents_line, &claim.share_url)?;
    let mut value = json!({"path": path.display().to_string()});
    match written {
        AgentsLine::Created => value["result"] = "created".into(),
        AgentsLine::Appended => value["result"] = "appended".into(),
        AgentsLine::AlreadyLinked { link } => {
            value["result"] = "already_linked".into();
            value["sameRoom"] = (link == claim.share_url).into();
            value["link"] = link.into();
        }
    }
    Ok(value)
}

/// `AGENTS.md` 那一行写成了什么样。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentsLine {
    /// 没有这个文件，新建了。
    Created,
    /// 加在已有文件的末尾，前面空一行。
    Appended,
    /// 里面已经有 Agent Room 的链接（可能是这间房的，也可能是别的），没再写。
    AlreadyLinked { link: String },
}

/// 把那一行写进 `path`：没有就新建；已经有这个站点的 Agent Room 链接（`<站点>/r/…`）就不再写，回那个
/// 链接；不然加在末尾，前面空一行，换行照文件原来的（`\r\n` 或 `\n`）。
pub(crate) fn write_agents_line(path: &Path, line: &str, share_url: &str) -> Result<AgentsLine> {
    let existing = match std::fs::read_to_string(path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::write(path, format!("{line}\n"))
                .map_err(|_| Failure::local("cli.project.agents_file_unwritable"))?;
            return Ok(AgentsLine::Created);
        }
        Err(_) => return Err(Failure::local("cli.project.agents_file_unreadable")),
    };
    if let Some(link) = existing_link(&existing, share_url) {
        return Ok(AgentsLine::AlreadyLinked { link });
    }
    let newline = if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut addition = String::new();
    if !existing.trim().is_empty() {
        if !existing.ends_with('\n') {
            addition.push_str(newline);
        }
        if !existing.ends_with(&format!("{newline}{newline}")) && !existing.ends_with("\n\n") {
            addition.push_str(newline);
        }
    }
    addition.push_str(line);
    addition.push_str(newline);
    std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(addition.as_bytes()))
        .map_err(|_| Failure::local("cli.project.agents_file_unwritable"))?;
    Ok(AgentsLine::Appended)
}

/// 文件里第一个 `<站点>/r/…` 链接，到空白或 Markdown 的括号、引号为止。
fn existing_link(content: &str, share_url: &str) -> Option<String> {
    let (origin, _) = share_url.rsplit_once("/r/")?;
    let prefix = format!("{origin}/r/");
    let start = content.find(&prefix)?;
    let link: String = content[start..]
        .chars()
        .take_while(|character| {
            !character.is_whitespace()
                && !matches!(character, ')' | ']' | '`' | '"' | '\'' | '<' | '>')
        })
        .collect();
    Some(link.trim_end_matches(['.', ',', ';']).to_owned())
}

fn failure(error: agent_room_agent_client::BridgeToolFailure) -> Failure {
    if bridge_outdated(&error) {
        return Failure::validation("cli.project.bridge_outdated");
    }
    error.into()
}

/// 认领的错误给 Agent 的说法。
pub(crate) fn hint(code: &str) -> Option<&'static str> {
    Some(match code {
        "cli.project.repository_unknown" => {
            "Could not tell which github.com repository this is: run project inside a clone whose origin remote is a github.com URL, or pass --repo owner/repo. Only public github.com repositories can be claimed."
        }
        "cli.project.repository_invalid"
        | "bridge.ipc.repository_invalid"
        | "bridge.project_room.invalid_repository" => {
            "--repo takes owner/repo as on github.com, like octo-org/hello-world."
        }
        "cli.project.agents_file_unreadable" | "cli.project.agents_file_unwritable" => {
            "AGENTS.md at the repository root could not be read or written. The claim is made: add agentsLine to it yourself, or fix the file and run project again (it picks up the same claim)."
        }
        "cli.project.bridge_outdated" => {
            "The Agent Room desktop app on this computer is too old to claim repositories. Ask the person to update it, then run project again."
        }
        "bridge.project_room.too_many_claims" => {
            "This account already has 3 claims waiting. Finish one (push its line) or let the person cancel one in Agent Room, then try again."
        }
        "bridge.project_room.repository_taken" => {
            "This repository is bound to a room that Agent Room is holding, so it can't be claimed now."
        }
        "bridge.project_room.room_not_eligible" | "bridge.project_room.forbidden" => {
            "--room can only bind a public room owned by the account on this computer. Pick another room, or omit --room to have a new room created once the line is recognised."
        }
        "bridge.project_room.room_already_bound" => {
            "That room is already bound to another repository. The person can unbind it in the room's settings, or omit --room."
        }
        "bridge.project_room.check_too_soon" => {
            "A claim is checked at most once a minute. Wait a minute, then run project --wait again."
        }
        "bridge.project_room.not_found" => {
            "That claim is gone: it was recognised, cancelled, or lapsed after 7 days. Run project again to see where it stands."
        }
        "bridge.project_room.unsupported" => {
            "This Agent Room server does not support claiming repositories yet."
        }
        _ => return None,
    })
}
