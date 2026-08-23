use std::num::NonZeroU16;

use agent_room_domain::{DomainError, DomainResult, ids::OutboxEventId, time::UtcMillis};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::persistence::RepositoryResult;

use super::PortFuture;

const MAX_EVENT_NAME_LENGTH: usize = 128;
const MAX_WORKER_NAME_LENGTH: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxMessage {
    id: OutboxEventId,
    aggregate_type: String,
    aggregate_id: Uuid,
    event_type: String,
    payload: Map<String, Value>,
    occurred_at: UtcMillis,
}

impl OutboxMessage {
    /// 创建待发布领域事件。
    ///
    /// # Errors
    ///
    /// 聚合类型或事件类型不符合稳定事件名约束时返回校验错误。
    pub fn new(
        id: OutboxEventId,
        aggregate_type: String,
        aggregate_id: Uuid,
        event_type: String,
        payload: Map<String, Value>,
        occurred_at: UtcMillis,
    ) -> DomainResult<Self> {
        validate_event_name("aggregate_type", &aggregate_type)?;
        validate_event_name("event_type", &event_type)?;

        Ok(Self {
            id,
            aggregate_type,
            aggregate_id,
            event_type,
            payload,
            occurred_at,
        })
    }

    pub const fn id(&self) -> OutboxEventId {
        self.id
    }

    pub fn aggregate_type(&self) -> &str {
        &self.aggregate_type
    }

    pub const fn aggregate_id(&self) -> Uuid {
        self.aggregate_id
    }

    pub fn event_type(&self) -> &str {
        &self.event_type
    }

    pub const fn payload(&self) -> &Map<String, Value> {
        &self.payload
    }

