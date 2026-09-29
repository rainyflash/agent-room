use std::time::{Duration, Instant};

use agent_room_protocol_conformance::generated::{RoomKeyExport, RoomKeysEvent};
use matrix_sdk::{
    deserialized_responses::{TimelineEvent, UnableToDecryptInfo, UnableToDecryptReason},
    ruma::{OwnedDeviceId, OwnedRoomId, OwnedUserId, RoomId, UserId, serde::Raw},
};
use serde_json::json;

use super::{
    KEYS_PER_MESSAGE, MAX_PENDING_REQUESTS, REQUEST_INTERVAL, RequestTarget, RoomKeyRequester,
    requestable, valid_answer,
};
use crate::room_keys::{ROOM_KEYS_EVENT_TYPE, RequestOrigin, SCHEMA_VERSION};

const ROOM: &str = "!private:agent-room.test";
const OTHER_ROOM: &str = "!other:agent-room.test";
const ME: &str = "@_agent_new:agent-room.test";
const SENDER: &str = "@_agent_old:agent-room.test";
const SENDER_DEVICE: &str = "OLDDEVICE";
const SENDER_CURVE: &str = "ccccccccccccccccccccccccccccccccccccccccccc";
const SENDER_ED25519: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

fn session(letter: char) -> String {
    std::iter::once(letter)
        .chain(std::iter::repeat_n('a', 42))
        .collect()
}

fn room() -> OwnedRoomId {
    RoomId::parse(ROOM).expect("房间标识有效")
}

fn target(device: Option<&str>) -> RequestTarget {
    RequestTarget {
        room: room(),
        sender: UserId::parse(SENDER).expect("用户标识有效"),
        device: device.map(OwnedDeviceId::from),
    }
}

fn origin() -> RequestOrigin {
    RequestOrigin {
        user_id: UserId::parse(SENDER).expect("用户标识有效"),
        device_id: OwnedDeviceId::from(SENDER_DEVICE),
        curve25519: SENDER_CURVE.to_owned(),
    }
}

fn answer(request_id: &str, sessions: &[String]) -> RoomKeysEvent {
    RoomKeysEvent {
        created_at: "2026-09-29T06:00:00.000Z".to_owned(),
        event_type: ROOM_KEYS_EVENT_TYPE.to_owned(),
        id: "01990d9e-8400-7000-8000-000000000201".to_owned(),
        keys: sessions
            .iter()
            .map(|session_id| RoomKeyExport {
                session_id: session_id.clone(),
                session_key: "k".repeat(220),
            })
            .collect(),
        request_id: request_id.to_owned(),
        room_id: ROOM.to_owned(),
        schema_version: SCHEMA_VERSION.to_owned(),
        sender_ed25519_key: SENDER_ED25519.to_owned(),
        sender_key: SENDER_CURVE.to_owned(),
        extensions: std::collections::BTreeMap::new(),
    }
}

fn undecryptable(
    sender: &str,
    session_id: &str,
    device: Option<&str>,
    reason: UnableToDecryptReason,
) -> TimelineEvent {
    let mut content = json!({
        "algorithm": "m.megolm.v1.aes-sha2",
        "ciphertext": "opaque",
        "session_id": session_id,
        "sender_key": SENDER_CURVE,
    });
    if let Some(device) = device {
        content["device_id"] = json!(device);
    }
    let raw = Raw::from_json_string(
        json!({
            "type": "m.room.encrypted",
            "event_id": "$encrypted",
            "sender": sender,
            "origin_server_ts": 1,
            "content": content,
        })
        .to_string(),
    )
    .expect("事件可以编码");
    TimelineEvent::from_utd(
        raw,
        UnableToDecryptInfo {
            session_id: Some(session_id.to_owned()),
            reason,
        },
    )
}

#[test]
fn 缺会话或缺消息序号的别人的消息才请求_点名的设备跟着走() {
    let me = UserId::parse(ME).expect("用户标识有效");
    let missing = UnableToDecryptReason::MissingMegolmSession {
        withheld_code: None,
    };

    let (target, session_id) = requestable(
        Some(&me),
        &room(),
        &undecryptable(SENDER, &session('a'), Some(SENDER_DEVICE), missing.clone()),
    )
    .expect("缺会话要请求");
    assert_eq!(target, self::target(Some(SENDER_DEVICE)));
    assert_eq!(session_id, session('a'));

    let (without_device, _) = requestable(
        Some(&me),
        &room(),
        &undecryptable(
            SENDER,
            &session('b'),
            None,
            UnableToDecryptReason::UnknownMegolmMessageIndex,
        ),
    )
    .expect("缺消息序号也要请求");
    assert_eq!(without_device.device, None, "没点名设备时发给对方每台设备");

    assert!(
        requestable(
            Some(&me),
            &room(),
            &undecryptable(ME, &session('c'), Some("MYOLDDEVICE"), missing.clone())
        )
        .is_none(),
        "自己别的设备建的会话不问"
    );
    assert!(
        requestable(
            Some(&me),
            &room(),
            &undecryptable(
                SENDER,
                &session('d'),
                None,
                UnableToDecryptReason::MismatchedIdentityKeys
            )
        )
        .is_none(),
        "不是缺密钥的解不开不问"
    );
    assert!(
        requestable(
            Some(&me),
            &room(),
            &undecryptable(SENDER, "short", None, missing)
        )
        .is_none(),
        "会话标识不合格式不问"
    );
}

