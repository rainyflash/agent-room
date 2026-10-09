use agent_room_application::ports::{
    MatrixBackfillPage, MatrixBackfillToken, MatrixEventId, MatrixEventType, MatrixFailure,
    MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId, MatrixRoomStatePosition,
    MatrixRoomSync, MatrixRoomSyncKind, MatrixSyncBatch, MatrixSyncToken, MatrixTimelineEvent,
    MatrixTransactionId, MatrixUserId, MatrixUserPresence,
};
use agent_room_domain::agent_lifecycle::MatrixPresenceState;
use matrix_sdk::{
    deserialized_responses::{TimelineEvent, VerificationLevel, VerificationState},
    ruma::{
        events::{AnySyncEphemeralRoomEvent, presence::PresenceEvent},
        serde::Raw,
    },
    sync::{RoomUpdates, State, SyncResponse},
};
use serde::Deserialize;
use serde_json::Value;

use crate::{error::invalid_response, trust::SenderTrustUpgrades};

const MAX_RAW_EVENT_BYTES: usize = 131_072;

pub(crate) fn map_sync_response(
    response: &SyncResponse,
    upgrades: &SenderTrustUpgrades,
) -> MatrixResult<MatrixSyncBatch> {
    let next_batch = MatrixSyncToken::new(response.next_batch.clone())
        .map_err(|_| invalid_response_failure(MatrixOperation::Sync))?;
    let rooms = map_room_updates(&response.rooms, upgrades)?;
    Ok(MatrixSyncBatch::new(next_batch, rooms).with_presence(map_presence(&response.presence)))
}

/// 同步里的 `m.presence`。只认三种标准状态；读不出来的跳过，不让整次同步失败。
fn map_presence(events: &[Raw<PresenceEvent>]) -> Vec<MatrixUserPresence> {
    events
        .iter()
        .filter_map(|raw| {
            let sender = raw.get_field::<String>("sender").ok().flatten()?;
            let content = raw.get_field::<PresenceContent>("content").ok().flatten()?;
            Some(MatrixUserPresence::new(
                MatrixUserId::new(sender).ok()?,
                map_presence_state(&content.presence)?,
                content.last_active_ago,
            ))
        })
        .collect()
}

pub(crate) fn map_presence_state(value: &str) -> Option<MatrixPresenceState> {
    match value {
        "online" => Some(MatrixPresenceState::Online),
        "unavailable" => Some(MatrixPresenceState::Unavailable),
        "offline" => Some(MatrixPresenceState::Offline),
        _ => None,
    }
}

#[derive(Deserialize)]
struct PresenceContent {
    presence: String,
    #[serde(default)]
    last_active_ago: Option<u64>,
}

pub(crate) fn map_backfill(
    response: &matrix_sdk::room::Messages,
    upgrades: &SenderTrustUpgrades,
) -> MatrixResult<MatrixBackfillPage> {
    let start = MatrixBackfillToken::new(response.start.clone())
        .map_err(|_| invalid_response_failure(MatrixOperation::Backfill))?;
    let end = response
        .end
        .as_ref()
        .map(|value| MatrixBackfillToken::new(value.clone()))
        .transpose()
        .map_err(|_| invalid_response_failure(MatrixOperation::Backfill))?;
    let events = response
        .chunk
        .iter()
        .map(|event| map_timeline_event(event, MatrixOperation::Backfill, upgrades))
        .collect::<MatrixResult<Vec<_>>>()?;
    Ok(MatrixBackfillPage::new(start, end, events))
}

