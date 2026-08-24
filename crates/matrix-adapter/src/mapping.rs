use agent_room_application::ports::{
    MatrixBackfillPage, MatrixBackfillToken, MatrixEventId, MatrixEventType, MatrixFailure,
    MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId, MatrixRoomSync,
    MatrixRoomSyncKind, MatrixSyncBatch, MatrixSyncToken, MatrixTimelineEvent, MatrixTransactionId,
    MatrixUserId,
};
use matrix_sdk::{
    deserialized_responses::TimelineEvent,
    ruma::serde::Raw,
    sync::{RoomUpdates, State, SyncResponse},
};
use serde::Deserialize;
use serde_json::Value;

use crate::error::invalid_response;

const MAX_RAW_EVENT_BYTES: usize = 131_072;

pub(crate) fn map_sync_response(response: &SyncResponse) -> MatrixResult<MatrixSyncBatch> {
    let next_batch = MatrixSyncToken::new(response.next_batch.clone())
        .map_err(|_| invalid_response_failure(MatrixOperation::Sync))?;
    let rooms = map_room_updates(&response.rooms)?;
    Ok(MatrixSyncBatch::new(next_batch, rooms))
}

pub(crate) fn map_backfill(
    response: &matrix_sdk::room::Messages,
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
        .map(|event| map_timeline_event(event, MatrixOperation::Backfill))
        .collect::<MatrixResult<Vec<_>>>()?;
    Ok(MatrixBackfillPage::new(start, end, events))
}

fn map_room_updates(updates: &RoomUpdates) -> MatrixResult<Vec<MatrixRoomSync>> {
    let mut rooms = Vec::with_capacity(
        updates.joined.len() + updates.invited.len() + updates.left.len() + updates.knocked.len(),
    );
    for (room_id, update) in &updates.joined {
        rooms.push(MatrixRoomSync::new(
            map_room_id(room_id.as_str())?,
            MatrixRoomSyncKind::Joined,
            update.timeline.limited,
            map_optional_backfill_token(update.timeline.prev_batch.as_deref())?,
            map_timeline(&update.timeline.events)?,
            map_state(&update.state)?,
        ));
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
        rooms.push(MatrixRoomSync::new(
            map_room_id(room_id.as_str())?,
            MatrixRoomSyncKind::Left,
            update.timeline.limited,
            map_optional_backfill_token(update.timeline.prev_batch.as_deref())?,
            map_timeline(&update.timeline.events)?,
            map_state(&update.state)?,
        ));
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

fn map_timeline(events: &[TimelineEvent]) -> MatrixResult<Vec<MatrixTimelineEvent>> {
    events
        .iter()
        .map(|event| map_timeline_event(event, MatrixOperation::Sync))
        .collect()
}

fn map_timeline_event(
    event: &TimelineEvent,
    operation: MatrixOperation,
) -> MatrixResult<MatrixTimelineEvent> {
    map_raw_event(event.raw(), operation)
}

fn map_state(state: &State) -> MatrixResult<Vec<MatrixTimelineEvent>> {
    match state {
        State::Before(events) | State::After(events) => {
            map_raw_events(events, MatrixOperation::Sync)
        }
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
    let transaction_id = envelope
        .unsigned
        .and_then(|value| value.transaction_id)
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
}

#[cfg(test)]
mod tests {
    use matrix_sdk::ruma::{events::AnySyncTimelineEvent, serde::Raw};

    use super::{MatrixOperation, map_raw_event};

    #[test]
    fn 原始事件保留事务标识并剥离无关字段() {
        let raw = Raw::<AnySyncTimelineEvent>::from_json_string(
            r#"{
                "type":"org.agentroom.message.preview.v1",
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
    fn 恶意超大事件在反序列化前被拒绝() {
        let raw = Raw::<AnySyncTimelineEvent>::from_json_string(format!(
            "{{\"type\":\"org.agentroom.test.v1\",\"content\":{{\"body\":\"{}\"}}}}",
            "x".repeat(131_072)
        ))
        .expect("原始 JSON 有效");

        assert!(map_raw_event(&raw, MatrixOperation::Sync).is_err());
    }
}
