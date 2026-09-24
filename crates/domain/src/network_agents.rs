//! 只凭网络接入的 Agent（见 `specs/network-agents/design.md`）：服务器替它保管身份，
//! Agent 只拿一个访问令牌。这里是与存储、传输无关的规则：名字、状态和主体标识。

use crate::{DomainError, DomainResult};

/// 网络 Agent 的主人是一个合成主体，用这个颁发者标记；它不能登录。
pub const NETWORK_AGENT_ISSUER: &str = "urn:agent-room:network-agent";

const MAX_NAME_CHARACTERS: usize = 64;

/// 不能冒充平台或管理者：规范化后等于这些，或以 `agentroom` 开头的名字都不给用。
const RESERVED_NAMES: &[&str] = &[
    "system",
    "admin",
    "administrator",
    "moderator",
    "mod",
    "root",
    "official",
    "系统",
    "管理员",
    "管理者",
    "版主",
    "官方",
    "客服",
];
const RESERVED_PREFIX: &str = "agentroom";

/// Agent 给自己起的名字：1 到 64 个字符（按字符算），去掉首尾空白，不含控制字符，
/// 不冒充平台或管理者。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkAgentName(String);

impl NetworkAgentName {
    /// # Errors
    ///
    /// 名字为空、过长、含控制字符或是保留名时返回错误。
    pub fn parse(input: &str) -> DomainResult<Self> {
        let name = input.trim();
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARACTERS {
            return Err(invalid("名字须为 1 到 64 个字符"));
        }
        if name.chars().any(char::is_control) {
            return Err(invalid("名字不能含控制字符"));
        }
        let normalized = normalized(name);
        if normalized.starts_with(RESERVED_PREFIX)
            || RESERVED_NAMES
                .iter()
                .any(|reserved| normalized == *reserved)
        {
            return Err(DomainError::Validation {
                field: "network_agent_name",
                reason: "这个名字留给平台与管理者",
            });
        }
        Ok(Self(name.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 已有同名的网络 Agent 时，加上 ` 2`、` 3` 这样的序号；过长时截短原名，
    /// 保证带序号的名字仍在 64 个字符以内。
    #[must_use]
    pub fn numbered(&self, number: u32) -> Self {
        if number <= 1 {
            return self.clone();
        }
        let suffix = format!(" {number}");
        let room = MAX_NAME_CHARACTERS.saturating_sub(suffix.chars().count());
        let base: String = self.0.chars().take(room).collect();
        Self(format!("{}{suffix}", base.trim_end()))
    }
}

/// 比较保留名时忽略大小写、空白和常见分隔符，挡住 `Agent-Room`、`A D M I N` 这类写法。
fn normalized(name: &str) -> String {
    name.chars()
        .filter(|character| {
            !character.is_whitespace() && !matches!(character, '-' | '_' | '.' | '·' | '・')
        })
        .flat_map(char::to_lowercase)
        .collect()
}

const fn invalid(reason: &'static str) -> DomainError {
    DomainError::Validation {
        field: "network_agent_name",
        reason,
    }
}

/// 网络 Agent 的状态。创建分几步：先记下身份（provisioning），建好 Agent 与实例后才生效；
/// 停用后令牌作废、离开所有房间，不能再恢复。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkAgentStatus {
    Provisioning,
    Active,
    Disabled,
}

impl NetworkAgentStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Provisioning => "provisioning",
            Self::Active => "active",
            Self::Disabled => "disabled",
        }
    }

    /// # Errors
    ///
    /// 不是已知状态时返回错误。
    pub fn parse(value: &str) -> DomainResult<Self> {
        match value {
            "provisioning" => Ok(Self::Provisioning),
            "active" => Ok(Self::Active),
            "disabled" => Ok(Self::Disabled),
            _ => Err(DomainError::Validation {
                field: "network_agent_status",
                reason: "未知的网络 Agent 状态",
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 名字去掉首尾空白_按字符数限长_不含控制字符() {
        assert_eq!(
            NetworkAgentName::parse("  Scout ").unwrap().as_str(),
            "Scout"
        );
        assert_eq!(
            NetworkAgentName::parse(&"名".repeat(64)).unwrap().as_str(),
            "名".repeat(64)
        );
        for invalid in ["", "   ", &"名".repeat(65), "换\n行", "tab\there"] {
            assert!(NetworkAgentName::parse(invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn 不能冒充平台或管理者() {
        for reserved in [
            "System",
            "ADMIN",
            "A d m i n",
            "Moderator",
            "Agent Room",
            "agent-room",
            "Agent Room 助手",
            "AgentRoom Official",
            "系统",
            "管理员",
            "官方",
        ] {
            assert!(NetworkAgentName::parse(reserved).is_err(), "{reserved}");
        }
        for allowed in ["Admiral", "Systematic reviewer", "我的 Agent", "Room Agent"] {
            assert!(NetworkAgentName::parse(allowed).is_ok(), "{allowed}");
        }
    }

    #[test]
    fn 重名加序号且仍在长度以内() {
        let name = NetworkAgentName::parse("Scout").unwrap();
        assert_eq!(name.numbered(1), name);
        assert_eq!(name.numbered(2).as_str(), "Scout 2");
        let long = NetworkAgentName::parse(&"a".repeat(64)).unwrap();
        let numbered = long.numbered(12);
        assert_eq!(numbered.as_str().chars().count(), 64);
        assert!(numbered.as_str().ends_with(" 12"));
        // 截短后末尾的空白不留在序号前面。
        let spaced = NetworkAgentName::parse(&format!("{} b", "a".repeat(61))).unwrap();
        assert_eq!(spaced.numbered(3).as_str(), format!("{} 3", "a".repeat(61)));
    }

    #[test]
    fn 状态可往返() {
        for status in [
            NetworkAgentStatus::Provisioning,
            NetworkAgentStatus::Active,
            NetworkAgentStatus::Disabled,
        ] {
            assert_eq!(NetworkAgentStatus::parse(status.as_str()).unwrap(), status);
        }
        assert!(NetworkAgentStatus::parse("banned").is_err());
    }
}
