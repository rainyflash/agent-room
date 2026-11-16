use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicI64, Ordering},
};

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    time::UtcMillis,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{ROUND_BATCH, ReminderDryRun, ReminderRoundReport};
use crate::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        Clock, MatrixResult, MatrixRoomId, MatrixUserId, PortFuture, ReminderCandidate,
        ReminderMatrixReader, ReminderMessage, ReminderMessagePage, ReminderRecipientRepository,
        ReminderRoom, ReminderRoomKind,
    },
    reminders::{ReminderPlanner, ReminderPlannerDependencies},
};

const NOW: i64 = 1_790_000_000_000;
const HOUR: i64 = 60 * 60 * 1_000;
const OTHER: &str = "@other:matrix.agent-room.localhost";
const ROOM: &str = "!room:matrix.agent-room.localhost";
const PAUSED: &str = "@paused:matrix.agent-room.localhost";

fn at(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}

struct Steady(AtomicI64);

impl Clock for Steady {
    fn now(&self) -> UtcMillis {
        at(self.0.load(Ordering::SeqCst))
    }
}

/// 假的库：按账户 ID 排的候选，照 `after`、`limit` 一页页给；记下每次从哪往后翻。每人一间房。
struct Recipients {
    candidates: Vec<ReminderCandidate>,
    pages: Mutex<Vec<Option<PrincipalId>>>,
    listing_fails: AtomicBool,
    rooms_fail_for: Option<PrincipalId>,
}

impl ReminderRecipientRepository for Recipients {
    fn reminder_candidates(
        &self,
        after: Option<PrincipalId>,
        limit: u16,
    ) -> PortFuture<'_, RepositoryResult<Vec<ReminderCandidate>>> {
        self.pages.lock().unwrap().push(after);
        let result = if self.listing_fails.load(Ordering::SeqCst) {
            Err(RepositoryError::new(
                "test.reminders",
                RepositoryErrorKind::Unavailable,
            ))
        } else {
            Ok(self
                .candidates
                .iter()
                .filter(|candidate| after.is_none_or(|after| candidate.principal_id > after))
                .take(usize::from(limit))
                .cloned()
                .collect())
        };
        Box::pin(async move { result })
    }

    fn reminder_rooms(
        &self,
        principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<ReminderRoom>>> {
        let result = if self.rooms_fail_for == Some(principal_id) {
            Err(RepositoryError::new(
                "test.reminders",
                RepositoryErrorKind::Unavailable,
            ))
        } else {
            Ok(vec![ReminderRoom {
                catalog_id: RoomCatalogId::from_uuid(Uuid::now_v7()),
                matrix_room_id: MatrixRoomId::new(ROOM).unwrap(),
                kind: ReminderRoomKind::Room,
                end_to_end_encrypted: true,
                joined_at: at(NOW - 48 * HOUR),
            }])
        };
        Box::pin(async move { result })
    }
}

/// 假的聊天服务器：房间里两小时前有人说了一句；`PAUSED` 那个人一直暂停提醒。
struct Matrix;

impl ReminderMatrixReader for Matrix {
    fn personal_workspace<'a>(
        &'a self,
        user: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<Option<Value>>> {
        let document = (user.as_str() == PAUSED).then(|| {
            json!({ "schema": 1, "entries": [{
                "kind": "dnd", "id": "global", "value": null, "clock": 1, "writerId": "web",
            }] })
        });
        Box::pin(async move { Ok(document) })
    }

    fn messages_since<'a>(
        &'a self,
        _user: &'a MatrixUserId,
        _room: &'a MatrixRoomId,
        _end_to_end_encrypted: bool,
        since: UtcMillis,
    ) -> PortFuture<'a, MatrixResult<ReminderMessagePage>> {
        let said = ReminderMessage {
            event_id: "$said".to_owned(),
            sender: MatrixUserId::new(OTHER).unwrap(),
            sent_at: at(NOW - 2 * HOUR),
            mentions_reader: false,
        };
        let messages = if said.sent_at >= since {
            vec![said]
        } else {
            Vec::new()
        };
        Box::pin(async move {
            Ok(ReminderMessagePage {
                messages,
                truncated: false,
            })
        })
    }
}

