//! 项目房间的认领（`specs/redesign/project-rooms.md` 的 3P-4a，IPC 4.9 起）：命令行和本机 MCP 经 Bridge
//! 替这台电脑的账户认领 GitHub 仓库（`ClaimRepository`）、现在核对（`CheckRepositoryClaim`）。都不要会话，
//! 和建房一样用这台电脑的授权发给控制面。
//!
//! 只有这两个方法：同一个人同一个仓库再认领，控制面回同一个认领，所以“看一眼”就是再认领一次；
//! 命令行、MCP 都不撤认领（没认上的 7 天后自己作废，网页上能撤）。

use agent_room_domain::project_rooms::GithubRepository;
use serde::{Deserialize, Serialize};

use crate::tools::{IpcMethodValidationFailure, failure, validate_uuid_v7};

/// 认领一个仓库（`ClaimRepository`）。带上 `catalogId` 是把名下那间已有的公开房间绑上去。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcClaimRepositoryRequest {
    /// `owner/repo`，照 GitHub 的规矩。
    pub repository: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_id: Option<String>,
}

impl IpcClaimRepositoryRequest {
    pub(crate) fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        GithubRepository::parse(&self.repository)
            .map_err(|_| failure("bridge.ipc.repository_invalid"))?;
        match &self.catalog_id {
            Some(catalog_id) => validate_uuid_v7(catalog_id, "bridge.ipc.room_invalid"),
            None => Ok(()),
        }
    }
}

/// 现在核对一个认领（`CheckRepositoryClaim`）。控制面每个认领每分钟只核对一次。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcCheckRepositoryClaimRequest {
    pub claim_id: String,
}

impl IpcCheckRepositoryClaimRequest {
    pub(crate) fn validate(&self) -> Result<(), IpcMethodValidationFailure> {
        validate_uuid_v7(&self.claim_id, "bridge.ipc.claim_id_invalid")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpcRepositoryClaimStatus {
    /// 还在等仓库里那一行。
    Pending,
    /// 认上了，仓库绑在 `catalogId` 那间房上。
    Bound,
}

/// 上一次核对：什么时候、结果（`not_in_repository`、`repository_taken` 之类，原样转交控制面的）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcRepositoryCheck {
    pub at_unix_ms: i64,
    pub result: String,
}

/// 认领、核对的回答。认上以后认领就没了：`claimId` 不写，`catalogId` 是绑上的那间房。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IpcRepositoryClaim {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_id: Option<String>,
    pub repository: String,
    pub share_code: String,
    pub share_url: String,
    /// 写进仓库根目录 `AGENTS.md` 的那一行。
    pub agents_line: String,
    /// README 徽章那一行（Markdown）。
    pub badge_markdown: String,
    pub status: IpcRepositoryClaimStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at_unix_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_check: Option<IpcRepositoryCheck>,
}

#[cfg(test)]
mod tests {
    use super::{
        IpcCheckRepositoryClaimRequest, IpcClaimRepositoryRequest, IpcRepositoryCheck,
        IpcRepositoryClaim, IpcRepositoryClaimStatus,
    };
    use crate::{IpcMethod, IpcResponse, tools::IpcMethodValidationFailure};

    const CATALOG_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e46";
    const CLAIM_ID: &str = "0198b601-77a1-7bb8-83eb-a8fe68c97e49";