fn map_room_updates(
    updates: &RoomUpdates,
    upgrades: &SenderTrustUpgrades,
) -> MatrixResult<Vec<MatrixRoomSync>> {
    let mut rooms = Vec::with_capacity(
        updates.joined.len() + updates.invited.len() + updates.left.len() + updates.knocked.len(),
    );
    for (room_id, update) in &updates.joined {
        let (state_position, state) = map_state(&update.state)?;
        let mut room = MatrixRoomSync::new(
            map_room_id(room_id.as_str())?,
            MatrixRoomSyncKind::Joined,
            update.timeline.limited,
            map_optional_backfill_token(update.timeline.prev_batch.as_deref())?,
            map_timeline(&update.timeline.events, upgrades)?,
            state,
        )
        .with_state_position(state_position);
        if let Some(typing) = map_typing(&update.ephemeral) {
            room = room.with_typing(typing);
        }
        rooms.push(room);
    }
    for (room_id, update) in &updates.invited {
        rooms.push(MatrixRoomSync::new(
            map_room_id(room_id.as_str())?,
            MatrixRoomSyncKind::Invited,
            false,
            None,
            Vec::new(),
            map_raw_events(&update.invite_state.events, MatrixOperation::Sync)?,
        ));
    }
    for (room_id, update) in &updates.left {
        let (state_position, state) = map_state(&update.state)?;
        rooms.push(
            MatrixRoomSync::new(
                map_room_id(room_id.as_str())?,
                MatrixRoomSyncKind::Left,
                update.timeline.limited,
                map_optional_backfill_token(update.timeline.prev_batch.as_deref())?,
                map_timeline(&update.timeline.events, upgrades)?,
                state,
            )
            .with_state_position(state_position),
        );
    }
    for (room_id, update) in &updates.knocked {
        rooms.push(MatrixRoomSync::new(
            map_room_id(room_id.as_str())?,
            MatrixRoomSyncKind::Knocked,
            false,
            None,
            Vec::new(),
            map_raw_events(&update.knock_state.events, MatrixOperation::Sync)?,
        ));
    }
    Ok(rooms)
}

/// 这一段里最后一个 `m.typing`：此刻在打字的人。先看类型，回执之类的不整个解析；
/// 读不出来的用户 ID 跳过。
fn map_typing(ephemeral: &[Raw<AnySyncEphemeralRoomEvent>]) -> Option<Vec<MatrixUserId>> {
    ephemeral.iter().rev().find_map(|raw| {
        if raw.get_field::<String>("type").ok().flatten().as_deref() != Some("m.typing") {
            return None;
        }
        let content = raw.get_field::<TypingContent>("content").ok().flatten()?;
        Some(
            content
                .user_ids
                .into_iter()
                .filter_map(|user_id| MatrixUserId::new(user_id).ok())
                .collect(),
        )
    })
}

#[derive(Deserialize)]
struct TypingContent {
    #[serde(default)]
    user_ids: Vec<String>,
}

fn map_timeline(
    events: &[TimelineEvent],
    upgrades: &SenderTrustUpgrades,
) -> MatrixResult<Vec<MatrixTimelineEvent>> {
    events
        .iter()
        .map(|event| map_timeline_event(event, MatrixOperation::Sync, upgrades))
        .collect()
}

pub(crate) fn map_timeline_event(
    event: &TimelineEvent,
    operation: MatrixOperation,
    upgrades: &SenderTrustUpgrades,
) -> MatrixResult<MatrixTimelineEvent> {
    let mapped = map_raw_event(event.raw(), operation)?;
    Ok(match event.encryption_info() {
        Some(info)
            if sender_device_trusted(&info.verification_state) || upgrades.contains(event) =>
        {
            mapped.with_trusted_end_to_end_encryption()
        }
        Some(_) => mapped.with_untrusted_end_to_end_encryption(),
        None => mapped,
    })
}

/// 发送设备由其主人的加密身份签名即可信，不要求本机核对过对方（首次见到时记住身份）。
///
/// 未签名或未知的设备、发送者不符，以及曾核对过又换了身份的用户仍被隔离。
pub(crate) const fn sender_device_trusted(state: &VerificationState) -> bool {
    matches!(
        state,
        VerificationState::Verified
            | VerificationState::Unverified(VerificationLevel::UnverifiedIdentity)
    )
}

fn map_state(state: &State) -> MatrixResult<(MatrixRoomStatePosition, Vec<MatrixTimelineEvent>)> {
    match state {
        State::Before(events) => Ok((
            MatrixRoomStatePosition::BeforeTimeline,
            map_raw_events(events, MatrixOperation::Sync)?,
        )),
        State::After(events) => Ok((
            MatrixRoomStatePosition::AfterTimeline,
            map_raw_events(events, MatrixOperation::Sync)?,
        )),
    }
}