fn people(count: usize) -> Vec<ReminderCandidate> {
    let mut people: Vec<ReminderCandidate> = (0..count)
        .map(|index| ReminderCandidate {
            principal_id: PrincipalId::from_uuid(Uuid::now_v7()),
            matrix_user_id: MatrixUserId::new(format!("@p{index}:matrix.agent-room.localhost"))
                .unwrap(),
            last_email_at: None,
            covered_until: None,
        })
        .collect();
    people.sort_by_key(|candidate| candidate.principal_id);
    people
}

struct World {
    dry_run: ReminderDryRun,
    clock: Arc<Steady>,
    recipients: Arc<Recipients>,
}

fn world(candidates: Vec<ReminderCandidate>, rooms_fail_for: Option<PrincipalId>) -> World {
    let clock = Arc::new(Steady(AtomicI64::new(NOW)));
    let recipients = Arc::new(Recipients {
        candidates,
        pages: Mutex::new(Vec::new()),
        listing_fails: AtomicBool::new(false),
        rooms_fail_for,
    });
    let planner = ReminderPlanner::new(ReminderPlannerDependencies {
        recipients: recipients.clone(),
        matrix: Arc::new(Matrix),
    });
    World {
        dry_run: ReminderDryRun::new(planner, clock.clone()),
        clock,
        recipients,
    }
}

#[tokio::test]
async fn 按批往后翻_每个账户都算一遍() {
    let candidates = people(usize::from(ROUND_BATCH) + 5);
    let fiftieth = candidates[usize::from(ROUND_BATCH) - 1].principal_id;
    let world = world(candidates, None);

    let report = world.dry_run.round().await;

    assert_eq!(
        report,
        ReminderRoundReport {
            accounts: 55,
            send: 55,
            rooms: 55,
            ..ReminderRoundReport::default()
        }
    );
    assert_eq!(
        *world.recipients.pages.lock().unwrap(),
        [None, Some(fiftieth)],
        "满一批才往后翻"
    );
}

#[tokio::test]
async fn 演练里发过的记在内存里_一天内不再发_之后从说到的那一刻算起() {
    let world = world(people(3), None);
    assert_eq!(world.dry_run.round().await.send, 3);

    world.clock.0.store(NOW + HOUR, Ordering::SeqCst);
    let next = world.dry_run.round().await;
    assert_eq!((next.send, next.too_soon), (0, 3), "一天最多一封");

    world.clock.0.store(NOW + 25 * HOUR, Ordering::SeqCst);
    let later = world.dry_run.round().await;
    assert_eq!(
        (later.send, later.nothing_new),
        (0, 3),
        "上一封说过的那句不再算"
    );
}

#[tokio::test]
async fn 没发的按原因记_读不了的账户记失败_别人照算() {
    let mut candidates = people(3);
    candidates[0].matrix_user_id = MatrixUserId::new(PAUSED).unwrap();
    let broken = candidates[1].principal_id;
    let world = world(candidates, Some(broken));

    let report = world.dry_run.round().await;

    assert_eq!(
        report,
        ReminderRoundReport {
            accounts: 3,
            send: 1,
            rooms: 1,
            paused: 1,
            failed: 1,
            ..ReminderRoundReport::default()
        }
    );
}

#[tokio::test]
async fn 列不出候选就停下_说这一轮没走完() {
    let world = world(people(2), None);
    world.recipients.listing_fails.store(true, Ordering::SeqCst);

    let report = world.dry_run.round().await;

    assert!(report.interrupted);
    assert_eq!(report.accounts, 0);
}
