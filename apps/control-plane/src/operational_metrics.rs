use std::{error::Error, fmt, time::Duration};

use sqlx::{FromRow, PgPool};
use tokio::{sync::oneshot, task::JoinHandle, time::MissedTickBehavior};

use crate::telemetry_metrics::TelemetryMetrics;

const METRIC_NAMES: [&str; 9] = [
    "database_pool_connections",
    "database_pool_idle_connections",
    "outbox_pending",
    "outbox_dead_lettered",
    "outbox_oldest_pending",
    "projection_unhealthy",
    "projection_oldest_update",
    "content_reclaimable",
    "account_deletion_pending",
];

/// 以固定指标名采样数据库运行状态，避免把租户、身份或对象标识写进标签。
pub(crate) struct OperationalMetricsRuntime {
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl OperationalMetricsRuntime {
    /// 启动与控制平面同生命周期的低基数采样器。
    ///
    /// # Errors
    ///
    /// 周期为零时拒绝启动，防止错误配置造成数据库忙循环。
    pub(crate) fn start(
        pool: PgPool,
        metrics: TelemetryMetrics,
        interval: Duration,
    ) -> Result<Self, OperationalMetricsError> {
        if interval.is_zero() {
            return Err(OperationalMetricsError::ZeroInterval);
        }
        let (stop, stop_requested) = oneshot::channel();
        let task = tokio::spawn(run_sampler(pool, metrics, interval, stop_requested));
        Ok(Self {
            stop: Some(stop),
            task,
        })
    }

    pub(crate) async fn shutdown(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Err(error) = self.task.await {
            tracing::error!(
                code = "observability.operational_sampler_join_failed",
                cancelled = error.is_cancelled(),
                panic = error.is_panic(),
                "运行指标采样器异常结束"
            );
        }
    }
}

#[derive(Debug, FromRow)]
struct DatabaseSnapshot {
    outbox_pending: i64,
    outbox_dead_lettered: i64,
    outbox_oldest_pending_seconds: f64,
    projection_unhealthy: i64,
    projection_oldest_update_seconds: f64,
    content_reclaimable: i64,
    account_deletion_pending: i64,
}

async fn run_sampler(
    pool: PgPool,
    metrics: TelemetryMetrics,
    interval: Duration,
    mut stop_requested: oneshot::Receiver<()>,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                record_pool_snapshot(&pool, &metrics);
                if sample_database(&pool, &metrics).await.is_err() {
                    metrics.record_sampler_failure();
                    tracing::warn!(
                        code = "observability.operational_sample_failed",
                        "数据库运行指标采样失败"
                    );
                }
            }
            _ = &mut stop_requested => break,
        }
    }
    tracing::info!("运行指标采样器已停止");
}

fn record_pool_snapshot(pool: &PgPool, metrics: &TelemetryMetrics) {
    metrics.record_operational_value(METRIC_NAMES[0], i64::from(pool.size()));
    metrics.record_operational_value(
        METRIC_NAMES[1],
        i64::try_from(pool.num_idle()).unwrap_or(i64::MAX),
    );
}

async fn sample_database(pool: &PgPool, metrics: &TelemetryMetrics) -> Result<(), sqlx::Error> {
    let snapshot = sqlx::query_as::<_, DatabaseSnapshot>(
        r"
        SELECT
            (SELECT count(*)::bigint
               FROM agent_room.outbox_event
              WHERE published_at IS NULL AND dead_lettered_at IS NULL) AS outbox_pending,
            (SELECT count(*)::bigint
               FROM agent_room.outbox_event
              WHERE dead_lettered_at IS NOT NULL) AS outbox_dead_lettered,
            COALESCE((SELECT max(EXTRACT(EPOCH FROM (clock_timestamp() - occurred_at)))::double precision
                        FROM agent_room.outbox_event
                       WHERE published_at IS NULL AND dead_lettered_at IS NULL), 0.0)
                AS outbox_oldest_pending_seconds,
            (SELECT count(*)::bigint
               FROM agent_room.matrix_projection_cursor
              WHERE health_state <> 'healthy') AS projection_unhealthy,
            COALESCE((SELECT max(EXTRACT(EPOCH FROM (clock_timestamp() - updated_at)))::double precision
                        FROM agent_room.matrix_projection_cursor), 0.0)
                AS projection_oldest_update_seconds,
            (SELECT count(*)::bigint
               FROM agent_room.content_object
              WHERE lifecycle_state IN ('orphaned', 'redacted', 'expired')) AS content_reclaimable,
            (SELECT count(*)::bigint
               FROM agent_room.account_deletion_job
              WHERE stage <> 'completed') AS account_deletion_pending
        ",
    )
    .fetch_one(pool)
    .await?;

    metrics.record_operational_value(METRIC_NAMES[2], snapshot.outbox_pending);
    metrics.record_operational_value(METRIC_NAMES[3], snapshot.outbox_dead_lettered);
    metrics.record_operational_age(METRIC_NAMES[4], snapshot.outbox_oldest_pending_seconds);
    metrics.record_operational_value(METRIC_NAMES[5], snapshot.projection_unhealthy);
    metrics.record_operational_age(METRIC_NAMES[6], snapshot.projection_oldest_update_seconds);
    metrics.record_operational_value(METRIC_NAMES[7], snapshot.content_reclaimable);
    metrics.record_operational_value(METRIC_NAMES[8], snapshot.account_deletion_pending);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationalMetricsError {
    ZeroInterval,
}

impl fmt::Display for OperationalMetricsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroInterval => formatter.write_str("运行指标采样周期必须大于零"),
        }
    }
}

impl Error for OperationalMetricsError {}

#[cfg(test)]
mod tests {
    use super::METRIC_NAMES;

    #[test]
    fn 运行指标名是固定低基数集合() {
        assert_eq!(METRIC_NAMES.len(), 9);
        for name in METRIC_NAMES {
            assert!(name.is_ascii());
            assert!(!name.contains(['/', '\\', '@', ':']));
        }
    }
}
