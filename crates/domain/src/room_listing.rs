//! 公开房间的设置（`specs/public-rooms/design.md`）：进不进目录、别人怎么进、新成员给什么权限、
//! 让不让没有账号的网络 Agent 进，以及每间房一个分享码。
//!
//! “公开”只管这几样，别的照私人房间：同一套权限、同一套治理。设置只有房主能改，见
//! [`crate::private_rooms::PrivateRoom::authorize_listing_change`]。

use crate::{DomainError, DomainResult, private_rooms::PrivateRoomPermissions};

/// 别人怎么进公开房间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoomJoinMode {
    /// 在目录里点“加入”就进来了。
    Open,
    /// 申请以后由房主或管理员批准。
    Request,
    /// 目录里看得到，只能由房主邀请。
    InviteOnly,
}

impl RoomJoinMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Request => "request",
            Self::InviteOnly => "invite_only",
        }
    }
}

impl TryFrom<&str> for RoomJoinMode {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "open" => Ok(Self::Open),
            "request" => Ok(Self::Request),
            "invite_only" => Ok(Self::InviteOnly),
            _ => Err(DomainError::Validation {
                field: "room_join_mode",
                reason: "只能是 open、request 或 invite_only",
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoomListingSettings {
    listed: bool,
    join_mode: RoomJoinMode,
    member_permissions: PrivateRoomPermissions,
    allow_network_agents: bool,
}

impl RoomListingSettings {
    /// 直接进来、或者批准进来的人默认给的：查看、发言、自动发言，和邀请链接一样。
    pub const DEFAULT_MEMBER_PERMISSIONS: PrivateRoomPermissions =
        PrivateRoomPermissions::INVITED_BY_AGENT;

    /// 还没公开过的房间看到的设置：私密，公开以后默认直接进、让没有账号的网络 Agent 进
    /// （维护者 2026-11-01 定，和公共大厅一样）。
    pub const UNLISTED: Self = Self {
        listed: false,
        join_mode: RoomJoinMode::Open,
        member_permissions: Self::DEFAULT_MEMBER_PERMISSIONS,
        allow_network_agents: true,
    };

    /// # Errors
    ///
    /// 新成员的权限没有查看、或者带着管理时返回错误：管理员要房主一个个设，不能谁进来都是。
    pub const fn new(
        listed: bool,
        join_mode: RoomJoinMode,
        member_permissions: PrivateRoomPermissions,
        allow_network_agents: bool,
    ) -> DomainResult<Self> {
        if !member_permissions.view() || member_permissions.manage() {
            return Err(DomainError::Validation {
                field: "room_listing_member_permissions",
                reason: "新成员要能查看，不能带着管理",
            });
        }
        Ok(Self {
            listed,
            join_mode,
            member_permissions,
            allow_network_agents,
        })
    }

    /// 在目录里，别人搜得到、按加入方式进来。
    pub const fn listed(self) -> bool {
        self.listed
    }

    pub const fn join_mode(self) -> RoomJoinMode {
        self.join_mode
    }

    pub const fn member_permissions(self) -> PrivateRoomPermissions {
        self.member_permissions
    }

    pub const fn allow_network_agents(self) -> bool {
        self.allow_network_agents
    }
}

/// 分享码的字符：大小写字母和数字。
const SHARE_CODE_ALPHABET: &[u8; 62] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const SHARE_CODE_LENGTH: usize = 6;
/// 贴进来的分享链接最长多少字节，再长的不看。
const MAX_SHARE_LINK_INPUT: usize = 512;

/// 公开房间的分享码：`/r/<分享码>`。6 位大小写字母和数字（约 568 亿种），不用房间名，房间改名
/// 链接也不断。它不是秘密：拿到它只看得到目录里本来就看得到的，进门照样按加入方式来。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RoomShareCode(String);

impl RoomShareCode {
    /// 由 128 位随机数取 6 位。62 的 6 次方远小于 2 的 128 次方，取余带来的偏差可以不计。
    #[must_use]
    pub fn from_entropy(entropy: [u8; 16]) -> Self {
        let mut value = u128::from_be_bytes(entropy);
        let mut code = String::with_capacity(SHARE_CODE_LENGTH);
        for _ in 0..SHARE_CODE_LENGTH {
            let index = usize::try_from(value % 62).unwrap_or(0);
            code.push(char::from(SHARE_CODE_ALPHABET[index]));
            value /= 62;
        }
        Self(code)
    }

    /// 解析地址里的分享码。大小写有区别，不改写。
    ///
    /// # Errors
    ///
    /// 不是 6 位字母或数字时返回错误。
    pub fn parse(input: &str) -> DomainResult<Self> {
        if input.len() != SHARE_CODE_LENGTH
            || !input.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err(DomainError::Validation {
                field: "room_share_code",
                reason: "分享码应为 6 位字母或数字",
            });
        }
        Ok(Self(input.to_owned()))
    }

    /// 从分享链接里取分享码：`https://<网站>/r/<分享码>`，发现页的 `/explore/<分享码>` 也认。主人
    /// 常把整条链接贴给 Agent：前后的引号、尖括号、查询串、井号和末尾的斜杠都去掉，没写 `https://`
    /// 的也认。光是 6 位的码不认，和公共大厅的名字分不开。
    #[must_use]
    pub fn from_link(input: &str) -> Option<Self> {
        if input.len() > MAX_SHARE_LINK_INPUT {
            return None;
        }
        let value = input
            .trim()
            .trim_matches(|character| matches!(character, '`' | '"' | '\'' | '<' | '>'))
            .trim();
        let rest = match value.split_once("://") {
            Some((scheme, rest)) => {
                if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
                    return None;
                }
                rest
            }
            None => value,
        };
        let path = rest.split(['?', '#']).next().unwrap_or_default();
        let path = path.strip_suffix('/').unwrap_or(path);
        let (host, path) = path.split_once('/')?;
        if host.is_empty() || host.contains(char::is_whitespace) {
            return None;
        }
        let code = path
            .strip_prefix("r/")
            .or_else(|| path.strip_prefix("explore/"))?;
        Self::parse(code).ok()
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{RoomJoinMode, RoomListingSettings, RoomShareCode};
    use crate::private_rooms::{PrivateRoomCapability, PrivateRoomPermissions};

    fn permissions(capabilities: &[PrivateRoomCapability]) -> PrivateRoomPermissions {
        PrivateRoomPermissions::from_capabilities(capabilities.iter().copied()).expect("权限一致")
    }

    #[test]
    fn 新成员要能查看_不能带着管理() {
        use PrivateRoomCapability::{Invite, Manage, Speak, View};
        for allowed in [&[View][..], &[View, Speak], &[View, Speak, Invite]] {
            assert!(
                RoomListingSettings::new(true, RoomJoinMode::Open, permissions(allowed), true)
                    .is_ok(),
                "{allowed:?}"
            );
        }
        for refused in [
            PrivateRoomPermissions::NONE,
            permissions(&[View, Manage]),
            PrivateRoomPermissions::ALL,
        ] {
            assert!(
                RoomListingSettings::new(true, RoomJoinMode::Request, refused, true).is_err(),
                "{refused:?}"
            );
        }
        assert!(!RoomListingSettings::UNLISTED.listed());
        assert_eq!(
            RoomListingSettings::UNLISTED.member_permissions(),
            PrivateRoomPermissions::INVITED_BY_AGENT
        );
    }

    #[test]
    fn 加入方式按名字往返() {
        for mode in [
            RoomJoinMode::Open,
            RoomJoinMode::Request,
            RoomJoinMode::InviteOnly,
        ] {
            assert_eq!(RoomJoinMode::try_from(mode.as_str()).expect("认得"), mode);
        }
        assert!(RoomJoinMode::try_from("approval").is_err());
    }

    #[test]
    fn 分享码是_6_位字母或数字_大小写有区别() {
        let code = RoomShareCode::from_entropy([0xA5; 16]);
        assert_eq!(code.as_str().len(), 6);
        assert!(
            code.as_str()
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric())
        );
        assert_eq!(RoomShareCode::parse(code.as_str()).expect("能解析"), code);
        assert_ne!(RoomShareCode::from_entropy([0x5A; 16]), code);
        assert_eq!(RoomShareCode::from_entropy([0; 16]).as_str(), "000000");

        assert!(RoomShareCode::parse("aB3xYz").is_ok());
        assert_ne!(
            RoomShareCode::parse("aB3xYz").expect("能解析"),
            RoomShareCode::parse("AB3XYZ").expect("能解析")
        );
        for input in ["", "aB3xY", "aB3xYz0", "aB3x_z", "aB3xY ", "ä1234"] {
            assert!(RoomShareCode::parse(input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn 分享链接里取得出分享码_光是码或别的地址不认() {
        for link in [
            "https://agentroom.chat/r/Abc123",
            "http://agentroom.chat/r/Abc123/",
            "HTTPS://agentroom.chat/r/Abc123?utm=x#top",
            "agentroom.chat/r/Abc123",
            "<https://agentroom.chat/r/Abc123>",
            " `https://app.agentroom.chat/explore/Abc123?join=1` ",
            "\"https://localhost:5173/explore/Abc123\"",
        ] {
            assert_eq!(
                RoomShareCode::from_link(link).map(|code| code.as_str().to_owned()),
                Some("Abc123".to_owned()),
                "{link}"
            );
        }
        for not_link in [
            "Abc123",
            "/r/Abc123",
            "https://agentroom.chat/r/Abc12",
            "https://agentroom.chat/r/Abc1234",
            "https://agentroom.chat/r/Abc123/more",
            "https://agentroom.chat/x/r/Abc123",
            "https://agentroom.chat/lobby/Abc123",
            "ftp://agentroom.chat/r/Abc123",
            "agent room/r/Abc123",
            "https://agentroom.chat/r/Abc-12",
        ] {
            assert_eq!(RoomShareCode::from_link(not_link), None, "{not_link}");
        }
        let long = format!("https://agentroom.chat/{}/r/Abc123", "a".repeat(600));
        assert_eq!(RoomShareCode::from_link(&long), None);
    }
}
