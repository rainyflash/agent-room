//! 项目房间（`specs/redesign/project-rooms.md`）：一个 GitHub 仓库一间公开房间。认领的人在仓库默认分支的
//! `AGENTS.md` 或 `README.md` 里放一行带这间房分享码的链接，服务器读到了就算认上。
//!
//! 这里只放规则：仓库的名字怎么验、那一行在哪个文件里、每次核对的结果怎么记。

use crate::{DomainError, DomainResult};

/// owner 最多几个字符（GitHub 的规矩）。
const MAX_OWNER_LENGTH: usize = 39;
/// repo 最多几个字符（GitHub 的规矩）。
const MAX_NAME_LENGTH: usize = 100;

/// github.com 上的一个仓库：`owner/repo`。名字照原样存一份给人看；GitHub 的名字不分大小写，比的时候
/// 不论大小写（[`Self::same_as`]）。
///
/// 规矩和 `/gh/` 地址、库里的约束一样：
/// - owner：字母、数字、连字符，1 到 39 个，不以连字符开头；
/// - repo：字母、数字、`.`、`_`、`-`，1 到 100 个，不能是 `.` 或 `..`。
///
/// 验过的名字里只有这几种字符，拼进网址不用再编码；读 GitHub 的那一边照样百分号编码一遍。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GithubRepository {
    owner: String,
    name: String,
}

impl GithubRepository {
    /// # Errors
    ///
    /// owner 或 repo 不合 GitHub 的规矩时返回错误。
    pub fn new(owner: &str, name: &str) -> DomainResult<Self> {
        if !valid_owner(owner) {
            return Err(DomainError::Validation {
                field: "repository_owner",
                reason: "owner 只能是字母、数字和连字符，1 到 39 个，不以连字符开头",
            });
        }
        if !valid_name(name) {
            return Err(DomainError::Validation {
                field: "repository_name",
                reason: "repo 只能是字母、数字、点、下划线和连字符，1 到 100 个，不能是 . 或 ..",
            });
        }
        Ok(Self {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }

    /// 解析 `owner/repo`，正好一个斜杠，前后不许有空白。
    ///
    /// # Errors
    ///
    /// 不是 `owner/repo` 的样子，或者名字不合规矩时返回错误。
    pub fn parse(full_name: &str) -> DomainResult<Self> {
        let Some((owner, name)) = full_name.split_once('/') else {
            return Err(DomainError::Validation {
                field: "repository",
                reason: "仓库要写成 owner/repo",
            });
        };
        Self::new(owner, name)
    }

    #[must_use]
    pub fn owner(&self) -> &str {
        &self.owner
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `owner/repo`，照原样的大小写。
    #[must_use]
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    /// 是不是同一个仓库：GitHub 的名字不分大小写。
    #[must_use]
    pub fn same_as(&self, other: &Self) -> bool {
        self.owner.eq_ignore_ascii_case(&other.owner) && self.name.eq_ignore_ascii_case(&other.name)
    }
}

/// owner 合不合 GitHub 的规矩。
#[must_use]
pub fn valid_owner(owner: &str) -> bool {
    !owner.is_empty()
        && owner.len() <= MAX_OWNER_LENGTH
        && !owner.starts_with('-')
        && owner
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

/// repo 合不合 GitHub 的规矩。
#[must_use]
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_LENGTH
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// 那一行在哪个文件里找到的。服务器只读这两个，先读 `AGENTS.md`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RepositoryFile {
    AgentsMd,
    ReadmeMd,
}

impl RepositoryFile {
    /// 读的先后：`AGENTS.md` 里找到了就不再读 README。
    pub const READ_ORDER: [Self; 2] = [Self::AgentsMd, Self::ReadmeMd];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AgentsMd => "AGENTS.md",
            Self::ReadmeMd => "README.md",
        }
    }

    /// # Errors
    ///
    /// 不是这两个文件名时返回错误。
    pub fn parse(value: &str) -> DomainResult<Self> {
        match value {
            "AGENTS.md" => Ok(Self::AgentsMd),
            "README.md" => Ok(Self::ReadmeMd),
            _ => Err(DomainError::Validation {
                field: "repository_file",
                reason: "只认 AGENTS.md 和 README.md",
            }),
        }
    }
}

/// 一个等着的认领上次核对的结果。认上了的认领直接删掉，不记结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClaimCheckResult {
    /// 正在读 GitHub。开始读之前先记下这一刻，别的控制面就不再同时核对它；读完换成真的结果。
    Checking,
    /// 默认分支的 `AGENTS.md`、`README.md` 里都没有这间房的链接（私有仓库、没有这个仓库也是这样）。
    NotInRepository,
    /// raw 回了跳转：仓库改过名或者搬了家，要用现在的名字再认领。
    RepositoryRenamed,
    /// 没找到，而且有文件超过了 512 KB，只读了开头。
    FileTooLarge,
    /// GitHub 暂时读不了：超时、出错、限了次数，或者全站这一分钟的次数用完了。会自己再核对。
    GithubUnavailable,
    /// 仓库绑着别的房间，那间房的链接还在仓库里（或者那间房被平台冻结、从目录里拿掉了）。
    RepositoryTaken,
    /// 那一行找到了，可房间没建成（聊天服务器一时不通之类）。会自己再试。
    CreationFailed,
}

