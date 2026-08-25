use crate::{DomainError, DomainResult};

pub const ACTIVE_EVENT_NAMESPACE: &str = "io.github.rainyflash.agentroom";
const LEGACY_EVENT_NAMESPACE: &str = concat!("org", ".agentroom");
const KNOWN_EVENT_FAMILIES: [&str; 9] = [
    "agent.profile",
    "agent.status",
    "handoff.receipt",
    "handoff.request",
    "message.preview",
    "message.revision",
    "moderation.notice",
    "room.policy",
    "task.reference",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolMajor(u16);

impl ProtocolMajor {
    /// 创建非零协议主版本。
    ///
    /// # Errors
    ///
    /// 主版本为零时返回校验错误。
    pub const fn new(value: u16) -> DomainResult<Self> {
        if value == 0 {
            return Err(DomainError::Validation {
                field: "protocol_major",
                reason: "必须大于零",
            });
        }
        Ok(Self(value))
    }

    pub const fn value(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolMode {
    ReadWrite { negotiated: ProtocolMajor },
    ReadOnlyUpgradeRequired { remote_latest: ProtocolMajor },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolCompatibilityPolicy {
    current: ProtocolMajor,
    previous: ProtocolMajor,
}

impl ProtocolCompatibilityPolicy {
    /// 建立只允许当前和前一主版本的兼容策略。
    ///
    /// # Errors
    ///
    /// 两个版本不连续或顺序颠倒时返回校验错误。
    pub const fn new(current: ProtocolMajor, previous: ProtocolMajor) -> DomainResult<Self> {
        if current.0 != previous.0 + 1 {
            return Err(DomainError::Validation {
                field: "protocol_compatibility",
                reason: "当前主版本必须紧邻前一主版本",
            });
        }
        Ok(Self { current, previous })
    }

    pub const fn public_test() -> Self {
        Self {
            current: ProtocolMajor(2),
            previous: ProtocolMajor(1),
        }
    }

    /// 与对端能力清单协商最高公共主版本。
    ///
    /// 没有公共版本时保持只读，从而允许用户看见未知事件但禁止写入错误语义。
    ///
    /// # Errors
    ///
    /// 对端未声明任何协议版本时返回校验错误。
    pub fn negotiate(self, remote: &[ProtocolMajor]) -> DomainResult<ProtocolMode> {
        if remote.is_empty() {
            return Err(DomainError::Validation {
                field: "remote_protocol_versions",
                reason: "不能为空",
            });
        }
        if remote.contains(&self.current) {
            return Ok(ProtocolMode::ReadWrite {
                negotiated: self.current,
            });
        }
        if remote.contains(&self.previous) {
            return Ok(ProtocolMode::ReadWrite {
                negotiated: self.previous,
            });
        }
        let Some(remote_latest) = remote.iter().copied().max() else {
            return Err(DomainError::InvariantViolation {
                entity: "protocol_compatibility",
                rule: "非空版本清单必须存在最大值",
            });
        };
        Ok(ProtocolMode::ReadOnlyUpgradeRequired { remote_latest })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventCompatibility {
    CurrentWritable,
    LegacyReadOnly,
    UnknownReadOnly,
}

impl EventCompatibility {
    pub fn classify(event_type: &str) -> Self {
        if is_known_event(event_type, ACTIVE_EVENT_NAMESPACE) {
            return Self::CurrentWritable;
        }
        if is_known_event(event_type, LEGACY_EVENT_NAMESPACE) {
            return Self::LegacyReadOnly;
        }
        Self::UnknownReadOnly
    }
}

fn is_known_event(event_type: &str, namespace: &str) -> bool {
    KNOWN_EVENT_FAMILIES
        .iter()
        .any(|family| event_type == format!("{namespace}.{family}.v1"))
}

#[cfg(test)]
mod tests {
    use super::{
        ACTIVE_EVENT_NAMESPACE, EventCompatibility, ProtocolCompatibilityPolicy, ProtocolMajor,
        ProtocolMode,
    };

    #[test]
    fn 协商优先当前版本并兼容前一主版本() {
        let policy = ProtocolCompatibilityPolicy::public_test();
        assert_eq!(
            policy
                .negotiate(&[
                    ProtocolMajor::new(1).expect("版本有效"),
                    ProtocolMajor::new(2).expect("版本有效"),
                ])
                .expect("协商成功"),
            ProtocolMode::ReadWrite {
                negotiated: ProtocolMajor::new(2).expect("版本有效")
            }
        );
        assert_eq!(
            policy
                .negotiate(&[ProtocolMajor::new(1).expect("版本有效")])
                .expect("协商成功"),
            ProtocolMode::ReadWrite {
                negotiated: ProtocolMajor::new(1).expect("版本有效")
            }
        );
    }

    #[test]
    fn 无公共版本和未知事件都只能只读() {
        assert_eq!(
            ProtocolCompatibilityPolicy::public_test()
                .negotiate(&[ProtocolMajor::new(9).expect("版本有效")])
                .expect("未知版本可安全降级"),
            ProtocolMode::ReadOnlyUpgradeRequired {
                remote_latest: ProtocolMajor::new(9).expect("版本有效")
            }
        );
        assert_eq!(
            EventCompatibility::classify("com.example.future.event.v9"),
            EventCompatibility::UnknownReadOnly
        );
    }

    #[test]
    fn 新命名空间可写而旧命名空间只能读取() {
        assert_eq!(
            EventCompatibility::classify(&format!("{ACTIVE_EVENT_NAMESPACE}.message.preview.v1")),
            EventCompatibility::CurrentWritable
        );
        assert_eq!(
            EventCompatibility::classify(&format!(
                "{}.message.preview.v1",
                concat!("org", ".agentroom")
            )),
            EventCompatibility::LegacyReadOnly
        );
    }
}
