//! 房主的 Agent 管房间（`specs/public-rooms/design.md`“房主的 Agent 管房间”）：房主和管理员把一间房交给
//! 自己的一个 Agent 管，写下规则、勾上它能做哪几样、定多久。Agent 照规则自己动手，不用每次问主人。

use crate::{DomainError, DomainResult, time::DurationMillis};

/// 规则最多写多少个字（按 Unicode 字符数）。
pub const MAX_ROOM_MODERATION_RULES_CHARACTERS: usize = 2_000;

/// 一份授权最长多久（毫秒），和自动发言授权一样是 30 天；不填就是这么久。
pub const MAX_ROOM_MODERATION_GRANT_LIFETIME_MILLIS: u64 = 30 * 24 * 60 * 60 * 1_000;

/// 一份授权最短多久（毫秒）。
pub const MIN_ROOM_MODERATION_GRANT_LIFETIME_MILLIS: u64 = 60 * 1_000;

/// Agent 动手时写的理由最多多少个字（按 Unicode 字符数）。
pub const MAX_AGENT_MODERATION_NOTE_CHARACTERS: usize = 500;

/// Agent 限时禁言最短、最长多久（毫秒）：一分钟到 30 天。不给时长就一直禁着，直到有人撤回。
pub const MIN_AGENT_MUTE_MILLIS: u64 = 60 * 1_000;
pub const MAX_AGENT_MUTE_MILLIS: u64 = 30 * 24 * 60 * 60 * 1_000;

/// Agent 能做的一样。禁言可以限时；解封只解它自己封的（第 7a-2 步）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RoomModerationPower {
    Mute,
    Kick,
    Ban,
    Unban,
}

impl RoomModerationPower {
    pub const ALL: [Self; 4] = [Self::Mute, Self::Kick, Self::Ban, Self::Unban];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mute => "mute",
            Self::Kick => "kick",
            Self::Ban => "ban",
            Self::Unban => "unban",
        }
    }
}

impl TryFrom<&str> for RoomModerationPower {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "mute" => Ok(Self::Mute),
            "kick" => Ok(Self::Kick),
            "ban" => Ok(Self::Ban),
            "unban" => Ok(Self::Unban),
            _ => Err(DomainError::Validation {
                field: "room_moderation_power",
                reason: "只能是 mute、kick、ban、unban",
            }),
        }
    }
}

/// 勾上的几样：至少一样，不重复，按固定的顺序排。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomModerationPowers(Vec<RoomModerationPower>);

impl RoomModerationPowers {
    /// # Errors
    ///
    /// 一样都没勾时返回错误。重复的只算一次。
    pub fn new(powers: impl IntoIterator<Item = RoomModerationPower>) -> DomainResult<Self> {
        let mut powers: Vec<_> = powers.into_iter().collect();
        powers.sort_unstable();
        powers.dedup();
        if powers.is_empty() {
            return Err(DomainError::Validation {
                field: "room_moderation_powers",
                reason: "至少要勾一样",
            });
        }
        Ok(Self(powers))
    }

    #[must_use]
    pub fn as_slice(&self) -> &[RoomModerationPower] {
        &self.0
    }

    #[must_use]
    pub fn allows(&self, power: RoomModerationPower) -> bool {
        self.0.contains(&power)
    }
}

/// 主人写给 Agent 的规则：去掉首尾空白，1 到 2000 个字；能换行、能有制表符，不收回车和别的控制字符，
/// 和公告一样。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomModerationRules(String);

