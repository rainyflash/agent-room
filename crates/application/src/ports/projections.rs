use agent_room_domain::{
    DomainError, DomainResult,
    ids::{AgentId, RoomInstanceId},
    time::{DurationMillis, UtcMillis},
};

use crate::persistence::RepositoryResult;

use super::PortFuture;

const MAX_CONSUMER_NAME_LENGTH: usize = 128;
const MAX_SYNC_TOKEN_LENGTH: usize = 4_096;
const MAX_EVENT_ID_LENGTH: usize = 512;
const MAX_ERROR_CODE_LENGTH: usize = 128;
const MAX_ACTIVITY_SCORE_MILLIS: u32 = 1_000_000;
const MAX_EVENTS_PER_BATCH: usize = 10_000;

pub const ROOM_PROJECTION_CONSUMER: &str = "matrix-room-projection-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixMembership {
    Invite,
    Join,
    Leave,
    Ban,
    Knock,
}

impl MatrixMembership {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Invite => "invite",
            Self::Join => "join",
            Self::Leave => "leave",
            Self::Ban => "ban",
            Self::Knock => "knock",
        }
    }

    pub const fn is_joined(self) -> bool {
        matches!(self, Self::Join)
    }
}

impl TryFrom<&str> for MatrixMembership {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "invite" => Ok(Self::Invite),
            "join" => Ok(Self::Join),
            "leave" => Ok(Self::Leave),
            "ban" => Ok(Self::Ban),
            "knock" => Ok(Self::Knock),
            _ => Err(DomainError::Validation {
                field: "matrix_membership",
                reason: "不是受支持的 Matrix 成员状态",
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityScoreMillis(u32);

impl ActivityScoreMillis {
    /// 创建活动度增量，单位为千分之一分。
    ///
    /// # Errors
    ///
    /// 零值或超过单事件上限时返回校验错误。
    pub fn new(value: u32) -> DomainResult<Self> {
        if value == 0 || value > MAX_ACTIVITY_SCORE_MILLIS {
            return Err(DomainError::Validation {
                field: "activity_score_millis",
                reason: "必须处于 1 到 1000000 之间",
            });
        }
        Ok(Self(value))
    }

    pub const fn value(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixProjectionEventKind {
    MembershipChanged {
        room_instance_id: RoomInstanceId,
        agent_id: AgentId,
        membership: MatrixMembership,
        power_level: i16,
    },
    ActivityObserved {
        room_instance_id: RoomInstanceId,
        score: ActivityScoreMillis,
    },
}

impl MatrixProjectionEventKind {
    pub const fn room_instance_id(self) -> RoomInstanceId {
        match self {
            Self::MembershipChanged {
                room_instance_id, ..
            }
            | Self::ActivityObserved {
                room_instance_id, ..
            } => room_instance_id,
        }
    }

    pub const fn event_kind(self) -> &'static str {
        match self {
            Self::MembershipChanged { .. } => "membership_changed",
            Self::ActivityObserved { .. } => "activity_observed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixProjectionEvent {
    event_id: String,
    event_digest: [u8; 32],
    kind: MatrixProjectionEventKind,
}

impl MatrixProjectionEvent {
    /// 创建去重所需信息完整的 Matrix 投影事件。
    ///
    /// # Errors
    ///
    /// 事件 ID 长度无效或成员权限级别越界时返回校验错误。
    pub fn new(
        event_id: String,
        event_digest: [u8; 32],
        kind: MatrixProjectionEventKind,
    ) -> DomainResult<Self> {
        validate_text("event_id", &event_id, 4, MAX_EVENT_ID_LENGTH)?;
        if let MatrixProjectionEventKind::MembershipChanged { power_level, .. } = kind
            && !(-100..=100).contains(&power_level)
        {
            return Err(DomainError::Validation {
                field: "power_level",
                reason: "必须处于 -100 到 100 之间",
            });
        }

        Ok(Self {
            event_id,
            event_digest,
            kind,
        })
    }

    pub fn event_id(&self) -> &str {
        &self.event_id
    }

    pub const fn event_digest(&self) -> &[u8; 32] {
        &self.event_digest
    }

    pub const fn kind(&self) -> MatrixProjectionEventKind {
        self.kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixProjectionBatch {
    consumer_name: String,
    expected_sync_token: Option<String>,
    next_sync_token: String,
    events: Vec<MatrixProjectionEvent>,
    projected_at: UtcMillis,
}

impl MatrixProjectionBatch {
    /// 创建带游标比较交换条件的增量投影批次。
    ///
    /// # Errors
    ///
    /// 消费者名或同步令牌长度无效时返回校验错误。
    pub fn new(
        consumer_name: String,
        expected_sync_token: Option<String>,
        next_sync_token: String,
        events: Vec<MatrixProjectionEvent>,
        projected_at: UtcMillis,
    ) -> DomainResult<Self> {
        validate_consumer_and_tokens(
            &consumer_name,
            expected_sync_token.as_deref(),
            &next_sync_token,
        )?;
        validate_event_count(events.len())?;
        Ok(Self {
            consumer_name,
            expected_sync_token,
            next_sync_token,
            events,
            projected_at,
        })
    }

    pub fn consumer_name(&self) -> &str {
        &self.consumer_name
    }

    pub fn expected_sync_token(&self) -> Option<&str> {
        self.expected_sync_token.as_deref()
    }

    pub fn next_sync_token(&self) -> &str {
        &self.next_sync_token
    }

    pub fn events(&self) -> &[MatrixProjectionEvent] {
        &self.events
    }

    pub const fn projected_at(&self) -> UtcMillis {
        self.projected_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixProjectionRebuild {
    consumer_name: String,
    next_sync_token: String,
    events: Vec<MatrixProjectionEvent>,
    projected_at: UtcMillis,
}

impl MatrixProjectionRebuild {
    /// 创建完整快照重建请求。
    ///
    /// # Errors
    ///
    /// 消费者名或最终同步令牌长度无效时返回校验错误。
    pub fn new(
        consumer_name: String,
        next_sync_token: String,
        events: Vec<MatrixProjectionEvent>,
        projected_at: UtcMillis,
    ) -> DomainResult<Self> {
        validate_consumer_and_tokens(&consumer_name, None, &next_sync_token)?;
        validate_event_count(events.len())?;
        Ok(Self {
            consumer_name,
            next_sync_token,
            events,
            projected_at,
        })
    }

    pub fn consumer_name(&self) -> &str {
        &self.consumer_name
    }

    pub fn next_sync_token(&self) -> &str {
        &self.next_sync_token
    }

    pub fn events(&self) -> &[MatrixProjectionEvent] {
        &self.events
    }

    pub const fn projected_at(&self) -> UtcMillis {
        self.projected_at
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionHealth {
    Healthy,
    Lagging,
    Failed,
}

impl ProjectionHealth {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Lagging => "lagging",
            Self::Failed => "failed",
        }
    }
}

impl TryFrom<&str> for ProjectionHealth {
    type Error = DomainError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "healthy" => Ok(Self::Healthy),
            "lagging" => Ok(Self::Lagging),
            "failed" => Ok(Self::Failed),
            _ => Err(DomainError::Validation {
                field: "projection_health",
                reason: "不是受支持的投影健康状态",
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionHealthReport {
    consumer_name: String,
    health: ProjectionHealth,
    error_code: Option<String>,
    observed_at: UtcMillis,
}

impl ProjectionHealthReport {
    /// 创建投影消费者健康报告。
    ///
    /// # Errors
    ///
    /// 健康状态携带错误码、异常状态缺少错误码或字段长度无效时返回校验错误。
    pub fn new(
        consumer_name: String,
        health: ProjectionHealth,
        error_code: Option<String>,
        observed_at: UtcMillis,
    ) -> DomainResult<Self> {
        validate_consumer_name(&consumer_name)?;
        match (health, error_code.as_deref()) {
            (ProjectionHealth::Healthy, None)
            | (ProjectionHealth::Lagging | ProjectionHealth::Failed, Some(_)) => {}
            (ProjectionHealth::Healthy, Some(_)) => {
                return Err(DomainError::Validation {
                    field: "error_code",
                    reason: "健康状态不能携带错误码",
                });
            }
            (ProjectionHealth::Lagging | ProjectionHealth::Failed, None) => {
                return Err(DomainError::Validation {
                    field: "error_code",
                    reason: "异常状态必须携带稳定错误码",
                });
            }
        }
        if let Some(code) = error_code.as_deref() {
            validate_text("error_code", code, 1, MAX_ERROR_CODE_LENGTH)?;
        }

        Ok(Self {
            consumer_name,
            health,
            error_code,
            observed_at,
        })
    }

    pub fn consumer_name(&self) -> &str {
        &self.consumer_name
    }

    pub const fn health(&self) -> ProjectionHealth {
        self.health
    }

    pub fn error_code(&self) -> Option<&str> {
        self.error_code.as_deref()
    }

    pub const fn observed_at(&self) -> UtcMillis {
        self.observed_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionCursor {
    sync_token: String,
    last_event_id: Option<String>,
    health: ProjectionHealth,
    updated_at: UtcMillis,
    version: u64,
}

impl ProjectionCursor {
    pub fn restore(
        sync_token: String,
        last_event_id: Option<String>,
        health: ProjectionHealth,
        updated_at: UtcMillis,
        version: u64,
    ) -> Self {
        Self {
            sync_token,
            last_event_id,
            health,
            updated_at,
            version,
        }
    }

    pub fn sync_token(&self) -> &str {
        &self.sync_token
    }

    pub fn last_event_id(&self) -> Option<&str> {
        self.last_event_id.as_deref()
    }

    pub const fn health(&self) -> ProjectionHealth {
        self.health
    }

    pub const fn updated_at(&self) -> UtcMillis {
        self.updated_at
    }

    pub const fn version(&self) -> u64 {
        self.version
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionApplyOutcome {
    Applied { new_events: u32, duplicates: u32 },
    Replayed { duplicates: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MembershipProjectionLookup {
    membership: Option<MatrixMembership>,
    power_level: Option<i16>,
    membership_projected_at: Option<UtcMillis>,
    cursor_updated_at: UtcMillis,
    health: ProjectionHealth,
}

impl MembershipProjectionLookup {
    pub const fn restore(
        membership: Option<MatrixMembership>,
        power_level: Option<i16>,
        membership_projected_at: Option<UtcMillis>,
        cursor_updated_at: UtcMillis,
        health: ProjectionHealth,
    ) -> Self {
        Self {
            membership,
            power_level,
            membership_projected_at,
            cursor_updated_at,
            health,
        }
    }

    pub const fn membership(self) -> Option<MatrixMembership> {
        self.membership
    }

    pub const fn power_level(self) -> Option<i16> {
        self.power_level
    }

    pub const fn membership_projected_at(self) -> Option<UtcMillis> {
        self.membership_projected_at
    }

    pub const fn cursor_updated_at(self) -> UtcMillis {
        self.cursor_updated_at
    }

    pub const fn health(self) -> ProjectionHealth {
        self.health
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembershipReadPlan {
    UseProjection,
    QueryMatrixMissing,
    QueryMatrixUnhealthy,
    QueryMatrixStale,
    QueryMatrixClockSkew,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectionFreshnessPolicy {
    maximum_age: DurationMillis,
}

impl ProjectionFreshnessPolicy {
    pub const fn new(maximum_age: DurationMillis) -> Self {
        Self { maximum_age }
    }

    pub fn plan(
        self,
        now: UtcMillis,
        projection: Option<&MembershipProjectionLookup>,
    ) -> MembershipReadPlan {
        let Some(projection) = projection else {
            return MembershipReadPlan::QueryMatrixMissing;
        };
        if projection.health() != ProjectionHealth::Healthy {
            return MembershipReadPlan::QueryMatrixUnhealthy;
        }

        let Some(age) = now
            .value()
            .checked_sub(projection.cursor_updated_at().value())
        else {
            return MembershipReadPlan::QueryMatrixClockSkew;
        };
        let Ok(age) = u64::try_from(age) else {
            return MembershipReadPlan::QueryMatrixClockSkew;
        };
        if age > self.maximum_age.value() {
            MembershipReadPlan::QueryMatrixStale
        } else {
            MembershipReadPlan::UseProjection
        }
    }
}

pub trait MatrixProjectionStore: Send + Sync {
    fn apply<'a>(
        &'a self,
        batch: &'a MatrixProjectionBatch,
    ) -> PortFuture<'a, RepositoryResult<ProjectionApplyOutcome>>;

    fn rebuild<'a>(
        &'a self,
        rebuild: &'a MatrixProjectionRebuild,
    ) -> PortFuture<'a, RepositoryResult<ProjectionApplyOutcome>>;

    fn cursor<'a>(
        &'a self,
        consumer_name: &'a str,
    ) -> PortFuture<'a, RepositoryResult<Option<ProjectionCursor>>>;

    fn membership<'a>(
        &'a self,
        consumer_name: &'a str,
        room_instance_id: RoomInstanceId,
        agent_id: AgentId,
    ) -> PortFuture<'a, RepositoryResult<Option<MembershipProjectionLookup>>>;

    fn report_health<'a>(
        &'a self,
        report: &'a ProjectionHealthReport,
    ) -> PortFuture<'a, RepositoryResult<()>>;
}

fn validate_consumer_and_tokens(
    consumer_name: &str,
    expected_sync_token: Option<&str>,
    next_sync_token: &str,
) -> DomainResult<()> {
    validate_consumer_name(consumer_name)?;
    if let Some(token) = expected_sync_token {
        validate_text("expected_sync_token", token, 1, MAX_SYNC_TOKEN_LENGTH)?;
    }
    validate_text("next_sync_token", next_sync_token, 1, MAX_SYNC_TOKEN_LENGTH)
}

fn validate_consumer_name(consumer_name: &str) -> DomainResult<()> {
    validate_text("consumer_name", consumer_name, 1, MAX_CONSUMER_NAME_LENGTH)?;
    if consumer_name != ROOM_PROJECTION_CONSUMER {
        return Err(DomainError::Validation {
            field: "consumer_name",
            reason: "房间查询投影只允许规范化单写消费者",
        });
    }
    Ok(())
}

fn validate_text(
    field: &'static str,
    value: &str,
    minimum: usize,
    maximum: usize,
) -> DomainResult<()> {
    if !(minimum..=maximum).contains(&value.len()) {
        return Err(DomainError::Validation {
            field,
            reason: "长度超出允许范围",
        });
    }
    Ok(())
}

fn validate_event_count(count: usize) -> DomainResult<()> {
    if count > MAX_EVENTS_PER_BATCH {
        return Err(DomainError::Validation {
            field: "events",
            reason: "单批事件数不能超过 10000",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use agent_room_domain::time::{DurationMillis, UtcMillis};

    use super::{
        MatrixProjectionBatch, MembershipProjectionLookup, MembershipReadPlan,
        ProjectionFreshnessPolicy, ProjectionHealth,
    };

    #[test]
    fn 安全读取只信任健康且未过期的投影() {
        let policy =
            ProjectionFreshnessPolicy::new(DurationMillis::new(5_000).expect("最大陈旧时间有效"));
        let now = UtcMillis::new(10_000).expect("当前时间有效");
        let fresh = MembershipProjectionLookup::restore(
            None,
            None,
            None,
            UtcMillis::new(6_000).expect("游标时间有效"),
            ProjectionHealth::Healthy,
        );
        let stale = MembershipProjectionLookup::restore(
            None,
            None,
            None,
            UtcMillis::new(4_999).expect("游标时间有效"),
            ProjectionHealth::Healthy,
        );
        let failed =
            MembershipProjectionLookup::restore(None, None, None, now, ProjectionHealth::Failed);
        let future = MembershipProjectionLookup::restore(
            None,
            None,
            None,
            UtcMillis::new(10_001).expect("游标时间有效"),
            ProjectionHealth::Healthy,
        );

        assert_eq!(
            policy.plan(now, Some(&fresh)),
            MembershipReadPlan::UseProjection
        );
        assert_eq!(
            policy.plan(now, Some(&stale)),
            MembershipReadPlan::QueryMatrixStale
        );
        assert_eq!(
            policy.plan(now, Some(&failed)),
            MembershipReadPlan::QueryMatrixUnhealthy
        );
        assert_eq!(
            policy.plan(now, Some(&future)),
            MembershipReadPlan::QueryMatrixClockSkew
        );
        assert_eq!(
            policy.plan(now, None),
            MembershipReadPlan::QueryMatrixMissing
        );
    }

    #[test]
    fn 房间查询投影拒绝第二个写入消费者() {
        let batch = MatrixProjectionBatch::new(
            "rogue-room-projector".to_owned(),
            None,
            "sync-1".to_owned(),
            Vec::new(),
            UtcMillis::new(1_000).expect("投影时间有效"),
        );

        assert!(batch.is_err());
    }
}
