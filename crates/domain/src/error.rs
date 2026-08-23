use thiserror::Error;

pub type DomainResult<T> = Result<T, DomainError>;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DomainError {
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