impl RoomModerationRules {
    /// # Errors
    ///
    /// 空的、超过 2000 个字，或带着换行和制表符以外的控制字符时返回错误。
    pub fn parse(input: &str) -> DomainResult<Self> {
        let rules = input.trim();
        if rules.is_empty()
            || rules.chars().count() > MAX_ROOM_MODERATION_RULES_CHARACTERS
            || rules
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
        {
            return Err(DomainError::Validation {
                field: "room_moderation_rules",
                reason: "规则要写 1 到 2000 个字，只能带换行和制表符这两种控制字符",
            });
        }
        Ok(Self(rules.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Agent 动手时写的理由：它照规则的哪一条、看到了什么。去掉首尾空白，1 到 500 个字；控制字符和规则一样，
/// 只收换行和制表符。房主和管理员在台账里看得到。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentModerationNote(String);

impl AgentModerationNote {
    /// # Errors
    ///
    /// 空的、超过 500 个字，或带着换行和制表符以外的控制字符时返回错误。
    pub fn parse(input: &str) -> DomainResult<Self> {
        let note = input.trim();
        if note.is_empty()
            || note.chars().count() > MAX_AGENT_MODERATION_NOTE_CHARACTERS
            || note
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
        {
            return Err(DomainError::Validation {
                field: "agent_moderation_note",
                reason: "理由要写 1 到 500 个字，只能带换行和制表符这两种控制字符",
            });
        }
        Ok(Self(note.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Agent 限时禁言的时长：不给就是一直禁着；给了要在一分钟到 30 天之间。
///
/// # Errors
///
/// 不在这个范围里时返回错误。
pub fn agent_mute_duration(
    requested: Option<DurationMillis>,
) -> DomainResult<Option<DurationMillis>> {
    match requested {
        Some(duration)
            if !(MIN_AGENT_MUTE_MILLIS..=MAX_AGENT_MUTE_MILLIS).contains(&duration.value()) =>
        {
            Err(DomainError::Validation {
                field: "agent_mute_duration",
                reason: "限时禁言要在一分钟到 30 天之间",
            })
        }
        requested => Ok(requested),
    }
}

/// 授权多久：不填是 30 天，填了要在一分钟到 30 天之间。
///
/// # Errors
///
/// 不在这个范围里时返回错误。
pub fn room_moderation_grant_lifetime(
    requested: Option<DurationMillis>,
) -> DomainResult<DurationMillis> {
    let millis = requested.map_or(
        MAX_ROOM_MODERATION_GRANT_LIFETIME_MILLIS,
        DurationMillis::value,
    );
    if !(MIN_ROOM_MODERATION_GRANT_LIFETIME_MILLIS..=MAX_ROOM_MODERATION_GRANT_LIFETIME_MILLIS)
        .contains(&millis)
    {
        return Err(DomainError::Validation {
            field: "room_moderation_grant_lifetime",
            reason: "授权要在一分钟到 30 天之间",
        });
    }
    DurationMillis::new(millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 勾的几样去重排好_一样没勾不行() {
        let powers = RoomModerationPowers::new([
            RoomModerationPower::Ban,
            RoomModerationPower::Mute,
            RoomModerationPower::Ban,
        ])
        .unwrap();
        assert_eq!(
            powers.as_slice(),
            [RoomModerationPower::Mute, RoomModerationPower::Ban]
        );
        assert!(powers.allows(RoomModerationPower::Ban));
        assert!(!powers.allows(RoomModerationPower::Kick));
        assert!(RoomModerationPowers::new([]).is_err());
        for power in RoomModerationPower::ALL {
            assert_eq!(
                RoomModerationPower::try_from(power.as_str()).unwrap(),
                power
            );
        }
        assert!(RoomModerationPower::try_from("hide").is_err());
    }

    #[test]
    fn 规则去掉首尾空白_空的和太长的不行() {
        let rules = RoomModerationRules::parse("  发广告链接的直接封禁。\n\t刷屏的禁言 10 分钟。 ")
            .unwrap();
        assert_eq!(
            rules.as_str(),
            "发广告链接的直接封禁。\n\t刷屏的禁言 10 分钟。"
        );
        assert!(RoomModerationRules::parse(" \n ").is_err());
        assert!(RoomModerationRules::parse("回车\r\n不行").is_err());
        let longest = "规".repeat(MAX_ROOM_MODERATION_RULES_CHARACTERS);
        assert!(RoomModerationRules::parse(&longest).is_ok());
        assert!(RoomModerationRules::parse(&format!("{longest}则")).is_err());
    }

    #[test]
    fn 不填是三十天_一分钟到三十天之间才行() {
        assert_eq!(
            room_moderation_grant_lifetime(None).unwrap().value(),
            MAX_ROOM_MODERATION_GRANT_LIFETIME_MILLIS
        );
        let hour = DurationMillis::new(60 * 60 * 1_000).unwrap();
        assert_eq!(room_moderation_grant_lifetime(Some(hour)).unwrap(), hour);
        assert!(
            room_moderation_grant_lifetime(Some(DurationMillis::new(59_999).unwrap())).is_err()
        );
        let longer = DurationMillis::new(30 * 24 * 60 * 60 * 1_000 + 1).unwrap();
        assert!(room_moderation_grant_lifetime(Some(longer)).is_err());
    }

    #[test]
    fn 理由去掉首尾空白_空的太长的带回车的不行() {
        let note =
            AgentModerationNote::parse("  连发了五条广告链接，按“发广告的封禁”。\n ").unwrap();
        assert_eq!(note.as_str(), "连发了五条广告链接，按“发广告的封禁”。");
        assert!(AgentModerationNote::parse("  ").is_err());
        assert!(AgentModerationNote::parse("回车\r不行").is_err());
        let longest = "理".repeat(MAX_AGENT_MODERATION_NOTE_CHARACTERS);
        assert!(AgentModerationNote::parse(&longest).is_ok());
        assert!(AgentModerationNote::parse(&format!("{longest}由")).is_err());
    }

    #[test]
    fn 限时禁言不给就一直禁着_给了要在一分钟到三十天之间() {
        assert_eq!(agent_mute_duration(None).unwrap(), None);
        let ten_minutes = DurationMillis::new(10 * 60 * 1_000).unwrap();
        assert_eq!(
            agent_mute_duration(Some(ten_minutes)).unwrap(),
            Some(ten_minutes)
        );
        assert!(agent_mute_duration(Some(DurationMillis::new(59_999).unwrap())).is_err());
        let longer = DurationMillis::new(MAX_AGENT_MUTE_MILLIS + 1).unwrap();
        assert!(agent_mute_duration(Some(longer)).is_err());
    }
}
