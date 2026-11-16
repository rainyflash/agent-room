//! 认领 GitHub 仓库（`specs/redesign/project-rooms.md` 的 3P-4b、3P-4c）：命令行 `agent-room project` 和
//! 本机 MCP 的 `agent_room_claim_repository` 共用的几样：从 `git remote` 认出 `owner/repo`，认领，等到
//! 认上。都经本机 Bridge、用这台电脑的授权。
//!
//! 等的时候每分钟核对一次，最多 15 分钟。同一个人同一个仓库再认领，控制面回同一个认领，所以再跑一次
//! 就接着等原来那个，不另起一个。

use std::time::Duration;

use agent_room_bridge_ipc::{
    IpcCheckRepositoryClaimRequest, IpcClaimRepositoryRequest, IpcErrorCategory, IpcMethod,
    IpcRepositoryClaim, IpcRepositoryClaimStatus, IpcResponse,
};
use tokio::time::Instant;

use crate::{BridgeToolClient, BridgeToolFailure};

/// 等认上时多久核对一次：控制面每个认领每分钟只核对一次。
pub const CHECK_INTERVAL: Duration = Duration::from_mins(1);
/// 最多等多久。
pub const WAIT_LIMIT: Duration = Duration::from_mins(15);
/// 核对太快：等下一分钟再来。
pub const CHECK_TOO_SOON: &str = "bridge.project_room.check_too_soon";
/// 没有这个认领了：多半是后台核对已经认上（认领随即删掉），或者 7 天没认上作废了。
pub const CLAIM_NOT_FOUND: &str = "bridge.project_room.not_found";

/// 从 `git remote get-url origin` 认出 `owner/repo`：只认 github.com 的 https 和 ssh 两种写法
/// （`https://github.com/o/r(.git)`、`git@github.com:o/r(.git)`、`ssh://git@github.com/o/r(.git)`）。
/// 认不出来回 `None`，由调用方要 `--repo`。网址里带的账号、令牌不留下。
#[must_use]
pub fn repository_from_remote(remote: &str) -> Option<String> {
    let remote = remote.trim();
    let (authority, path) = if let Some(rest) = strip_scheme(remote, "https://") {
        let (authority, path) = rest.split_once('/')?;
        (strip_port(authority, ":443"), path)
    } else if let Some(rest) = strip_scheme(remote, "ssh://") {
        let (authority, path) = rest.split_once('/')?;
        (strip_port(authority, ":22"), path)
    } else if remote.contains("://") {
        return None;
    } else {
        // scp 那样的写法：`git@github.com:owner/repo.git`。
        remote.split_once(':')?
    };
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if !host.eq_ignore_ascii_case("github.com") {
        return None;
    }
    let path = path.strip_suffix('/').unwrap_or(path);
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.split_once('/')?;
    let repository = format!("{owner}/{name}");
    valid_repository(&repository).then_some(repository)
}

fn strip_scheme<'a>(value: &'a str, scheme: &str) -> Option<&'a str> {
    value
        .get(..scheme.len())
        .filter(|prefix| prefix.eq_ignore_ascii_case(scheme))
        .map(|_| &value[scheme.len()..])
}

fn strip_port<'a>(authority: &'a str, port: &str) -> &'a str {
    authority.strip_suffix(port).unwrap_or(authority)
}

/// `owner/repo` 照 GitHub 的规矩（和 Bridge、控制面同一套校验）。
#[must_use]
pub fn valid_repository(repository: &str) -> bool {
    IpcMethod::ClaimRepository(IpcClaimRepositoryRequest {
        repository: repository.to_owned(),
        catalog_id: None,
    })
    .validate()
    .is_ok()
}

/// 替这台电脑的账户认领；带上 `catalog_id` 是把名下那间公开房间绑上去。
///
/// # Errors
///
/// Bridge 或控制面拒绝时原样返回它的错误。
pub async fn claim(
    backend: &dyn BridgeToolClient,
    repository: &str,
    catalog_id: Option<String>,
) -> Result<IpcRepositoryClaim, BridgeToolFailure> {
    answer(
        backend
            .invoke(IpcMethod::ClaimRepository(IpcClaimRepositoryRequest {
                repository: repository.to_owned(),
                catalog_id,
            }))
            .await?,
    )
}

/// 现在核对一次。
///
/// # Errors
///
/// Bridge 或控制面拒绝时原样返回它的错误（太快是 [`CHECK_TOO_SOON`]）。
pub async fn check(
    backend: &dyn BridgeToolClient,
    claim_id: &str,
) -> Result<IpcRepositoryClaim, BridgeToolFailure> {
    answer(
        backend
            .invoke(IpcMethod::CheckRepositoryClaim(
                IpcCheckRepositoryClaimRequest {
                    claim_id: claim_id.to_owned(),
                },
            ))
            .await?,
    )
}

fn answer(response: IpcResponse) -> Result<IpcRepositoryClaim, BridgeToolFailure> {
    match response {
        IpcResponse::RepositoryClaim { claim } => Ok(claim),
        _ => Err(BridgeToolFailure::new(
            "agent.project_room.response_invalid",
            IpcErrorCategory::Internal,
            false,
            std::collections::BTreeMap::new(),
        )),
    }
}

/// 等的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimWait {
    /// 认上了，绑在 `catalogId` 那间房上。
    Bound(IpcRepositoryClaim),
    /// 到时间还没认上；再跑一次接着等这个认领。
    Pending(IpcRepositoryClaim),
    /// 原来的认领没了（作废了），再认领拿到的是一个新码的认领：仓库里那一行要换成新的。
    Replaced(IpcRepositoryClaim),
}

/// 等到认上：先核对一次，之后每分钟一次，最多 `limit`。核对太快就等下一分钟。核对说没有这个认领时
/// （后台核对认上以后认领就删了），用同样的仓库和房间再认领一次看看：认上了的回绑上的那间房。
///
/// # Errors
///
/// 别的拒绝（Bridge 连不上、没有授权）原样返回。
pub async fn wait_until_bound(
    backend: &dyn BridgeToolClient,
    claim: IpcRepositoryClaim,
    target: Option<String>,
    limit: Duration,
) -> Result<ClaimWait, BridgeToolFailure> {
    let deadline = Instant::now() + limit;
    let mut current = claim;
    loop {
        let Some(claim_id) = current.claim_id.clone() else {
            // 认上了的没有认领号。
            return Ok(ClaimWait::Bound(current));
        };
        if current.status == IpcRepositoryClaimStatus::Bound {
            return Ok(ClaimWait::Bound(current));
        }
        match check(backend, &claim_id).await {
            Ok(next) => current = next,
            Err(failure) if failure.code() == CHECK_TOO_SOON => {}
            Err(failure) if failure.code() == CLAIM_NOT_FOUND => {
                let again = self::claim(backend, &current.repository, target.clone()).await?;
                if again.status == IpcRepositoryClaimStatus::Pending
                    && again.share_code != current.share_code
                {
                    return Ok(ClaimWait::Replaced(again));
                }
                current = again;
            }
            Err(failure) => return Err(failure),
        }
        if current.status == IpcRepositoryClaimStatus::Bound {
            return Ok(ClaimWait::Bound(current));
        }
        if Instant::now() + CHECK_INTERVAL >= deadline {
            return Ok(ClaimWait::Pending(current));
        }
        tokio::time::sleep(CHECK_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests;