impl ClaimCheckResult {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Checking => "checking",
            Self::NotInRepository => "not_in_repository",
            Self::RepositoryRenamed => "repository_renamed",
            Self::FileTooLarge => "file_too_large",
            Self::GithubUnavailable => "github_unavailable",
            Self::RepositoryTaken => "repository_taken",
            Self::CreationFailed => "creation_failed",
        }
    }

    /// # Errors
    ///
    /// 不认得的结果返回错误。
    pub fn parse(value: &str) -> DomainResult<Self> {
        match value {
            "checking" => Ok(Self::Checking),
            "not_in_repository" => Ok(Self::NotInRepository),
            "repository_renamed" => Ok(Self::RepositoryRenamed),
            "file_too_large" => Ok(Self::FileTooLarge),
            "github_unavailable" => Ok(Self::GithubUnavailable),
            "repository_taken" => Ok(Self::RepositoryTaken),
            "creation_failed" => Ok(Self::CreationFailed),
            _ => Err(DomainError::Validation {
                field: "claim_check_result",
                reason: "不认得的核对结果",
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ClaimCheckResult, GithubRepository, RepositoryFile};

    #[test]
    fn 仓库名照_github_的规矩验() {
        for valid in [
            "rainyflash/agent-room",
            "a/b",
            "A-1/x.y_z-",
            "abcdefghijklmnopqrstuvwxyzabcdefghijklm/r",
            "o/.github",
            "o/...",
        ] {
            assert!(GithubRepository::parse(valid).is_ok(), "{valid} 应该合规矩");
        }
        let long_name = format!("o/{}", "r".repeat(100));
        assert!(GithubRepository::parse(&long_name).is_ok());
        for invalid in [
            "",
            "agent-room",
            "/repo",
            "owner/",
            "-owner/repo",
            "own_er/repo",
            "owner/re po",
            "owner/repo/more",
            "owner/.",
            "owner/..",
            "abcdefghijklmnopqrstuvwxyzabcdefghijklmn/r",
            " owner/repo",
            "owner/repo%2F",
            "owner/rep\u{e9}",
        ] {
            assert!(
                GithubRepository::parse(invalid).is_err(),
                "{invalid} 不合规矩"
            );
        }
        let too_long = format!("o/{}", "r".repeat(101));
        assert!(GithubRepository::parse(&too_long).is_err());
    }

    #[test]
    fn 比仓库时不论大小写_显示时照原样() {
        let first = GithubRepository::parse("RainyFlash/Agent-Room").expect("合规矩");
        let second = GithubRepository::parse("rainyflash/agent-room").expect("合规矩");
        assert!(first.same_as(&second));
        assert_ne!(first, second);
        assert_eq!(first.full_name(), "RainyFlash/Agent-Room");
        let other = GithubRepository::parse("rainyflash/agent-rooms").expect("合规矩");
        assert!(!first.same_as(&other));
    }

    #[test]
    fn 文件名和核对结果来回不走样() {
        for file in RepositoryFile::READ_ORDER {
            assert_eq!(RepositoryFile::parse(file.as_str()), Ok(file));
        }
        assert!(RepositoryFile::parse("CLAUDE.md").is_err());
        for result in [
            ClaimCheckResult::Checking,
            ClaimCheckResult::NotInRepository,
            ClaimCheckResult::RepositoryRenamed,
            ClaimCheckResult::FileTooLarge,
            ClaimCheckResult::GithubUnavailable,
            ClaimCheckResult::RepositoryTaken,
            ClaimCheckResult::CreationFailed,
        ] {
            assert_eq!(ClaimCheckResult::parse(result.as_str()), Ok(result));
        }
        assert!(ClaimCheckResult::parse("verified").is_err());
    }
}
