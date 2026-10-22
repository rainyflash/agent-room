//! Agent 名字后面的四位编号（`specs/agent-name-tag/design.md`）：房间里同名的 Agent
//! 靠它分开。编号由服务器在建 Agent 时写进名字，名字照旧签进名片和每条消息，
//! 已经装着的客户端不用升级就显示。

use std::fmt;

use uuid::Uuid;

/// 带编号的名字最多这么多个字符，和协议、网页对名字的上限一样。
pub const MAX_TAGGED_NAME_CHARACTERS: usize = 80;

/// ` #` 加四位数字。
const TAG_CHARACTERS: usize = 6;

/// `0000` 到 `9999` 的编号。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentNameTag(u16);

impl AgentNameTag {
    /// 取 ID 末尾 32 位除以 10000 的余数。UUIDv7 的末段是随机数，同一个 Agent 重试、
    /// 重建时编号不变；补编号的迁移照同一个算法算。
    #[must_use]
    pub fn from_uuid(id: Uuid) -> Self {
        let bytes = id.as_bytes();
        let tail = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        // 余数小于 10000，放得进 u16。
        Self(u16::try_from(tail % 10_000).unwrap_or_default())
    }

    #[must_use]
    pub const fn value(self) -> u16 {
        self.0
    }
}

impl fmt::Display for AgentNameTag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "#{:04}", self.0)
    }
}

/// 去掉首尾空白和末尾的 ` #` 加四位数字：拿自己显示的名字再进一次不会变成两个编号，
/// 也没法把别人的编号写进自己的名字里冒充。
#[must_use]
pub fn untagged_name(name: &str) -> &str {
    let name = name.trim();
    let Some((base, digits)) = name.rsplit_once(" #") else {
        return name;
    };
    let base = base.trim_end();
    if digits.len() == 4 && digits.bytes().all(|byte| byte.is_ascii_digit()) && !base.is_empty() {
        base
    } else {
        name
    }
}

/// 要的名字加上编号：先去掉已有的编号，原名太长就截短原名，整个名字不超过
/// [`MAX_TAGGED_NAME_CHARACTERS`] 个字符。
#[must_use]
pub fn tagged_name(name: &str, tag: AgentNameTag) -> String {
    let room = MAX_TAGGED_NAME_CHARACTERS - TAG_CHARACTERS;
    let base: String = untagged_name(name).chars().take(room).collect();
    format!("{} {tag}", base.trim_end())
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{AgentNameTag, MAX_TAGGED_NAME_CHARACTERS, tagged_name, untagged_name};

    fn tag(value: &str) -> AgentNameTag {
        AgentNameTag::from_uuid(Uuid::parse_str(value).unwrap())
    }

    #[test]
    fn 编号取_id_末尾_32_位除以一万的余数() {
        // 0x000004d2 = 1234；0xffffffff = 4294967295，余 7295。
        assert_eq!(tag("01900000-0000-7000-8000-0000000004d2").value(), 1234);
        assert_eq!(tag("01900000-0000-7000-8000-0000ffffffff").value(), 7295);
        assert_eq!(
            tag("01900000-0000-7000-8000-000000000007").to_string(),
            "#0007"
        );
    }

    #[test]
    fn 名字后面加编号() {
        let tag = tag("01900000-0000-7000-8000-000000000910");
        assert_eq!(tag.to_string(), "#2320");
        assert_eq!(tagged_name("Codex", tag), "Codex #2320");
        assert_eq!(tagged_name("  Codex  ", tag), "Codex #2320");
    }

    #[test]
    fn 已经带编号的先去掉再加() {
        let tag = tag("01900000-0000-7000-8000-000000000910");
        assert_eq!(tagged_name("Codex #2316", tag), "Codex #2320");
        assert_eq!(tagged_name("Codex  #0001 ", tag), "Codex #2320");
        // 不是 ` #` 加四位数字的照旧留着。
        assert_eq!(tagged_name("Codex #12", tag), "Codex #12 #2320");
        assert_eq!(tagged_name("Codex#2316", tag), "Codex#2316 #2320");
        assert_eq!(tagged_name("#2316", tag), "#2316 #2320");
        assert_eq!(untagged_name(" #2316"), "#2316");
        assert_eq!(untagged_name("Codex #２３１６"), "Codex #２３１６");
    }

    #[test]
    fn 太长就截原名留编号() {
        let tag = tag("01900000-0000-7000-8000-000000000910");
        let long = "名".repeat(128);
        let tagged = tagged_name(&long, tag);
        assert_eq!(tagged.chars().count(), MAX_TAGGED_NAME_CHARACTERS);
        assert!(tagged.ends_with("名 #2320"));
        // 截在空格上时不留两个空格。
        let spaced = format!("{} tail", "a".repeat(73));
        assert_eq!(
            tagged_name(&spaced, tag),
            format!("{} #2320", "a".repeat(73))
        );
    }
}