    pub const fn occurred_at(&self) -> UtcMillis {
        self.occurred_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxClaim {
    worker_name: String,
    batch_size: NonZeroU16,
    claimed_at: UtcMillis,
    lease_expires_at: UtcMillis,
}

impl OutboxClaim {
    /// 创建一次有界租约领取请求。
    ///
    /// # Errors
    ///
    /// Worker 名称无效或租约没有严格晚于领取时间时返回校验错误。
    pub fn new(
        worker_name: String,
        batch_size: NonZeroU16,
        claimed_at: UtcMillis,
        lease_expires_at: UtcMillis,
    ) -> DomainResult<Self> {
        validate_bounded_text("worker_name", &worker_name, MAX_WORKER_NAME_LENGTH)?;
        if lease_expires_at <= claimed_at {
            return Err(DomainError::Validation {
                field: "lease_expires_at",
                reason: "必须晚于领取时间",
            });
        }

        Ok(Self {
            worker_name,
            batch_size,
            claimed_at,
            lease_expires_at,
        })
    }

    pub fn worker_name(&self) -> &str {
        &self.worker_name
    }

    pub const fn batch_size(&self) -> NonZeroU16 {
        self.batch_size
    }

    pub const fn claimed_at(&self) -> UtcMillis {
        self.claimed_at
    }

    pub const fn lease_expires_at(&self) -> UtcMillis {
        self.lease_expires_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedOutboxEvent {
    message: OutboxMessage,
    attempt_count: u16,
    worker_name: String,
    lease_expires_at: UtcMillis,
}

impl ClaimedOutboxEvent {
    pub fn restore(
        message: OutboxMessage,
        attempt_count: u16,
        worker_name: String,
        lease_expires_at: UtcMillis,
    ) -> Self {
        Self {
            message,
            attempt_count,
            worker_name,
            lease_expires_at,
        }
    }

    pub const fn message(&self) -> &OutboxMessage {
        &self.message
    }

    pub const fn attempt_count(&self) -> u16 {
        self.attempt_count
    }

    pub fn worker_name(&self) -> &str {
        &self.worker_name
    }

    pub const fn lease_expires_at(&self) -> UtcMillis {
        self.lease_expires_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxFailure {
    error_code: String,
    failed_at: UtcMillis,
    next_attempt_at: UtcMillis,
    max_attempts: NonZeroU16,
}

impl OutboxFailure {
    /// 创建一次可观察的失败处理指令。
    ///
    /// # Errors
    ///
    /// 错误码无效或下次重试时间没有严格晚于失败时间时返回校验错误。
    pub fn new(
        error_code: String,
        failed_at: UtcMillis,
        next_attempt_at: UtcMillis,
        max_attempts: NonZeroU16,
    ) -> DomainResult<Self> {
        validate_event_name("error_code", &error_code)?;
        if next_attempt_at <= failed_at {
            return Err(DomainError::Validation {
                field: "next_attempt_at",
                reason: "必须晚于失败时间",
            });
        }

        Ok(Self {
            error_code,
            failed_at,
            next_attempt_at,
            max_attempts,
        })
    }

    pub fn error_code(&self) -> &str {
        &self.error_code
    }

    pub const fn failed_at(&self) -> UtcMillis {
        self.failed_at
    }

    pub const fn next_attempt_at(&self) -> UtcMillis {
        self.next_attempt_at
    }

    pub const fn max_attempts(&self) -> NonZeroU16 {
        self.max_attempts
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxFailureOutcome {
    RetryScheduled { attempt_count: u16 },
    DeadLettered { attempt_count: u16 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutboxBacklog {
    ready: u64,
    scheduled: u64,
    leased: u64,
    dead_lettered: u64,
    oldest_pending_at: Option<UtcMillis>,
}

impl OutboxBacklog {
    pub const fn restore(
        ready: u64,
        scheduled: u64,
        leased: u64,
        dead_lettered: u64,
        oldest_pending_at: Option<UtcMillis>,
    ) -> Self {
        Self {
            ready,
            scheduled,
            leased,
            dead_lettered,
            oldest_pending_at,
        }
    }

    pub const fn ready(self) -> u64 {
        self.ready
    }

    pub const fn scheduled(self) -> u64 {
        self.scheduled
    }

    pub const fn leased(self) -> u64 {
        self.leased
    }

    pub const fn dead_lettered(self) -> u64 {
        self.dead_lettered
    }

    pub const fn oldest_pending_at(self) -> Option<UtcMillis> {
        self.oldest_pending_at
    }
}

pub trait OutboxRepository: Send + Sync {
    fn claim<'a>(
        &'a self,
        claim: &'a OutboxClaim,
    ) -> PortFuture<'a, RepositoryResult<Vec<ClaimedOutboxEvent>>>;

    fn mark_published<'a>(
        &'a self,
        event_id: OutboxEventId,
        worker_name: &'a str,
        published_at: UtcMillis,
    ) -> PortFuture<'a, RepositoryResult<()>>;

    fn record_failure<'a>(
        &'a self,
        event_id: OutboxEventId,
        worker_name: &'a str,
        failure: &'a OutboxFailure,
    ) -> PortFuture<'a, RepositoryResult<OutboxFailureOutcome>>;

    fn backlog(&self, now: UtcMillis) -> PortFuture<'_, RepositoryResult<OutboxBacklog>>;
}

fn validate_event_name(field: &'static str, value: &str) -> DomainResult<()> {
    validate_bounded_text(field, value, MAX_EVENT_NAME_LENGTH)?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte))
    {
        return Err(DomainError::Validation {
            field,
            reason: "只能包含小写 ASCII 字母、数字、点、下划线或连字符",
        });
    }
    Ok(())
}

fn validate_bounded_text(field: &'static str, value: &str, maximum: usize) -> DomainResult<()> {
    if value.is_empty() || value.len() > maximum {
        return Err(DomainError::Validation {
            field,
            reason: "长度超出允许范围",
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use agent_room_domain::{ids::OutboxEventId, time::UtcMillis};
    use serde_json::Map;
    use uuid::Uuid;

    use super::{OutboxClaim, OutboxFailure, OutboxMessage};

    #[test]
    fn 事件名和租约边界在进入适配器前即被拒绝() {
        let time = UtcMillis::new(1_000).expect("测试时间有效");
        let invalid = OutboxMessage::new(
            OutboxEventId::from_uuid(Uuid::now_v7()),
            "Agent".to_owned(),
            Uuid::now_v7(),
            "agent.registered.v1".to_owned(),
            Map::new(),
            time,
        );
        assert!(invalid.is_err());

        let claim = OutboxClaim::new(
            "worker-1".to_owned(),
            NonZeroU16::new(1).expect("批量有效"),
            time,
            time,
        );
        assert!(claim.is_err());

        let failure = OutboxFailure::new(
            "matrix.unavailable".to_owned(),
            time,
            time,
            NonZeroU16::new(3).expect("重试次数有效"),
        );
        assert!(failure.is_err());
    }
}
