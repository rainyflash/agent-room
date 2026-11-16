use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use agent_room_domain::{
    ids::{PrincipalId, RoomCatalogId},
    time::UtcMillis,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{ReminderPlanFailure, ReminderPlanner, ReminderPlannerDependencies};
use crate::{
    persistence::{RepositoryError, RepositoryErrorKind, RepositoryResult},
    ports::{
        MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId,
        MatrixUserId, PortFuture, ReminderCandidate, ReminderMatrixReader, ReminderMessage,
        ReminderMessagePage, ReminderRecipientRepository, ReminderRoom, ReminderRoomKind,
    },
    reminders::{ReminderDecision, ReminderHold, ReminderPlan, SETTLE_MS},
};

const NOW: i64 = 1_790_000_000_000;
const HOUR: i64 = 60 * 60 * 1_000;
const READER: &str = "@reader:matrix.agent-room.localhost";
const OTHER: &str = "@other:matrix.agent-room.localhost";
const BUSY: &str = "!busy:matrix.agent-room.localhost";
const QUIET: &str = "!quiet:matrix.agent-room.localhost";
const LOCKED: &str = "!locked:matrix.agent-room.localhost";

fn at(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}

/// 假的库：他在的房间，`unavailable` 时一律读不了。
struct Recipients {
    rooms: Vec<ReminderRoom>,
    unavailable: bool,
}

impl ReminderRecipientRepository for Recipients {
    fn reminder_candidates(
        &self,
        _after: Option<PrincipalId>,
        _limit: u16,
    ) -> PortFuture<'_, RepositoryResult<Vec<ReminderCandidate>>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn reminder_rooms(
        &self,
        _principal_id: PrincipalId,
    ) -> PortFuture<'_, RepositoryResult<Vec<ReminderRoom>>> {
        let result = if self.unavailable {
            Err(RepositoryError::new(
                "test.reminders",
                RepositoryErrorKind::Unavailable,
            ))
        } else {
            Ok(self.rooms.clone())
        };
        Box::pin(async move { result })
    }
}

/// 假的聊天服务器：他的个人工作区，每间房里的消息；`locked` 的房间读不了。记下问了哪几间、从哪一刻。
struct Matrix {
    workspace: Option<Value>,
    workspace_unavailable: bool,
    messages: HashMap<String, Vec<ReminderMessage>>,
    asked: Mutex<Vec<(String, i64)>>,
}

impl ReminderMatrixReader for Matrix {
    fn personal_workspace<'a>(
        &'a self,
        _user: &'a MatrixUserId,
    ) -> PortFuture<'a, MatrixResult<Option<Value>>> {
        let result = if self.workspace_unavailable {
            Err(MatrixFailure::new(
                MatrixOperation::ReadAccountData,
                MatrixFailureKind::DependencyUnavailable,
            ))
        } else {
            Ok(self.workspace.clone())
        };
        Box::pin(async move { result })
    }

    fn messages_since<'a>(
        &'a self,
        _user: &'a MatrixUserId,
        room: &'a MatrixRoomId,
        _end_to_end_encrypted: bool,
        since: UtcMillis,
    ) -> PortFuture<'a, MatrixResult<ReminderMessagePage>> {
        self.asked
            .lock()
            .unwrap()
            .push((room.as_str().to_owned(), since.value()));
        let result = if room.as_str() == LOCKED {
            Err(MatrixFailure::new(
                MatrixOperation::Backfill,
                MatrixFailureKind::Forbidden,
            ))
        } else {
            Ok(ReminderMessagePage {
                messages: self
                    .messages
                    .get(room.as_str())
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|message| message.sent_at >= since)
                    .collect(),
                truncated: false,
            })
        };
        Box::pin(async move { result })
    }
}

fn room(id: &str) -> ReminderRoom {
    ReminderRoom {
        catalog_id: RoomCatalogId::from_uuid(Uuid::now_v7()),
        matrix_room_id: MatrixRoomId::new(id).unwrap(),
        kind: ReminderRoomKind::Room,
        end_to_end_encrypted: true,
        joined_at: at(NOW - 10 * HOUR),
    }
}

fn said(id: &str, sent_at: i64) -> ReminderMessage {
    ReminderMessage {
        event_id: id.to_owned(),
        sender: MatrixUserId::new(OTHER).unwrap(),
        sent_at: at(sent_at),
        mentions_reader: false,
    }
}

fn candidate(last_email_at: Option<i64>) -> ReminderCandidate {
    ReminderCandidate {
        principal_id: PrincipalId::from_uuid(Uuid::now_v7()),
        matrix_user_id: MatrixUserId::new(READER).unwrap(),
        last_email_at: last_email_at.map(at),
        covered_until: None,
    }
}

