use crate::{DomainError, DomainResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcMillis(i64);

impl UtcMillis {
    /// 创建 Unix epoch 之后的毫秒时间。
    ///
    /// # Errors
    ///
    /// 当数值为负数时返回校验错误。
    pub fn new(value: i64) -> DomainResult<Self> {
        if value < 0 {
            return Err(DomainError::Validation {
                field: "utc_millis",
                reason: "不能早于 Unix epoch",
            });
        }

        Ok(Self(value))
    }

    pub const fn value(self) -> i64 {
        self.0
    }

    /// 在当前时间上增加一个非零时长。
    ///
    /// # Errors
    ///
    /// 当时长无法转换为有符号整数或加法溢出时返回时间溢出错误。
    pub fn checked_add(self, duration: DurationMillis) -> DomainResult<Self> {
        let duration = i64::try_from(duration.value()).map_err(|_| DomainError::TimeOverflow)?;
        self.0
            .checked_add(duration)
            .map(Self)
            .ok_or(DomainError::TimeOverflow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DurationMillis(u64);

impl DurationMillis {
    /// 创建非零毫秒时长。
    ///
    /// # Errors
    ///
    /// 当数值为零时返回校验错误。
    pub fn new(value: u64) -> DomainResult<Self> {
        if value == 0 {
            return Err(DomainError::Validation {
                field: "duration_millis",
                reason: "必须大于零",
            });
        }

        Ok(Self(value))
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{DurationMillis, UtcMillis};

    #[test]
    fn 拒绝负时间和零时长() {
        assert!(UtcMillis::new(-1).is_err());
        assert!(DurationMillis::new(0).is_err());
    }
}
