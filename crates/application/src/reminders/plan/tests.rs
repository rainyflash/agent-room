use agent_room_domain::{ids::RoomCatalogId, time::UtcMillis};
use uuid::Uuid;

use super::{
    LOOKBACK_MS, MINIMUM_GAP_MS, ReminderDecision, ReminderHold, SETTLE_MS, Standing, Start,
    decide, tally,
};
use crate::{
    ports::{
        MatrixRoomId, MatrixUserId, ReminderMessage, ReminderMessagePage, ReminderRoom,
        ReminderRoomKind,
    },
    reminders::workspace::{Pause, PersonalWorkspace, ReadPosition},
};

const NOW: i64 = 1_790_000_000_000;
const HOUR: i64 = 60 * 60 * 1_000;
const ROOM: &str = "!room:matrix.agent-room.localhost";
const READER: &str = "@reader:matrix.agent-room.localhost";
const OTHER: &str = "@other:matrix.agent-room.localhost";

fn at(value: i64) -> UtcMillis {
    UtcMillis::new(value).unwrap()
}

fn room(encrypted: bool, joined_at: i64) -> ReminderRoom {
    ReminderRoom {
        catalog_id: RoomCatalogId::from_uuid(Uuid::now_v7()),
        matrix_room_id: MatrixRoomId::new(ROOM).unwrap(),
        kind: ReminderRoomKind::Room,
        end_to_end_encrypted: encrypted,
        joined_at: at(joined_at),
    }
}

fn message(id: &str, sender: &str, sent_at: i64, mentions_reader: bool) -> ReminderMessage {
    ReminderMessage {
        event_id: id.to_owned(),
        sender: MatrixUserId::new(sender).unwrap(),
        sent_at: at(sent_at),
        mentions_reader,
    }
}

fn page(messages: Vec<ReminderMessage>) -> ReminderMessagePage {
    ReminderMessagePage {
        messages,
        truncated: false,
    }
}

fn reader() -> MatrixUserId {
    MatrixUserId::new(READER).unwrap()
}

fn standing(workspace: &PersonalWorkspace) -> Standing<'_> {
    Standing {
        now: at(NOW),
        covered_until: None,
        workspace,
    }
}

#[test]
fn 起点取最晚的那个_读到的位置在同一毫秒里只放过排在后面的() {
    let mut workspace = PersonalWorkspace::empty();
    workspace.read.insert(
        ROOM.to_owned(),
        ReadPosition {
            at: NOW - 3 * HOUR,
            event_id: "$b".to_owned(),
        },
    );
    let room = room(true, NOW - 10 * HOUR);

    let start = standing(&workspace).room_start(&room);

    assert_eq!(start.at, NOW - 3 * HOUR);
    assert!(!start.admits(NOW - 3 * HOUR, "$a"));
    assert!(!start.admits(NOW - 3 * HOUR, "$b"));
    assert!(start.admits(NOW - 3 * HOUR, "$c"));
    assert!(start.admits(NOW - 3 * HOUR + 1, "$a"));

    // 上一封说到的那一刻更晚：那一毫秒整个算过了。
    let later = Standing {
        covered_until: Some(at(NOW - 3 * HOUR)),
        ..standing(&workspace)
    }
    .room_start(&room);
    assert!(!later.admits(NOW - 3 * HOUR, "$c"));
}

#[test]
fn 没读过的房间从进来的时候算_最多往回七天_暂停结束以后才算() {
    let workspace = PersonalWorkspace::empty();
    assert_eq!(
        standing(&workspace)
            .room_start(&room(true, NOW - 2 * HOUR))
            .at,
        NOW - 2 * HOUR
    );
    assert_eq!(
        standing(&workspace).room_start(&room(true, 0)).at,
        NOW - LOOKBACK_MS
    );

    let mut paused = PersonalWorkspace::empty();
    paused.pause = Pause::Until(NOW - HOUR);
    let state = standing(&paused);
    assert!(!state.paused());
    assert_eq!(state.room_start(&room(true, 0)).at, NOW - HOUR);
}

#[test]
fn 暂停中和不满二十四小时都不发() {
    let mut until = PersonalWorkspace::empty();
    until.pause = Pause::Until(NOW + 1);
    assert!(standing(&until).paused());
    let mut forever = PersonalWorkspace::empty();
    forever.pause = Pause::Indefinite;
    assert!(standing(&forever).paused());

    assert!(Standing::too_soon(
        Some(at(NOW - MINIMUM_GAP_MS + 1)),
        at(NOW)
    ));
    assert!(!Standing::too_soon(Some(at(NOW - MINIMUM_GAP_MS)), at(NOW)));
    assert!(!Standing::too_soon(None, at(NOW)));
}

#[test]
fn 加密的房间每条都算_自己发的不算() {
    let room = room(true, 0);
    let start = Start {
        at: NOW - 5 * HOUR,
        after_event: None,
    };
    let counted = tally(
        &room,
        &start,
        &page(vec![
            message("$3", OTHER, NOW - HOUR, false),
            message("$2", READER, NOW - 2 * HOUR, false),
            message("$1", OTHER, NOW - 4 * HOUR, false),
            message("$0", OTHER, NOW - 6 * HOUR, false),
        ]),
        &reader(),
    )
    .expect("有新消息");

    assert_eq!(counted.messages, 2);
    assert_eq!(counted.earliest_at, NOW - 4 * HOUR);
    assert!(!counted.mentions_only);
    assert!(!counted.more);
}

#[test]
fn 不加密的房间只算点了他名字的() {
    let room = room(false, 0);
    let start = Start {
        at: NOW - 5 * HOUR,
        after_event: None,
    };
    let only_chatter = page(vec![message("$1", OTHER, NOW - HOUR, false)]);
    assert_eq!(tally(&room, &start, &only_chatter, &reader()), None);

    let mut mentioned = page(vec![
        message("$2", OTHER, NOW - HOUR, true),
        message("$1", OTHER, NOW - 2 * HOUR, false),
    ]);
    mentioned.truncated = true;
    let counted = tally(&room, &start, &mentioned, &reader()).expect("点了他");
    assert_eq!(counted.messages, 1);
    assert!(counted.mentions_only);
    assert!(counted.more);
}

#[test]
fn 最早一条满一小时才发_房间按新消息多少排() {
    let tally_of = |messages, earliest_at| super::ReminderRoomTally {
        catalog_id: RoomCatalogId::from_uuid(Uuid::now_v7()),
        kind: ReminderRoomKind::Room,
        mentions_only: false,
        messages,
        more: false,
        earliest_at,
    };
    assert_eq!(
        decide(at(NOW), Vec::new()),
        ReminderDecision::Hold(ReminderHold::NothingNew)
    );
    assert_eq!(
        decide(at(NOW), vec![tally_of(3, NOW - SETTLE_MS + 1)]),
        ReminderDecision::Hold(ReminderHold::TooFresh)
    );

    let few = tally_of(2, NOW - SETTLE_MS);
    let many = tally_of(9, NOW - 1);
    let ReminderDecision::Send {
        rooms,
        covered_until,
    } = decide(at(NOW), vec![few.clone(), many.clone()])
    else {
        panic!("该发");
    };
    assert_eq!(rooms, vec![many, few]);
    assert_eq!(covered_until, at(NOW));
}