fn map_raw_events<T>(
    events: &[Raw<T>],
    operation: MatrixOperation,
) -> MatrixResult<Vec<MatrixTimelineEvent>> {
    events
        .iter()
        .map(|event| map_raw_event(event, operation))
        .collect()
}

fn map_raw_event<T>(
    event: &Raw<T>,
    operation: MatrixOperation,
) -> MatrixResult<MatrixTimelineEvent> {
    let raw = event.json().get();
    if raw.len() > MAX_RAW_EVENT_BYTES {
        return invalid_response(operation);
    }
    let envelope: EventEnvelope =
        serde_json::from_str(raw).map_err(|_| invalid_response_failure(operation))?;
    let event_id = envelope
        .event_id
        .map(MatrixEventId::new)
        .transpose()
        .map_err(|_| invalid_response_failure(operation))?;
    let sender = envelope
        .sender
        .map(MatrixUserId::new)
        .transpose()
        .map_err(|_| invalid_response_failure(operation))?;
    let event_type = MatrixEventType::new(envelope.event_type)
        .map_err(|_| invalid_response_failure(operation))?;
    let (transaction_id, previous_membership) =
        envelope.unsigned.map_or((None, None), |unsigned| {
            (
                unsigned.transaction_id,
                unsigned
                    .prev_content
                    .and_then(|previous| previous.membership),
            )
        });
    let transaction_id = transaction_id
        .map(MatrixTransactionId::new)
        .transpose()
        .map_err(|_| invalid_response_failure(operation))?;
    MatrixTimelineEvent::new(
        event_id,
        sender,
        event_type,
        envelope.state_key,
        transaction_id,
        envelope.origin_server_timestamp,
        envelope.content,
    )
    .map(|event| event.with_previous_membership(previous_membership))
    .map_err(|_| invalid_response_failure(operation))
}

fn map_room_id(value: &str) -> MatrixResult<MatrixRoomId> {
    MatrixRoomId::new(value.to_owned()).map_err(|_| invalid_response_failure(MatrixOperation::Sync))
}

fn map_optional_backfill_token(value: Option<&str>) -> MatrixResult<Option<MatrixBackfillToken>> {
    value
        .map(|token| MatrixBackfillToken::new(token.to_owned()))
        .transpose()
        .map_err(|_| invalid_response_failure(MatrixOperation::Sync))
}

const fn invalid_response_failure(operation: MatrixOperation) -> MatrixFailure {
    MatrixFailure::new(operation, MatrixFailureKind::InvalidResponse)
}

#[derive(Debug, Deserialize)]
struct EventEnvelope {
    #[serde(default)]
    event_id: Option<String>,
    #[serde(default)]
    sender: Option<String>,
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    state_key: Option<String>,
    #[serde(rename = "origin_server_ts", default)]
    origin_server_timestamp: Option<u64>,
    #[serde(default)]
    unsigned: Option<EventUnsigned>,
    content: Value,
}

#[derive(Debug, Deserialize)]
struct EventUnsigned {
    #[serde(default)]
    transaction_id: Option<String>,
    #[serde(default)]
    prev_content: Option<PreviousContent>,
}

/// 状态事件之前的内容：只留成员状态，用来分清加入和改昵称。
#[derive(Debug, Deserialize)]
struct PreviousContent {
    #[serde(default)]
    membership: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use matrix_sdk::{
        deserialized_responses::{
            AlgorithmInfo, DecryptedRoomEvent, DeviceLinkProblem, EncryptionInfo, TimelineEvent,
            VerificationLevel, VerificationState,
        },
        ruma::{
            OwnedDeviceId, UserId,
            events::{AnySyncTimelineEvent, AnyTimelineEvent},
            serde::Raw,
        },
    };