fn workspace(entries: &[Value]) -> Value {
    json!({ "schema": 1, "entries": entries })
}

fn entry(kind: &str, id: &str, value: Value) -> Value {
    let mut entry = json!({ "kind": kind, "id": id, "clock": 1, "writerId": "web" });
    entry["value"] = value;
    entry
}

struct World {
    planner: ReminderPlanner,
    matrix: Arc<Matrix>,
    rooms: Vec<ReminderRoom>,
}

fn world(workspace: Option<Value>, rooms: Vec<ReminderRoom>) -> World {
    let matrix = Arc::new(Matrix {
        workspace,
        workspace_unavailable: false,
        messages: HashMap::from([
            (
                BUSY.to_owned(),
                vec![said("$b3", NOW - 2 * HOUR), said("$b2", NOW - 3 * HOUR)],
            ),
            (QUIET.to_owned(), vec![said("$q1", NOW - 5 * HOUR)]),
        ]),
        asked: Mutex::new(Vec::new()),
    });
    let planner = ReminderPlanner::new(ReminderPlannerDependencies {
        recipients: Arc::new(Recipients {
            rooms: rooms.clone(),
            unavailable: false,
        }),
        matrix: matrix.clone(),
    });
    World {
        planner,
        matrix,
        rooms,
    }
}

#[tokio::test]
async fn 免打扰的房间不读_读不了的房间跳过_别的照读到的位置算() {
    let document = workspace(&[
        entry("mute", QUIET, json!(true)),
        entry(
            "read",
            BUSY,
            json!({ "eventId": "$b2", "timestamp": NOW - 3 * HOUR }),
        ),
    ]);
    let world = world(Some(document), vec![room(BUSY), room(QUIET), room(LOCKED)]);

    let plan = world.planner.plan(&candidate(None), at(NOW)).await.unwrap();

    let ReminderPlan {
        decision: ReminderDecision::Send { rooms, .. },
        unreadable_rooms,
    } = plan
    else {
        panic!("该发：{plan:?}");
    };
    assert_eq!(unreadable_rooms, 1);
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0].catalog_id, world.rooms[0].catalog_id);
    assert_eq!(rooms[0].messages, 1, "读到的那一条不算");
    let asked = world.matrix.asked.lock().unwrap().clone();
    assert_eq!(
        asked,
        vec![
            (BUSY.to_owned(), NOW - 3 * HOUR),
            (LOCKED.to_owned(), NOW - 10 * HOUR)
        ]
    );
}

#[tokio::test]
async fn 不满二十四小时不读聊天服务器_暂停中不列房间() {
    let world = world(None, vec![room(BUSY)]);
    let plan = world
        .planner
        .plan(&candidate(Some(NOW - HOUR)), at(NOW))
        .await
        .unwrap();
    assert_eq!(plan.decision, ReminderDecision::Hold(ReminderHold::TooSoon));
    assert!(world.matrix.asked.lock().unwrap().is_empty());

    let paused = self::world(
        Some(workspace(&[entry("dnd", "global", Value::Null)])),
        vec![room(BUSY)],
    );
    let plan = paused
        .planner
        .plan(&candidate(None), at(NOW))
        .await
        .unwrap();
    assert_eq!(plan.decision, ReminderDecision::Hold(ReminderHold::Paused));
    assert!(paused.matrix.asked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn 最早一条还不满一小时就先不发() {
    let world = world(None, vec![room(BUSY)]);
    let early = at(NOW - 3 * HOUR + SETTLE_MS - 1);
    let plan = world.planner.plan(&candidate(None), early).await.unwrap();
    assert_eq!(
        plan.decision,
        ReminderDecision::Hold(ReminderHold::TooFresh)
    );
}

#[tokio::test]
async fn 个人工作区或者房间列不出来就算这个账户这一轮失败() {
    let unreadable = ReminderPlanner::new(ReminderPlannerDependencies {
        recipients: Arc::new(Recipients {
            rooms: vec![room(BUSY)],
            unavailable: false,
        }),
        matrix: Arc::new(Matrix {
            workspace: None,
            workspace_unavailable: true,
            messages: HashMap::new(),
            asked: Mutex::new(Vec::new()),
        }),
    });
    assert_eq!(
        unreadable.plan(&candidate(None), at(NOW)).await,
        Err(ReminderPlanFailure::Workspace)
    );

    let no_rooms = ReminderPlanner::new(ReminderPlannerDependencies {
        recipients: Arc::new(Recipients {
            rooms: Vec::new(),
            unavailable: true,
        }),
        matrix: world(None, Vec::new()).matrix,
    });
    assert_eq!(
        no_rooms.plan(&candidate(None), at(NOW)).await,
        Err(ReminderPlanFailure::Repository)
    );
}
