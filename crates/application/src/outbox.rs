use std::num::NonZeroU16;

use agent_room_domain::{
    DomainError,
    time::{DurationMillis, UtcMillis},
};
use thiserror::Error;

use crate::{
    persistence::RepositoryError,
    ports::{
        ClaimedOutboxEvent, Clock, OutboxClaim, OutboxFailure, OutboxFailureOutcome,
        OutboxPublisher, OutboxRepository, PublishFailure, PublishFailureKind,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExponentialBackoff {
    base_delay: DurationMillis,
    maximum_delay: DurationMillis,
    maximum_attempts: NonZeroU16,
}

impl ExponentialBackoff {
    /// 创建有上限的指数退避策略。
    ///
    /// # Errors
    ///
    /// 最大延迟小于基础延迟或尝试次数超过数据库上限时返回校验错误。
    pub fn new(
        base_delay: DurationMillis,
        maximum_delay: DurationMillis,
        maximum_attempts: NonZeroU16,
    ) -> Result<Self, DomainError> {
        if maximum_delay < base_delay {
            return Err(DomainError::Validation {
                field: "maximum_delay",
                reason: "不能小于基础退避时间",
            });
        }
        if maximum_attempts.get() > 100 {
            return Err(DomainError::Validation {
                field: "maximum_attempts",
                reason: "不能超过 100",
            });
        }
        Ok(Self {
            base_delay,
            maximum_delay,
            maximum_attempts,
        })
    }

    fn failure(
        self,
        event: &ClaimedOutboxEvent,
        failure: &PublishFailure,
        failed_at: UtcMillis,
    ) -> Result<OutboxFailure, DomainError> {
        let attempt_number = event.attempt_count().saturating_add(1);
        let exponent = u32::from(attempt_number.saturating_sub(1)).min(63);
        let factor = 1_u128 << exponent;
        let delay = u128::from(self.base_delay.value())
            .saturating_mul(factor)
            .min(u128::from(self.maximum_delay.value()));
        let delay = u64::try_from(delay).map_err(|_| DomainError::TimeOverflow)?;
        let delay = DurationMillis::new(delay)?;
        let next_attempt_at = failed_at.checked_add(delay)?;
        let maximum_attempts = match failure.kind() {
            PublishFailureKind::Transient => self.maximum_attempts,
            PublishFailureKind::Permanent => NonZeroU16::MIN,
        };
        OutboxFailure::new(
            failure.code().to_owned(),
            failed_at,
            next_attempt_at,
            maximum_attempts,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OutboxProcessingReport {
    claimed: u16,
    published: u16,
    retry_scheduled: u16,
    dead_lettered: u16,
}

impl OutboxProcessingReport {
    pub const fn claimed(self) -> u16 {
        self.claimed
    }

    pub const fn published(self) -> u16 {
        self.published
    }

    pub const fn retry_scheduled(self) -> u16 {
        self.retry_scheduled
    }

    pub const fn dead_lettered(self) -> u16 {
        self.dead_lettered
    }
}

#[derive(Debug, Error)]
pub enum OutboxProcessingError {
    #[error("Outbox 仓储操作失败")]
    Repository(#[from] RepositoryError),
    #[error("Outbox 时间或退避计算失败")]
    Policy(#[from] DomainError),
}

pub struct OutboxProcessor<R, P, C> {
    repository: R,
    publisher: P,
    clock: C,
    backoff: ExponentialBackoff,
}

impl<R, P, C> OutboxProcessor<R, P, C>
where
    R: OutboxRepository,
    P: OutboxPublisher,
    C: Clock,
{
    pub const fn new(repository: R, publisher: P, clock: C, backoff: ExponentialBackoff) -> Self {
        Self {
            repository,
            publisher,
            clock,
            backoff,
        }
    }

    /// 领取并处理一个有界批次，不在应用层睡眠或持有数据库事务。
    ///
    /// # Errors
    ///
    /// 仓储不可用、租约确认冲突或退避时间溢出时返回显式错误。
    pub async fn process_once(
        &self,
        claim: &OutboxClaim,
    ) -> Result<OutboxProcessingReport, OutboxProcessingError> {
        let events = self.repository.claim(claim).await?;
        let mut report = OutboxProcessingReport {
            claimed: u16::try_from(events.len()).map_err(|_| DomainError::Validation {
                field: "claimed_events",
                reason: "超过单批上限",
            })?,
            ..OutboxProcessingReport::default()
        };

        for event in events {
            match self.publisher.publish(event.message()).await {
                Ok(()) => {
                    self.repository
                        .mark_published(event.message().id(), event.worker_name(), self.clock.now())
                        .await?;
                    report.published = report.published.saturating_add(1);
                }
                Err(publish_failure) => {
                    let failure =
                        self.backoff
                            .failure(&event, &publish_failure, self.clock.now())?;
                    match self
                        .repository
                        .record_failure(event.message().id(), event.worker_name(), &failure)
                        .await?
                    {
                        OutboxFailureOutcome::RetryScheduled { .. } => {
                            report.retry_scheduled = report.retry_scheduled.saturating_add(1);
                        }
                        OutboxFailureOutcome::DeadLettered { .. } => {
                            report.dead_lettered = report.dead_lettered.saturating_add(1);
                        }
                    }
                }
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, num::NonZeroU16, sync::Mutex};

    use agent_room_domain::{
        ids::OutboxEventId,
        time::{DurationMillis, UtcMillis},
    };
    use serde_json::Map;
    use uuid::Uuid;

    use super::{ExponentialBackoff, OutboxProcessor};
    use crate::{
        persistence::RepositoryResult,
        ports::{
            ClaimedOutboxEvent, Clock, OutboxBacklog, OutboxClaim, OutboxFailure,
            OutboxFailureOutcome, OutboxMessage, OutboxPublisher, OutboxRepository, PortFuture,
            PublishFailure, PublishFailureKind,
        },
    };

    struct FakeRepository {
        claimed: Vec<ClaimedOutboxEvent>,
        completions: Mutex<Vec<OutboxFailureOutcome>>,
        published: Mutex<Vec<OutboxEventId>>,
    }

    impl OutboxRepository for FakeRepository {
        fn claim<'a>(
            &'a self,
            _claim: &'a OutboxClaim,
        ) -> PortFuture<'a, RepositoryResult<Vec<ClaimedOutboxEvent>>> {
            Box::pin(async move { Ok(self.claimed.clone()) })
        }

        fn mark_published<'a>(
            &'a self,
            event_id: OutboxEventId,
            _worker_name: &'a str,
            _published_at: UtcMillis,
        ) -> PortFuture<'a, RepositoryResult<()>> {
            Box::pin(async move {
                self.published
                    .lock()
                    .expect("测试发布记录锁不得中毒")
                    .push(event_id);
                Ok(())
            })
        }

        fn record_failure<'a>(
            &'a self,
            _event_id: OutboxEventId,
            _worker_name: &'a str,
            failure: &'a OutboxFailure,
        ) -> PortFuture<'a, RepositoryResult<OutboxFailureOutcome>> {
            Box::pin(async move {
                let outcome = if failure.max_attempts() == NonZeroU16::MIN {
                    OutboxFailureOutcome::DeadLettered { attempt_count: 1 }
                } else {
                    OutboxFailureOutcome::RetryScheduled { attempt_count: 1 }
                };
                self.completions
                    .lock()
                    .expect("测试失败记录锁不得中毒")
                    .push(outcome);
                Ok(outcome)
            })
        }

        fn backlog(&self, _now: UtcMillis) -> PortFuture<'_, RepositoryResult<OutboxBacklog>> {
            Box::pin(async move { Ok(OutboxBacklog::restore(0, 0, 0, 0, None)) })
        }
    }

    struct FakePublisher {
        outcomes: Mutex<VecDeque<Result<(), PublishFailure>>>,
    }

    impl OutboxPublisher for FakePublisher {
        fn publish<'a>(
            &'a self,
            _message: &'a OutboxMessage,
        ) -> PortFuture<'a, Result<(), PublishFailure>> {
            Box::pin(async move {
                self.outcomes
                    .lock()
                    .expect("测试发布结果锁不得中毒")
                    .pop_front()
                    .expect("每个事件都必须配置发布结果")
            })
        }
    }

    struct StaticClock(UtcMillis);

    impl Clock for StaticClock {
        fn now(&self) -> UtcMillis {
            self.0
        }
    }

    #[tokio::test]
    async fn 单批处理统一收敛成功重试与永久失败() {
        let now = time(2_000);
        let messages = [message(1), message(2), message(3)];
        let repository = FakeRepository {
            claimed: messages
                .into_iter()
                .map(|message| {
                    ClaimedOutboxEvent::restore(message, 0, "worker".to_owned(), time(3_000))
                })
                .collect(),
            completions: Mutex::new(Vec::new()),
            published: Mutex::new(Vec::new()),
        };
        let publisher = FakePublisher {
            outcomes: Mutex::new(VecDeque::from([
                Ok(()),
                Err(PublishFailure::new(
                    "matrix.unavailable".to_owned(),
                    PublishFailureKind::Transient,
                )
                .expect("瞬时错误有效")),
                Err(PublishFailure::new(
                    "matrix.rejected".to_owned(),
                    PublishFailureKind::Permanent,
                )
                .expect("永久错误有效")),
            ])),
        };
        let backoff = ExponentialBackoff::new(
            DurationMillis::new(100).expect("基础退避有效"),
            DurationMillis::new(10_000).expect("最大退避有效"),
            NonZeroU16::new(5).expect("最大尝试数有效"),
        )
        .expect("退避策略有效");
        let processor = OutboxProcessor::new(repository, publisher, StaticClock(now), backoff);
        let claim = OutboxClaim::new(
            "worker".to_owned(),
            NonZeroU16::new(3).expect("批大小有效"),
            time(1_000),
            time(3_000),
        )
        .expect("领取参数有效");

        let report = processor.process_once(&claim).await.expect("批处理应完成");

        assert_eq!(report.claimed(), 3);
        assert_eq!(report.published(), 1);
        assert_eq!(report.retry_scheduled(), 1);
        assert_eq!(report.dead_lettered(), 1);
    }

    fn message(seed: u128) -> OutboxMessage {
        let aggregate_id = Uuid::now_v7();
        OutboxMessage::new(
            OutboxEventId::from_uuid(Uuid::from_u128(
                0x01945c1e_7b5a_7000_8000_000000000000 | seed,
            )),
            "agent".to_owned(),
            aggregate_id,
            "agent.registered.v1".to_owned(),
            Map::new(),
            time(1_000),
        )
        .expect("测试事件有效")
    }

    fn time(value: i64) -> UtcMillis {
        UtcMillis::new(value).expect("测试时间有效")
    }
}
