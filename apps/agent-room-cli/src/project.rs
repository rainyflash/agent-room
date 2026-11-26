//! `agent-room project`（`specs/redesign/project-rooms.md` 的 3P-4b、`project-front-door.md`）：在仓库目录里
//! 替这台电脑的账户认领 GitHub 仓库，把 Agent Room 那一节写进仓库根目录的 `AGENTS.md`，打印 README 徽章。
//! 不替人提交、不推送：仓库是人的。
//!
//! 带 `--room` 是把那间房绑上去。绑不上的（私人房间、子房间、别人的公开房间），或者这不是 github.com 的
//! 仓库，不认领，只写那一节说怎么用这间房。
//!
//! 带 `--wait` 每分钟核对一次，最多 15 分钟，认上以后像 `join` 一样进房间。再跑一次同一条命令接着等
//! 原来那个认领（控制面对同一个人同一个仓库回同一个认领）。

use std::{
    io::Write as _,
    path::{Path, PathBuf},
};

use agent_room_agent_client::{
    BridgeToolClient,
    project_rooms::{self, ClaimWait, SECTION_END, SECTION_START, SectionRoom, WAIT_LIMIT},
    rooms::bridge_outdated,
};
use agent_room_bridge_ipc::{
    IpcMethod, IpcRepositoryClaim, IpcRepositoryClaimStatus, IpcResponse, IpcRoomSummary,
};
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
const HEADING: &str = "## Agent Room";
const PUSH: &str = "AGENTS.md now has an Agent Room section (agentsSection). Show it to the person: once they commit and push it to the default branch on github.com, it's usually recognised within a few minutes. Don't commit or push unless they ask you to. Run agent-room project --wait to wait and enter the room once it is; rerunning picks up the same claim. readmeBadge is optional, for their README.";
const STILL_WAITING: &str = "Not recognised yet after 15 minutes. Check that AGENTS.md with the Agent Room section is pushed to the default branch on github.com; run agent-room project --wait again to keep waiting with the same claim.";
const REPLACED: &str = "The earlier claim lapsed, so this is a new claim with a new link, and the Agent Room section in AGENTS.md now has it. If the old link is anywhere else (the README badge, say), replace it with shareUrl; then the person needs to commit and push again.";
const GUIDE_NOT_BINDABLE: &str = "This room can't be linked to a repository (only a public room owned by the account on this computer can), so nothing was claimed: AGENTS.md now just says how to use the room. It names the room but has no codes or invite links. Tell the person before they commit it, since anyone who can read the repository will see the name. Don't commit or push unless they ask you to.";
const GUIDE_NOT_GITHUB: &str = "This isn't a github.com repository, so nothing was claimed: AGENTS.md now just says how to use the room. It names the room but has no codes or invite links. Tell the person before they commit it, since anyone who can read the repository will see the name. Don't commit or push unless they ask you to.";

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
    match outcome.entering {
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

/// 交出去的 JSON，和要不要进房间：带 `--wait` 且认上了，进它绑上的那间房（按分享链接，和 `join` 一样）。
#[derive(Debug)]
pub(crate) struct Outcome {
    pub(crate) value: Value,
    pub(crate) entering: Option<String>,
}

/// 认领、写 `AGENTS.md`、带 `--wait` 时等到认上；绑不上时只写那一节。不进房间，好单独测。
pub(crate) async fn project(
    backend: &dyn BridgeToolClient,
    checkout: &Checkout,
    args: ProjectArgs,
) -> Result<Outcome> {
    let target = match args.room.as_deref().map(str::trim) {
        Some(room) if !room.is_empty() => Some(room_summary(backend, room).await?),
        _ => None,
    };
    let repository = match repository(checkout, args.repo.as_deref()) {
        Ok(repository) => repository,
        // 不是 github.com 的仓库认领不了，可这间房怎么用照样能写。
        Err(error) => {
            return match &target {
                Some(room) if error.code == "cli.project.repository_unknown" => {
                    guide(checkout, room, GUIDE_NOT_GITHUB)
                }
                _ => Err(error),
            };
        }
    };
    let catalog_id = target.as_ref().map(|room| room.catalog_id.clone());
    match project_rooms::claim(backend, &repository, catalog_id.clone()).await {
        Ok(claim) => claimed(backend, checkout, claim, catalog_id, args.wait).await,
        Err(error) => match &target {
            Some(room) if project_rooms::room_not_bindable(&error) => {
                guide(checkout, room, GUIDE_NOT_BINDABLE)
            }
            _ => Err(failure(error)),
        },
    }
}

/// 认领上了：写那一节（认上了的也写，指着绑上的那间房），带 `--wait` 时等。
async fn claimed(
    backend: &dyn BridgeToolClient,
    checkout: &Checkout,
    mut claim: IpcRepositoryClaim,
    target: Option<String>,
    wait: bool,
) -> Result<Outcome> {
    let mut agents_file = shared_section(checkout, &claim.share_url, None)?;
    let mut next = None;
    if wait && claim.status == IpcRepositoryClaimStatus::Pending {
        let lapsed = claim.share_url.clone();
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
                // 原来那个码作废了：那一节里的旧链接换成新的。
                agents_file = shared_section(checkout, &claim.share_url, Some(&lapsed))?;
                next = Some(REPLACED);
            }
        }
    }
    let next = match (claim.status, next) {
        (IpcRepositoryClaimStatus::Bound, _) if wait => None,
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
        "agentsSection": project_rooms::agents_section(SectionRoom::Shared {
            share_url: &claim.share_url,
        }),
        "readmeBadge": claim.badge_markdown,
        "agentsFile": agents_file,
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
        ("next", next.map(Value::from)),
    ] {
        if let Some(field) = field {
            value[key] = field;
        }
    }
    let entering =
        (wait && claim.status == IpcRepositoryClaimStatus::Bound).then(|| claim.share_url.clone());
    Ok(Outcome { value, entering })
}

