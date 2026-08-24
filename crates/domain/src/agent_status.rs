use crate::{
    DomainError, DomainResult,
    ids::AgentInstanceId,
    time::{DurationMillis, UtcMillis},
};

const MAX_TASK_SUMMARY_CHARACTERS: usize = 160;
const PROGRESS_BASIS_POINTS: u16 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentWorkStatus {
    Offline,
    Idle,
    Working,
    WaitingInput,
    Blocked,
    Completed,
}

impl AgentWorkStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Idle => "idle",
            Self::Working => "working",
            Self::WaitingInput => "waiting_input",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentStatusVisibility {
    Coarse,
    Detailed,
}

impl AgentStatusVisibility {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Coarse => "coarse",
            Self::Detailed => "detailed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTaskSummary(String);

impl AgentTaskSummary {
    /// 创建可公开展示的脱敏任务摘要。
    ///
    /// # Errors
    ///
    /// 空白、超过 160 个字符或含控制字符时返回校验错误。
    pub fn new(value: impl Into<String>) -> DomainResult<Self> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(DomainError::Validation {
                field: "agent_task_summary",
                reason: "不能为空白",
            });
        }
        if value.chars().count() > MAX_TASK_SUMMARY_CHARACTERS {
            return Err(DomainError::Validation {
                field: "agent_task_summary",
                reason: "不能超过 160 个字符",
            });
        }
        if value.chars().any(char::is_control) {
            return Err(DomainError::Validation {
                field: "agent_task_summary",
                reason: "不能包含控制字符",
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentTaskProgress(u16);

impl AgentTaskProgress {
    /// 从万分比创建宿主明确提供的进度。
    ///
    /// # Errors
    ///
    /// 大于 `10_000` 时返回校验错误。
    pub fn from_basis_points(value: u16) -> DomainResult<Self> {
        if value > PROGRESS_BASIS_POINTS {
            return Err(DomainError::Validation {
                field: "agent_task_progress",
                reason: "必须位于 0 到 10_000 个基点之间",
            });
        }
        Ok(Self(value))
    }

    pub const fn basis_points(self) -> u16 {
        self.0
    }

    pub fn fraction(self) -> f64 {
        f64::from(self.0) / f64::from(PROGRESS_BASIS_POINTS)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStatusDetails {
    task_summary: Option<AgentTaskSummary>,
    started_at: Option<UtcMillis>,
    progress: Option<AgentTaskProgress>,
}

impl AgentStatusDetails {
    pub const fn new(
        task_summary: Option<AgentTaskSummary>,
        started_at: Option<UtcMillis>,
        progress: Option<AgentTaskProgress>,
    ) -> Self {
        Self {
            task_summary,
            started_at,
            progress,
        }
    }

    pub const fn task_summary(&self) -> Option<&AgentTaskSummary> {
        self.task_summary.as_ref()
    }

    pub const fn started_at(&self) -> Option<UtcMillis> {
        self.started_at
    }

    pub const fn progress(&self) -> Option<AgentTaskProgress> {
        self.progress
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStatusSnapshot {
    status: AgentWorkStatus,
    visibility: AgentStatusVisibility,
    details: Option<AgentStatusDetails>,
}

impl AgentStatusSnapshot {
    /// 创建面向单个房间的状态投影。
    ///
    /// # Errors
    ///
    /// 粗粒度可见性携带任务详情时返回不变式错误。
    pub fn new(
        status: AgentWorkStatus,
        visibility: AgentStatusVisibility,
        details: Option<AgentStatusDetails>,
    ) -> DomainResult<Self> {
        if visibility == AgentStatusVisibility::Coarse && details.is_some() {
            return Err(DomainError::InvariantViolation {
                entity: "agent_status_snapshot",
                rule: "粗粒度状态不得携带任务详情",
            });
        }
        Ok(Self {
            status,
            visibility,
            details,
        })
    }

    pub const fn status(&self) -> AgentWorkStatus {
        self.status
    }

    pub const fn visibility(&self) -> AgentStatusVisibility {
        self.visibility
    }

    pub const fn details(&self) -> Option<&AgentStatusDetails> {
        self.details.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStatusLease {
    agent_instance_id: AgentInstanceId,
    snapshot: AgentStatusSnapshot,
    published_at: UtcMillis,
    expires_at: UtcMillis,
}

impl AgentStatusLease {
    /// 签发一个有界工作状态租约。
    ///
    /// # Errors
    ///
    /// 过期时间计算溢出时返回错误。
    pub fn issue(
        agent_instance_id: AgentInstanceId,
        snapshot: AgentStatusSnapshot,
        published_at: UtcMillis,
        lifetime: DurationMillis,
    ) -> DomainResult<Self> {
        let expires_at = published_at.checked_add(lifetime)?;
        Ok(Self {
            agent_instance_id,
            snapshot,
            published_at,
            expires_at,
        })
    }

    pub const fn agent_instance_id(&self) -> AgentInstanceId {
        self.agent_instance_id
    }

    pub const fn snapshot(&self) -> &AgentStatusSnapshot {
        &self.snapshot
    }

    pub const fn published_at(&self) -> UtcMillis {
        self.published_at
    }

    pub const fn expires_at(&self) -> UtcMillis {
        self.expires_at
    }

    /// 使用本地时钟与允许偏差计算展示状态。
    ///
    /// # Errors
    ///
    /// 偏差加法溢出时返回错误。
    pub fn effective_status(
        &self,
        observed_at: UtcMillis,
        allowed_clock_skew: DurationMillis,
    ) -> DomainResult<AgentWorkStatus> {
        let tolerated_expiry = self.expires_at.checked_add(allowed_clock_skew)?;
        if observed_at >= tolerated_expiry {
            return Ok(AgentWorkStatus::Offline);
        }
        Ok(self.snapshot.status())
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use uuid::Uuid;

    use super::{
        AgentStatusDetails, AgentStatusLease, AgentStatusSnapshot, AgentStatusVisibility,
        AgentTaskProgress, AgentTaskSummary, AgentWorkStatus,
    };
    use crate::{
        ids::AgentInstanceId,
        time::{DurationMillis, UtcMillis},
    };

    #[test]
    fn 粗粒度状态拒绝携带任务详情() {
        let details = AgentStatusDetails::new(
            Some(AgentTaskSummary::new("正在编译").expect("摘要有效")),
            None,
            None,
        );
        assert!(
            AgentStatusSnapshot::new(
                AgentWorkStatus::Working,
                AgentStatusVisibility::Coarse,
                Some(details),
            )
            .is_err()
        );
    }

    #[test]
    fn 租约到期由读取端本地判为离线() {
        let lease = AgentStatusLease::issue(
            AgentInstanceId::from_uuid(Uuid::from_u128(1)),
            AgentStatusSnapshot::new(
                AgentWorkStatus::Working,
                AgentStatusVisibility::Coarse,
                None,
            )
            .expect("状态有效"),
            UtcMillis::new(1_000).expect("时间有效"),
            DurationMillis::new(300_000).expect("租期有效"),
        )
        .expect("租约有效");
        let skew = DurationMillis::new(30_000).expect("时钟偏差有效");

        assert_eq!(
            lease
                .effective_status(UtcMillis::new(330_999).expect("时间有效"), skew)
                .expect("状态可计算"),
            AgentWorkStatus::Working
        );
        assert_eq!(
            lease
                .effective_status(UtcMillis::new(331_000).expect("时间有效"), skew)
                .expect("状态可计算"),
            AgentWorkStatus::Offline
        );
    }

    #[test]
    fn 摘要拒绝控制字符和越界文本() {
        assert!(AgentTaskSummary::new("提示词\n泄漏").is_err());
        assert!(AgentTaskSummary::new("界".repeat(161)).is_err());
        assert!(AgentTaskSummary::new("已脱敏任务").is_ok());
    }

    proptest! {
        #[test]
        fn 任意合法进度都保持协议范围(value in 0_u16..=10_000) {
            let progress = AgentTaskProgress::from_basis_points(value).expect("进度有效");
            prop_assert!((0.0..=1.0).contains(&progress.fraction()));
            prop_assert_eq!(progress.basis_points(), value);
        }
    }
}