    use super::{
        MatrixOperation, SenderTrustUpgrades, map_presence, map_raw_event, map_timeline_event,
        map_typing,
    };
    use agent_room_domain::agent_lifecycle::MatrixPresenceState;

    #[test]
    fn 在线状态只认三种标准状态_读不出来的跳过() {
        let raw = |json: &str| Raw::from_json_string(json.to_owned()).expect("原始 JSON 有效");
        let events = [
            raw(
                r#"{"type":"m.presence","sender":"@ada:example.org","content":{"presence":"online","last_active_ago":0,"currently_active":true}}"#,
            ),
            raw(
                r#"{"type":"m.presence","sender":"@bob:example.org","content":{"presence":"unavailable"}}"#,
            ),
            raw(
                r#"{"type":"m.presence","sender":"@cy:example.org","content":{"presence":"offline","last_active_ago":1200000}}"#,
            ),
            raw(
                r#"{"type":"m.presence","sender":"@dee:example.org","content":{"presence":"busy"}}"#,
            ),
            raw(r#"{"type":"m.presence","sender":"not a user","content":{"presence":"online"}}"#),
            raw(r#"{"type":"m.presence","sender":"@eve:example.org","content":{}}"#),
        ];
        let presence = map_presence(&events);
        let presence: Vec<_> = presence
            .iter()
            .map(|entry| {
                (
                    entry.user_id().as_str(),
                    entry.state(),
                    entry.last_active_ago_ms(),
                )
            })
            .collect();
        assert_eq!(
            presence,
            [
                ("@ada:example.org", MatrixPresenceState::Online, Some(0)),
                ("@bob:example.org", MatrixPresenceState::Unavailable, None),
                (
                    "@cy:example.org",
                    MatrixPresenceState::Offline,
                    Some(1_200_000)
                ),
            ]
        );
    }

    #[test]
    fn 正在输入取这一段里最后一次的完整名单() {
        let raw = |json: &str| Raw::from_json_string(json.to_owned()).expect("原始 JSON 有效");
        let ephemeral = [
            raw(r#"{"type":"m.typing","content":{"user_ids":["@ada:example.org"]}}"#),
            raw(r#"{"type":"m.receipt","content":{"$event:example.org":{}}}"#),
            raw(
                r#"{"type":"m.typing","content":{"user_ids":["@ada:example.org","@bob:example.org","not a user"]}}"#,
            ),
        ];
        let typing = map_typing(&ephemeral).expect("带回了正在输入");
        let typing: Vec<&str> = typing
            .iter()
            .map(agent_room_application::ports::MatrixUserId::as_str)
            .collect();
        assert_eq!(typing, ["@ada:example.org", "@bob:example.org"]);

        let stopped = [raw(r#"{"type":"m.typing","content":{"user_ids":[]}}"#)];
        assert_eq!(map_typing(&stopped), Some(Vec::new()), "都停了就是空名单");
        let receipts = [raw(r#"{"type":"m.receipt","content":{}}"#)];
        assert_eq!(map_typing(&receipts), None, "没变就没有");
    }

    #[test]
    fn 原始事件保留事务标识并剥离无关字段() {
        let raw = Raw::<AnySyncTimelineEvent>::from_json_string(
            r#"{
                "type":"io.github.rainyflash.agentroom.message.preview.v1",
                "event_id":"$event:example.org",
                "sender":"@agent:example.org",
                "origin_server_ts":1234,
                "unsigned":{"transaction_id":"txn-stable","age":5},
                "content":{"schemaVersion":"1.0"}
            }"#
            .to_owned(),
        )
        .expect("原始事件 JSON 有效");

        let event = map_raw_event(&raw, MatrixOperation::Sync).expect("事件映射成功");
        assert_eq!(
            event.event_id().expect("事件标识存在").as_str(),
            "$event:example.org"
        );
        assert_eq!(
            event.transaction_id().expect("事务标识存在").as_str(),
            "txn-stable"
        );
    }

    #[test]
    fn 成员事件留着上一个成员状态_分清加入和改昵称() {
        let raw = |prev: &str| {
            Raw::<AnySyncTimelineEvent>::from_json_string(format!(
                r#"{{"type":"m.room.member","state_key":"@scout:example.org","event_id":"$m:example.org","sender":"@scout:example.org","origin_server_ts":1234,"content":{{"membership":"join"}}{prev}}}"#
            ))
            .expect("原始事件 JSON 有效")
        };
        let renamed = map_raw_event(
            &raw(r#","unsigned":{"prev_content":{"membership":"join","displayname":"旧名"}}"#),
            MatrixOperation::Sync,
        )
        .expect("事件映射成功");
        assert_eq!(renamed.previous_membership(), Some("join"));
        let joined = map_raw_event(
            &raw(r#","unsigned":{"prev_content":{"membership":"invite"}}"#),
            MatrixOperation::Sync,
        )
        .expect("事件映射成功");
        assert_eq!(joined.previous_membership(), Some("invite"));
        let first = map_raw_event(&raw(""), MatrixOperation::Sync).expect("事件映射成功");
        assert_eq!(first.previous_membership(), None);
    }

    #[test]
    fn 恶意超大事件在反序列化前被拒绝() {
        let raw = Raw::<AnySyncTimelineEvent>::from_json_string(format!(
            "{{\"type\":\"io.github.rainyflash.agentroom.test.v1\",\"content\":{{\"body\":\"{}\"}}}}",
            "x".repeat(131_072)
        ))
        .expect("原始 JSON 有效");

        assert!(map_raw_event(&raw, MatrixOperation::Sync).is_err());
    }

    #[test]
    fn 主人签名的发送设备无需本机核对即可信() {
        // 核对过安全码的，以及只是由主人签名、本机没核对过的（首次见到时记住身份）都可信。
        for state in [
            VerificationState::Verified,
            VerificationState::Unverified(VerificationLevel::UnverifiedIdentity),
        ] {
            let event = map_timeline_event(
                &decrypted_event(state),
                MatrixOperation::Sync,
                &SenderTrustUpgrades::default(),
            )
            .expect("可信加密事件可映射");
            assert!(event.end_to_end_encrypted());
            assert!(event.end_to_end_sender_trusted());
        }
    }

    #[test]
    fn 未签名设备和换了身份的核对对象仍被隔离() {
        for state in [
            VerificationLevel::UnsignedDevice,
            VerificationLevel::VerificationViolation,
            VerificationLevel::MismatchedSender,
            VerificationLevel::None(DeviceLinkProblem::MissingDevice),
        ] {
            let event = map_timeline_event(
                &decrypted_event(VerificationState::Unverified(state)),
                MatrixOperation::Sync,
                &SenderTrustUpgrades::default(),
            )
            .expect("未可信加密事件仍应进入隔离边界");
            assert!(event.end_to_end_encrypted());
            assert!(!event.end_to_end_sender_trusted());
        }
    }

    fn decrypted_event(verification_state: VerificationState) -> TimelineEvent {
        let event = Raw::<AnyTimelineEvent>::from_json_string(
            r#"{
                "type":"io.github.rainyflash.agentroom.message.preview.v1",
                "event_id":"$encrypted:example.org",
                "sender":"@agent:example.org",
                "room_id":"!room:example.org",
                "origin_server_ts":1234,
                "content":{"schemaVersion":"1.0"}
            }"#
            .to_owned(),
        )
        .expect("解密事件 JSON 有效");
        TimelineEvent::from_decrypted(
            DecryptedRoomEvent {
                event,
                encryption_info: Arc::new(EncryptionInfo {
                    sender: UserId::parse("@agent:example.org").expect("用户标识有效"),
                    sender_device: Some(OwnedDeviceId::from("AGENT_DEVICE")),
                    forwarder: None,
                    algorithm_info: AlgorithmInfo::MegolmV1AesSha2 {
                        curve25519_key: "curve-key".to_owned(),
                        sender_claimed_keys: BTreeMap::new(),
                        session_id: Some("session-id".to_owned()),
                    },
                    verification_state,
                }),
                unsigned_encryption_info: None,
            },
            None,
        )
    }
}