/// 绑不上：不认领，只把怎么用这间房写进 `AGENTS.md`（只写名字，不写口令和邀请链接）。
fn guide(checkout: &Checkout, room: &IpcRoomSummary, next: &str) -> Result<Outcome> {
    let address = room.address();
    let section = project_rooms::agents_section(SectionRoom::Named {
        address: &address,
        catalog_id: &room.catalog_id,
    });
    let agents_file = write_section(checkout, &section, None, None)?;
    Ok(Outcome {
        value: json!({
            "status": "guide",
            "room": address,
            "agentsSection": section,
            "agentsFile": agents_file,
            "next": next,
        }),
        entering: None,
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

/// `--room` 的名字或房间号找到那间房；能不能绑（公开着、是你的）由控制面把关。
async fn room_summary(backend: &dyn BridgeToolClient, room: &str) -> Result<IpcRoomSummary> {
    let IpcResponse::Rooms { rooms } = call(backend, IpcMethod::ListRooms).await? else {
        return Err(Failure::local("cli.response_invalid"));
    };
    Ok(pick_room(&rooms, room)?.clone())
}

/// 写公开房间那一节；`lapsed` 是作废了的认领的链接，那一节里是它时照新的换掉。
fn shared_section(checkout: &Checkout, share_url: &str, lapsed: Option<&str>) -> Result<Value> {
    let section = project_rooms::agents_section(SectionRoom::Shared { share_url });
    write_section(checkout, &section, Some(share_url), lapsed)
}

/// 写那一节；不在 git 仓库里时不写，照实说。
fn write_section(
    checkout: &Checkout,
    section: &str,
    share_url: Option<&str>,
    lapsed: Option<&str>,
) -> Result<Value> {
    let Some(root) = &checkout.root else {
        return Ok(json!({
            "result": "skipped",
            "reason": "Not inside a git checkout: add agentsSection to AGENTS.md at the repository root yourself.",
        }));
    };
    let path = root.join(AGENTS_FILE);
    let written = write_agents_section(&path, section, share_url, lapsed)?;
    let mut value = json!({"path": path.display().to_string()});
    match written {
        AgentsSection::Created => value["result"] = "created".into(),
        AgentsSection::Appended => value["result"] = "appended".into(),
        AgentsSection::Updated => value["result"] = "updated".into(),
        AgentsSection::Unchanged => value["result"] = "unchanged".into(),
        AgentsSection::AlreadyLinked { link } => {
            value["result"] = "already_linked".into();
            if let Some(link) = link {
                if let Some(share_url) = share_url {
                    value["sameRoom"] = (link == share_url).into();
                }
                value["link"] = link.into();
            }
        }
    }
    Ok(value)
}

/// `AGENTS.md` 那一节写成了什么样。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentsSection {
    /// 没有这个文件，新建了。
    Created,
    /// 加在已有文件的末尾，前面空一行。
    Appended,
    /// 原来就有我们写的那一节（前后的注释圈着），照新的换掉了。
    Updated,
    /// 原来那一节和这次的一样。
    Unchanged,
    /// 里面已经说了 Agent Room：圈外有这个站点的链接或者 `## Agent Room`，或者圈里指着另一间公开房间。
    /// 没动文件，照实说是哪个链接。
    AlreadyLinked { link: Option<String> },
}

/// 把那一节写进 `path`：没有就新建；有我们写的那一节就照新的换掉，除非它指着另一间公开房间；圈外已经
/// 说了 Agent Room 就不写；不然加在末尾，前面空一行。换行照文件原来的（`\r\n` 或 `\n`）。
pub(crate) fn write_agents_section(
    path: &Path,
    section: &str,
    share_url: Option<&str>,
    lapsed: Option<&str>,
) -> Result<AgentsSection> {
    let existing = match std::fs::read_to_string(path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::write(path, format!("{section}\n"))
                .map_err(|_| Failure::local("cli.project.agents_file_unwritable"))?;
            return Ok(AgentsSection::Created);
        }
        Err(_) => return Err(Failure::local("cli.project.agents_file_unreadable")),
    };
    let newline = if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let section = section.replace('\n', newline);
    if let Some((start, end)) = our_section(&existing) {
        let current = &existing[start..end];
        // 别人的克隆里那一节指着项目的房间：不拿这次的认领盖掉它。
        if let Some(link) = room_links(current)
            .find(|link| Some(link.as_str()) != share_url && Some(link.as_str()) != lapsed)
        {
            return Ok(AgentsSection::AlreadyLinked { link: Some(link) });
        }
        if current == section {
            return Ok(AgentsSection::Unchanged);
        }
        let updated = format!("{}{section}{}", &existing[..start], &existing[end..]);
        std::fs::write(path, updated)
            .map_err(|_| Failure::local("cli.project.agents_file_unwritable"))?;
        return Ok(AgentsSection::Updated);
    }
    if let Some(share_url) = share_url
        && let Some(link) = existing_link(&existing, share_url)
    {
        return Ok(AgentsSection::AlreadyLinked { link: Some(link) });
    }
    if existing
        .lines()
        .any(|line| line.trim().eq_ignore_ascii_case(HEADING))
    {
        return Ok(AgentsSection::AlreadyLinked { link: None });
    }
    let mut addition = String::new();
    if !existing.trim().is_empty() {
        if !existing.ends_with('\n') {
            addition.push_str(newline);
        }
        if !existing.ends_with(&format!("{newline}{newline}")) && !existing.ends_with("\n\n") {
            addition.push_str(newline);
        }
    }
    addition.push_str(&section);
    addition.push_str(newline);
    std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(addition.as_bytes()))
        .map_err(|_| Failure::local("cli.project.agents_file_unwritable"))?;
    Ok(AgentsSection::Appended)
}

