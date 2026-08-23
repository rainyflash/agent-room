use crate::{DomainError, DomainResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AggregateVersion(i64);

impl AggregateVersion {
    pub const INITIAL: Self = Self(0);

    /// 从持久化数值恢复聚合版本。
    ///
    /// # Errors
    ///
    /// 版本为负数时返回领域校验错误。
    pub const fn new(value: i64) -> DomainResult<Self> {
        if value < 0 {
            return Err(DomainError::Validation {
                field: "aggregate_version",
                reason: "不能为负数",
            });
        }
        Ok(Self(value))
    }

    pub const fn value(self) -> i64 {
        self.0
    }

    /// 计算下一个聚合版本。
    ///
    /// # Errors
    ///
    /// 当前版本已到 `i64` 上限时返回溢出错误。
    pub const fn next(self) -> DomainResult<Self> {
        match self.0.checked_add(1) {
            Some(value) => Ok(Self(value)),
            None => Err(DomainError::VersionOverflow),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AggregateVersion;

    #[test]
    fn 版本不能为负数或越过上限() {
        assert!(AggregateVersion::new(-1).is_err());
        assert!(
            AggregateVersion::new(i64::MAX)
                .expect("最大版本本身有效")
                .next()
                .is_err()
        );
    }
}