#[test]
fn 同一个会话一小时内只请求一次() {
    let requester = RoomKeyRequester::default();
    let now = Instant::now();

    assert!(requester.first_request_in_interval(&room(), &session('a'), now));
    assert!(!requester.first_request_in_interval(
        &room(),
        &session('a'),
        now + Duration::from_mins(59)
    ));
    assert!(requester.first_request_in_interval(&room(), &session('b'), now));
    let other = RoomId::parse(OTHER_ROOM).expect("房间标识有效");
    assert!(requester.first_request_in_interval(&other, &session('a'), now));
    assert!(requester.first_request_in_interval(&room(), &session('a'), now + REQUEST_INTERVAL));
}

#[test]
fn 应答要对得上发过的请求_人_设备_房间_会话和公钥都要一致() {
    let requester = RoomKeyRequester::default();
    requester.remember(
        "request-1",
        &target(Some(SENDER_DEVICE)),
        &[session('a'), session('b')],
    );

    assert!(
        requester
            .matching_request(&answer("request-1", &[session('a')]), &origin())
            .is_ok()
    );
    assert!(
        requester
            .matching_request(&answer("unknown", &[session('a')]), &origin())
            .is_err(),
        "不是我们发过的请求"
    );
    assert!(
        requester
            .matching_request(&answer("request-1", &[session('z')]), &origin())
            .is_err(),
        "夹带没请求的会话"
    );
    let mut stranger = origin();
    stranger.user_id = UserId::parse("@stranger:agent-room.test").expect("用户标识有效");
    assert!(
        requester
            .matching_request(&answer("request-1", &[session('a')]), &stranger)
            .is_err(),
        "别人冒充应答"
    );
    let mut other_device = origin();
    other_device.device_id = OwnedDeviceId::from("OTHERDEVICE");
    assert!(
        requester
            .matching_request(&answer("request-1", &[session('a')]), &other_device)
            .is_err(),
        "点名的设备之外的设备应答"
    );
    let mut other_room = answer("request-1", &[session('a')]);
    other_room.room_id = OTHER_ROOM.to_owned();
    assert!(
        requester.matching_request(&other_room, &origin()).is_err(),
        "别的房间"
    );
    let mut other_key = answer("request-1", &[session('a')]);
    other_key.sender_key = "x".repeat(43);
    assert!(
        requester.matching_request(&other_key, &origin()).is_err(),
        "外层公钥不是 Olm 送达的那台设备"
    );
}

#[test]
fn 没点名设备的请求_对方哪台设备应答都行() {
    let requester = RoomKeyRequester::default();
    requester.remember("request-2", &target(None), &[session('a')]);
    let mut any_device = origin();
    any_device.device_id = OwnedDeviceId::from("ANYDEVICE");

    assert!(
        requester
            .matching_request(&answer("request-2", &[session('a')]), &any_device)
            .is_ok()
    );
}

#[test]
fn 导入后点名的会话不再等_房间记为有新密钥_取走一次就清空() {
    let requester = RoomKeyRequester::default();
    requester.remember(
        "request-3",
        &target(Some(SENDER_DEVICE)),
        &[session('a'), session('b')],
    );

    requester.settle(&answer("request-3", &[session('a')]));
    assert!(
        requester
            .matching_request(&answer("request-3", &[session('b')]), &origin())
            .is_ok()
    );
    assert!(
        requester
            .matching_request(&answer("request-3", &[session('a')]), &origin())
            .is_err()
    );

    requester.settle(&answer("request-3", &[session('b')]));
    assert!(
        requester
            .matching_request(&answer("request-3", &[session('b')]), &origin())
            .is_err()
    );

    assert_eq!(requester.take_recovered_rooms(), vec![room()]);
    assert!(requester.take_recovered_rooms().is_empty());
}

#[test]
fn 等应答的请求有上限_满了丢最早的() {
    let requester = RoomKeyRequester::default();
    for index in 0..MAX_PENDING_REQUESTS {
        requester.remember(&format!("request-{index}"), &target(None), &[session('a')]);
    }
    requester.remember("newest", &target(None), &[session('a')]);

    let pending = requester.pending.lock().expect("锁可用");
    assert_eq!(pending.len(), MAX_PENDING_REQUESTS);
    assert!(pending.contains_key("newest"));
    assert!(!pending.contains_key("request-0"));
}

#[test]
fn 应答格式不合的一律不理() {
    assert!(valid_answer(&answer("request", &[session('a')])));

    let mut wrong_type = answer("request", &[session('a')]);
    wrong_type.event_type = "io.github.rainyflash.agentroom.room_keys.request.v1".to_owned();
    assert!(!valid_answer(&wrong_type));

    assert!(!valid_answer(&answer("request", &[])), "没有密钥");
    let too_many = (0..=KEYS_PER_MESSAGE)
        .map(|index| session(char::from(b'a' + u8::try_from(index % 26).expect("字母"))))
        .collect::<Vec<_>>();
    assert!(!valid_answer(&answer("request", &too_many)), "超过每条上限");

    let mut bad_key = answer("request", &[session('a')]);
    bad_key.keys[0].session_key = "not base64!".to_owned();
    assert!(!valid_answer(&bad_key));

    let mut bad_sender = answer("request", &[session('a')]);
    bad_sender.sender_ed25519_key = "short".to_owned();
    assert!(!valid_answer(&bad_sender));

    let owned: OwnedUserId = UserId::parse(SENDER).expect("用户标识有效");
    assert_eq!(owned.as_str(), SENDER);
}