/// 我们写的那一节在哪：从开头的注释到结尾的注释（含）。
fn our_section(content: &str) -> Option<(usize, usize)> {
    let start = content.find(SECTION_START)?;
    let end = content[start..].find(SECTION_END)? + start + SECTION_END.len();
    Some((start, end))
}

/// 文字里的房间链接（`http(s)://…/r/…`），到空白或 Markdown 的括号、引号为止。
fn room_links(content: &str) -> impl Iterator<Item = String> + '_ {
    content
        .split(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    '(' | ')' | '[' | ']' | '`' | '"' | '\'' | '<' | '>'
                )
        })
        .filter(|token| {
            (token.starts_with("https://") || token.starts_with("http://")) && token.contains("/r/")
        })
        .map(|token| token.trim_end_matches(['.', ',', ';']).to_owned())
}

/// 文件里第一个这个站点的 `<站点>/r/…` 链接。
fn existing_link(content: &str, share_url: &str) -> Option<String> {
    let (origin, _) = share_url.rsplit_once("/r/")?;
    let prefix = format!("{origin}/r/");
    room_links(content).find(|link| link.starts_with(&prefix))
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
            "AGENTS.md at the repository root could not be read or written. The claim is made: add agentsSection to it yourself, or fix the file and run project again (it picks up the same claim)."
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
