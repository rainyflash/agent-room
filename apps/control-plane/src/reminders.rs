//! 提醒的每小时一轮（`specs/redesign/reminders.md` 的 4R-4c）。现在是演练：只算、不发信、不写库，
//! 每轮一行日志和指标 `agent_room.reminder.planned{outcome}`，只有个数。
//!
//! 启动后先等一阵再算第一轮：发版时控制面会重启，别一起来就替每个人把房间读一遍。与 HTTP 服务共同
//! 存活、共同关闭；正在算的那一轮在关闭时直接放下（只读，不留半截）。

use std::{sync::Arc, time::Duration};

use agent_room_application::reminders::{ReminderDryRun, ReminderRoundReport};
use tokio::{
    sync::oneshot,
    task::JoinHandle,
    time::{Instant, MissedTickBehavior, interval_at},
};

use crate::telemetry_metrics::TelemetryMetrics;

const INTERVAL: Duration = Duration::from_hours(1);
const FIRST_ROUND_DELAY: Duration = Duration::from_mins(10);

pub(crate) struct ReminderDryRunWorker {
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl ReminderDryRunWorker {
    pub(crate) fn start(dry_run: Arc<ReminderDryRun>, metrics: TelemetryMetrics) -> Self {
        Self::start_every(dry_run, metrics, FIRST_ROUND_DELAY, INTERVAL)
    }

    fn start_every(
        dry_run: Arc<ReminderDryRun>,
        metrics: TelemetryMetrics,
        first: Duration,
        period: Duration,
    ) -> Self {
        let (stop, stop_requested) = oneshot::channel();
        let task = tokio::spawn(run_worker(dry_run, metrics, first, period, stop_requested));
        Self {
            stop: Some(stop),
            task,
        }
    }

    pub(crate) async fn shutdown(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Err(error) = self.task.await {
            tracing::error!(
                code = "reminder.dry_run_join_failed",
                cancelled = error.is_cancelled(),
                panic = error.is_panic(),
                "提醒演练的定时任务异常结束"
            );
        }
    }
}

async fn run_worker(
    dry_run: Arc<ReminderDryRun>,
    metrics: TelemetryMetrics,
    first: Duration,
    period: Duration,
    mut stop_requested: oneshot::Receiver<()>,
) {
    let mut ticks = interval_at(Instant::now() + first, period);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = ticks.tick() => tokio::select! {
                report = dry_run.round() => {
                    record(&metrics, &report);
                    log(&report);
                }
                _ = &mut stop_requested => break,
            },
            _ = &mut stop_requested => break,
        }
    }
    tracing::info!("提醒演练的定时任务已停止");
}

/// 每个账户一个去向；每种都加一次（没有的加 0），图上每种都有一条线。
fn record(metrics: &TelemetryMetrics, report: &ReminderRoundReport) {
    for (outcome, count) in outcomes(report) {
        metrics.record_reminder_planned(outcome, u64::from(count));
    }
}

fn outcomes(report: &ReminderRoundReport) -> [(&'static str, u32); 6] {
    [
        ("send", report.send),
        ("paused", report.paused),
        ("too_soon", report.too_soon),
        ("nothing_new", report.nothing_new),
        ("too_fresh", report.too_fresh),
        ("failed", report.failed),
    ]
}

fn log(report: &ReminderRoundReport) {
    if report.interrupted {
        tracing::warn!(
            code = "reminder.round_interrupted",
            accounts = report.accounts,
            send = report.send,
            "列不出要提醒的账户，这一轮提醒演练没算完，一小时后再来"
        );
        return;
    }
    tracing::info!(
        accounts = report.accounts,
        send = report.send,
        rooms = report.rooms,
        paused = report.paused,
        too_soon = report.too_soon,
        nothing_new = report.nothing_new,
        too_fresh = report.too_fresh,
        unreadable_rooms = report.unreadable_rooms,
        failed = report.failed,
        "算了一轮提醒（演练，不发信）"
    );
}

#[cfg(test)]
mod real_dependency_tests;

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use agent_room_application::{
        persistence::RepositoryResult,
        ports::{
            Clock, MatrixResult, MatrixRoomId, MatrixUserId, PortFuture, ReminderCandidate,
            ReminderMatrixReader, ReminderMessagePage, ReminderRecipientRepository, ReminderRoom,
        },
        reminders::{
            ReminderDryRun, ReminderPlanner, ReminderPlannerDependencies, ReminderRoundReport,
        },
    };
    use agent_room_domain::{ids::PrincipalId, time::UtcMillis};
    use serde_json::Value;
    use tokio::sync::Notify;

    use super::{ReminderDryRunWorker, outcomes};
    use crate::telemetry_metrics::TelemetryMetrics;

    /// 没有候选的库；每列一次就叫一声。
    #[derive(Default)]
    struct Empty {
        listed: Notify,
    }

    impl ReminderRecipientRepository for Empty {
        fn reminder_candidates(
            &self,
            _after: Option<PrincipalId>,
            _limit: u16,
        ) -> PortFuture<'_, RepositoryResult<Vec<ReminderCandidate>>> {
            self.listed.notify_waiters();
            Box::pin(async { Ok(Vec::new()) })
        }

        fn reminder_rooms(
            &self,
            _principal_id: PrincipalId,
        ) -> PortFuture<'_, RepositoryResult<Vec<ReminderRoom>>> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    struct Silent;

    impl ReminderMatrixReader for Silent {
        fn personal_workspace<'a>(
            &'a self,
            _user: &'a MatrixUserId,
        ) -> PortFuture<'a, MatrixResult<Option<Value>>> {
            Box::pin(async { Ok(None) })
        }

        fn messages_since<'a>(
            &'a self,
            _user: &'a MatrixUserId,
            _room: &'a MatrixRoomId,
            _end_to_end_encrypted: bool,
            _since: UtcMillis,
        ) -> PortFuture<'a, MatrixResult<ReminderMessagePage>> {
            Box::pin(async {
                Ok(ReminderMessagePage {
                    messages: Vec::new(),
                    truncated: false,
                })
            })
        }
    }

    struct Fixed;

    impl Clock for Fixed {
        fn now(&self) -> UtcMillis {
            UtcMillis::new(1_790_000_000_000).unwrap()
        }
    }

    #[tokio::test]
    async fn 等过第一轮的延迟才算_关闭时停下() {
        let recipients = Arc::new(Empty::default());
        let planner = ReminderPlanner::new(ReminderPlannerDependencies {
            recipients: recipients.clone(),
            matrix: Arc::new(Silent),
        });
        let listed = recipients.listed.notified();
        let worker = ReminderDryRunWorker::start_every(
            Arc::new(ReminderDryRun::new(planner, Arc::new(Fixed))),
            TelemetryMetrics::new(),
            Duration::from_millis(20),
            Duration::from_hours(1),
        );
        tokio::time::timeout(Duration::from_secs(5), listed)
            .await
            .expect("等过延迟以后算了一轮");
        tokio::time::timeout(Duration::from_secs(5), worker.shutdown())
            .await
            .expect("关得掉");
    }

    #[test]
    fn 每个账户只落在一种去向里() {
        let report = ReminderRoundReport {
            accounts: 21,
            send: 1,
            rooms: 9,
            paused: 2,
            too_soon: 3,
            nothing_new: 4,
            too_fresh: 5,
            unreadable_rooms: 7,
            failed: 6,
            interrupted: false,
        };
        let total: u32 = outcomes(&report).iter().map(|(_, count)| count).sum();
        assert_eq!(total, report.accounts);
    }
}
