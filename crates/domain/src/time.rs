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

/// 用着就续的寿命：连续 `idle` 没用就过期，从起算时刻起最长 `maximum`。
///
/// 这台电脑的授权和账户登录都按它续期，见 `specs/session-lifetime/design.md`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlidingLifetime {
    idle: DurationMillis,
    maximum: DurationMillis,
}

impl SlidingLifetime {
    /// 创建用着就续的寿命。
    ///
    /// # Errors
    ///
    /// 空闲时限长于最长时限时返回校验错误。
    pub fn new(idle: DurationMillis, maximum: DurationMillis) -> DomainResult<Self> {
        if idle > maximum {
            return Err(DomainError::Validation {
                field: "sliding_lifetime",
                reason: "空闲时限不能长于最长时限",
            });
        }
        Ok(Self { idle, maximum })
    }

    pub const fn idle(self) -> DurationMillis {
        self.idle
    }

    pub const fn maximum(self) -> DurationMillis {
        self.maximum
    }

    /// 从 `anchor` 起算，最晚能用到什么时候。
    ///
    /// # Errors
    ///
    /// 加法溢出时返回时间溢出错误。
    pub fn latest(self, anchor: UtcMillis) -> DomainResult<UtcMillis> {
        anchor.checked_add(self.maximum)
    }

    /// `now` 用过一次以后续到什么时候：再给一个空闲时限，但不越过最晚时刻。
    ///
    /// # Errors
    ///
    /// 加法溢出时返回时间溢出错误。
    pub fn renewed_until(self, anchor: UtcMillis, now: UtcMillis) -> DomainResult<UtcMillis> {
        Ok(now.checked_add(self.idle)?.min(self.latest(anchor)?))
    }
}

#[cfg(test)]
mod tests {
    use super::{DurationMillis, SlidingLifetime, UtcMillis};

    const DAY: u64 = 24 * 60 * 60 * 1_000;

    fn at(day: u64) -> UtcMillis {
        UtcMillis::new(i64::try_from(day * DAY).expect("测试时间有效")).expect("测试时间有效")
    }

    fn lifetime() -> SlidingLifetime {
        SlidingLifetime::new(
            DurationMillis::new(30 * DAY).expect("时长有效"),
            DurationMillis::new(365 * DAY).expect("时长有效"),
        )
        .expect("寿命有效")
    }

    #[test]
    fn 拒绝负时间和零时长() {
        assert!(UtcMillis::new(-1).is_err());
        assert!(DurationMillis::new(0).is_err());
    }

    #[test]
    fn 用一次就从这次起再给一个空闲时限() {
        assert_eq!(lifetime().renewed_until(at(0), at(0)), Ok(at(30)));
        assert_eq!(lifetime().renewed_until(at(0), at(200)), Ok(at(230)));
    }

    #[test]
    fn 续期不越过起算时刻加最长时限() {
        assert_eq!(lifetime().latest(at(10)), Ok(at(375)));
        assert_eq!(lifetime().renewed_until(at(10), at(360)), Ok(at(375)));
        assert_eq!(lifetime().renewed_until(at(10), at(400)), Ok(at(375)));
    }

    #[test]
    fn 空闲时限不能长于最长时限() {
        let day = DurationMillis::new(DAY).expect("时长有效");
        let month = DurationMillis::new(30 * DAY).expect("时长有效");
        assert!(SlidingLifetime::new(month, day).is_err());
        assert!(SlidingLifetime::new(month, month).is_ok());
    }
}