    fn code(method: &IpcMethod) -> Result<(), &'static str> {
        method.validate().map_err(IpcMethodValidationFailure::code)
    }

    #[test]
    fn 认领和核对的请求来回编码_不设的字段不写出去() {
        let claim = IpcMethod::ClaimRepository(IpcClaimRepositoryRequest {
            repository: "octo-org/hello.world".to_owned(),
            catalog_id: None,
        });
        let encoded = serde_json::to_value(&claim).unwrap();
        assert_eq!(
            encoded,
            serde_json::json!({"claim_repository": {"repository": "octo-org/hello.world"}})
        );
        assert_eq!(serde_json::from_value::<IpcMethod>(encoded).unwrap(), claim);
        assert_eq!(claim.name(), "claim_repository");

        let check = IpcMethod::CheckRepositoryClaim(IpcCheckRepositoryClaimRequest {
            claim_id: CLAIM_ID.to_owned(),
        });
        let encoded = serde_json::to_value(&check).unwrap();
        assert_eq!(
            encoded,
            serde_json::json!({"check_repository_claim": {"claimId": CLAIM_ID}})
        );
        assert_eq!(serde_json::from_value::<IpcMethod>(encoded).unwrap(), check);
        assert_eq!(check.name(), "check_repository_claim");
        assert!(
            serde_json::from_value::<IpcMethod>(serde_json::json!({
                "claim_repository": {"repository": "octo/hello", "extra": true}
            }))
            .is_err(),
            "不认识的字段不收"
        );
    }

    #[test]
    fn 仓库照_github_的规矩_房间和认领要是_uuid_v7() {
        let claim = |repository: &str, catalog_id: Option<&str>| {
            IpcMethod::ClaimRepository(IpcClaimRepositoryRequest {
                repository: repository.to_owned(),
                catalog_id: catalog_id.map(str::to_owned),
            })
        };
        assert_eq!(code(&claim("octo/hello", Some(CATALOG_ID))), Ok(()));
        for bad in [
            "",
            "octo",
            "octo/",
            "/hello",
            "octo/hello/extra",
            "octo hello/x",
        ] {
            assert_eq!(
                code(&claim(bad, None)),
                Err("bridge.ipc.repository_invalid"),
                "{bad}"
            );
        }
        assert_eq!(
            code(&claim("octo/hello", Some("not-a-uuid"))),
            Err("bridge.ipc.room_invalid")
        );
        assert_eq!(
            code(&IpcMethod::CheckRepositoryClaim(
                IpcCheckRepositoryClaimRequest {
                    claim_id: "0198b601-77a1-4bb8-83eb-a8fe68c97e49".to_owned(),
                }
            )),
            Err("bridge.ipc.claim_id_invalid")
        );
    }

    #[test]
    fn 认领和核对不包在会话里_用管房间的权限() {
        let claim = IpcMethod::ClaimRepository(IpcClaimRepositoryRequest {
            repository: "octo/hello".to_owned(),
            catalog_id: None,
        });
        assert_eq!(
            claim.required_scope(),
            agent_room_bridge_core::ipc::IpcScope::RoomsManage
        );
        let wrapped = IpcMethod::WithSession {
            session_id: "0198b601-77a1-7bb8-83eb-a8fe68c97e40".to_owned(),
            method: Box::new(claim),
        };
        assert_eq!(code(&wrapped), Err("bridge.ipc.session_method_invalid"));
    }

    #[test]
    fn 回答来回编码_认上了不带认领号() {
        let pending = IpcResponse::RepositoryClaim {
            claim: IpcRepositoryClaim {
                claim_id: Some(CLAIM_ID.to_owned()),
                repository: "octo/hello".to_owned(),
                share_code: "aB3xYz".to_owned(),
                share_url: "https://agent-room.test/r/aB3xYz".to_owned(),
                agents_line: "Agent Room: this project's room is https://agent-room.test/r/aB3xYz."
                    .to_owned(),
                badge_markdown: "[![Agent Room](x)](y)".to_owned(),
                status: IpcRepositoryClaimStatus::Pending,
                catalog_id: None,
                claimed_at_unix_ms: Some(1_000),
                last_check: Some(IpcRepositoryCheck {
                    at_unix_ms: 2_000,
                    result: "not_in_repository".to_owned(),
                }),
            },
        };
        let encoded = serde_json::to_value(&pending).unwrap();
        assert_eq!(encoded["type"], "repository_claim");
        assert_eq!(encoded["claim"]["status"], "pending");
        assert_eq!(encoded["claim"]["lastCheck"]["result"], "not_in_repository");
        assert_eq!(
            serde_json::from_value::<IpcResponse>(encoded).unwrap(),
            pending
        );

        let IpcResponse::RepositoryClaim { claim } = pending else {
            unreachable!()
        };
        let bound = IpcResponse::RepositoryClaim {
            claim: IpcRepositoryClaim {
                claim_id: None,
                status: IpcRepositoryClaimStatus::Bound,
                catalog_id: Some(CATALOG_ID.to_owned()),
                claimed_at_unix_ms: None,
                last_check: None,
                ..claim
            },
        };
        let encoded = serde_json::to_value(&bound).unwrap();
        for absent in ["claimId", "claimedAtUnixMs", "lastCheck"] {
            assert!(encoded["claim"].get(absent).is_none(), "{absent}");
        }
        assert_eq!(encoded["claim"]["status"], "bound");
        assert_eq!(
            serde_json::from_value::<IpcResponse>(encoded).unwrap(),
            bound
        );
    }
}
