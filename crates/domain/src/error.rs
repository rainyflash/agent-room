use thiserror::Error;

pub type DomainResult<T> = Result<T, DomainError>;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DomainError {
    #[error("无权执行操作：{action}")]
    Forbidden { action: &'static str },
    /// 管得了这间房，但这件事只有房主能做：对象是管理员，或者给出、拿掉的是“管理”这一项
    /// （`specs/redesign/room-roles.md`）。
    #[error("只有房主能执行操作：{action}")]
    OwnerOnly { action: &'static str },
    #[error("{entity} 违反不变式：{rule}")]
    InvariantViolation {
        entity: &'static str,
        rule: &'static str,
    },
    #[error("{entity} 不能从 {from} 转换为 {to}")]
    InvalidTransition {
        entity: &'static str,
        from: &'static str,
        to: &'static str,
    },
    #[error("字段 {field} 无效：{reason}")]
    Validation {
        field: &'static str,
        reason: &'static str,
    },
    #[error("房间容量已达到上限 {capacity}")]
    CapacityExceeded { capacity: u16 },
    #[error("时间计算溢出")]
    TimeOverflow,
    #[error("聚合版本计算溢出")]
    VersionOverflow,
}
