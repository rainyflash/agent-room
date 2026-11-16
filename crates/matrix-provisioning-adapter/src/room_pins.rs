//! 房间的置顶（`specs/redesign/brief-and-pins.md`“置顶”）：以应用服务改 Matrix 标准的
//! `m.room.pinned_events`（`pinned` 是事件 ID 的列表，别的 Matrix 客户端也认得），谁、什么时候置顶的
//! 另记在我们自己的字段 [`PINS_FIELD`] 里：状态事件是应用服务发的，发件人看不出是谁。
//!
//! 只记事件 ID，不记那句写了什么：加密房间里那句照旧是加密的，在各自的设备上解开。

use std::hash::{DefaultHasher, Hash, Hasher};

use agent_room_application::ports::{
    MatrixFailure, MatrixFailureKind, MatrixOperation, MatrixResult, MatrixRoomId, PinTarget,
    PortFuture, PrivateRoomPinsGateway, RoomPinsChange, RoomPinsWriteFailure,
};
use agent_room_domain::{
    DomainError,
    room_pins::{RoomPin, RoomPinEventId, RoomPins},
    time::UtcMillis,
};
use chrono::{DateTime, SecondsFormat};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    MatrixApplicationServiceProvisioner, decode_json, decode_matrix_error, invalid_response,
    map_matrix_error, map_transport_error, read_body_within, read_limited_body,
    room_listing::put_room_state, rooms::endpoint_with_segments,
};

const PINNED_EVENTS_TYPE: &str = "m.room.pinned_events";

/// `m.room.pinned_events` 里我们自己的那一段：每一句谁、什么时候置顶的，和最近一次改了什么。
const PINS_FIELD: &str = "io.github.rainyflash.agentroom.pins";

/// 读一条事件时最多收多少字节：一条事件最大 64 KiB，再留出 `unsigned` 的余量。
const MAX_EVENT_RESPONSE_BYTES: usize = 96 * 1_024;

/// 能置顶的是消息：加密房间里外面都是 `m.room.encrypted`；不加密的房间里是我们自己的消息类型；
/// 别的 Matrix 客户端发的 `m.room.message` 也算。改过的那一版（`revision`）不算，置顶原来那句。
const MESSAGE_EVENT_TYPES: [&str; 4] = [
    "m.room.encrypted",
    "m.room.message",
    "io.github.rainyflash.agentroom.message.preview.v1",
    "io.github.rainyflash.agentroom.message.preview.v2",
];

/// 改置顶是“读出整份、改一项、整份写回”，Matrix 没有按版本写的办法：两个人同时置顶，后写的那份
/// 没有先置顶的那条。所以这个进程里改同一个房间的置顶一次只做一个（按房间分到固定的几把锁上），
/// 和改权限一样（`rooms.rs` 的 `update_power_levels`）。生产上只有一个控制面。
static PIN_LOCKS: [tokio::sync::Mutex<()>; 64] = [const { tokio::sync::Mutex::const_new(()) }; 64];

fn pin_lock(room_id: &MatrixRoomId) -> &'static tokio::sync::Mutex<()> {
    let mut hasher = DefaultHasher::new();
    room_id.as_str().hash(&mut hasher);
    let index = usize::try_from(hasher.finish() % 64).unwrap_or_default();
    &PIN_LOCKS[index]
}

impl PrivateRoomPinsGateway for MatrixApplicationServiceProvisioner {
    fn pin_target<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        event_id: &'a RoomPinEventId,
    ) -> PortFuture<'a, MatrixResult<PinTarget>> {
        Box::pin(read_pin_target(self, room_id, event_id))
    }

    fn change_pins<'a>(
        &'a self,
        room_id: &'a MatrixRoomId,
        change: &'a RoomPinsChange,
    ) -> PortFuture<'a, Result<RoomPins, RoomPinsWriteFailure>> {
        Box::pin(async move {
            let _room = pin_lock(room_id).lock().await;
            let mut pins = read_pins(self, room_id).await?;
            let changed = if change.pinned {
                pins.pin(RoomPin {
                    event_id: change.event_id.clone(),
                    pinned_by: change.actor.as_str().to_owned(),
                    pinned_at: change.at,
                })
                .map_err(|error| match error {
                    DomainError::CapacityExceeded { .. } => RoomPinsWriteFailure::Full,
                    _ => RoomPinsWriteFailure::Matrix(invalid_configuration()),
                })?
            } else {
                pins.unpin(&change.event_id)
            };
            if changed {
                put_room_state(self, room_id, PINNED_EVENTS_TYPE, &content(&pins, change)?).await?;
            }
            Ok(pins)
        })
    }
}

/// 以建房间的应用服务账号读这一条事件（它从建房起就在房间里，看得到每一句）。不在这间房里、读不到的，
/// Synapse 一律回 404。
async fn read_pin_target(
    provisioner: &MatrixApplicationServiceProvisioner,
    room_id: &MatrixRoomId,
    event_id: &RoomPinEventId,
) -> MatrixResult<PinTarget> {
    let operation = MatrixOperation::ReadRoomEvent;
    let endpoint = endpoint_with_segments(
        &provisioner.homeserver_url,
        &[
            "_matrix",
            "client",
            "v3",
            "rooms",
            room_id.as_str(),
            "event",
            event_id.as_str(),
        ],
        operation,
    )?;
    let response = provisioner
        .client
        .get(endpoint)
        .bearer_auth(provisioner.access_token.expose())
        .send()
        .await
        .map_err(|error| map_transport_error(operation, &error))?;
    let status = response.status();
    if status == StatusCode::NOT_FOUND {
        return Ok(PinTarget::NotMessage);
    }
    if !status.is_success() {
        let body = read_limited_body(response, operation).await?;
        let error = decode_matrix_error(&body, operation)?;
        return Err(map_matrix_error(operation, status, &error));
    }
    let body = read_body_within(response, operation, MAX_EVENT_RESPONSE_BYTES).await?;
    let event: RoomEvent = decode_json(&body, operation)?;
    let message = event.state_key.is_none()
        && MESSAGE_EVENT_TYPES.contains(&event.event_type.as_str())
        && event
            .unsigned
            .as_ref()
            .is_none_or(|unsigned| unsigned.redacted_because.is_none());
    Ok(if message {
        PinTarget::Message
    } else {
        PinTarget::NotMessage
    })
}

#[derive(Deserialize)]
struct RoomEvent {
    #[serde(rename = "type")]
    event_type: String,
    state_key: Option<String>,
    unsigned: Option<EventUnsigned>,
}

#[derive(Deserialize)]
struct EventUnsigned {
    redacted_because: Option<Value>,
}

#[derive(Deserialize)]
struct PinnedEventsState {
    #[serde(default)]
    pinned: Vec<String>,
    #[serde(rename = "io.github.rainyflash.agentroom.pins")]
    pins: Option<PinsField>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PinsField {
    entries: Vec<PinEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PinEntry {
    event_id: String,
    pinned_by: String,
    pinned_at: String,
}

/// 读这间房此刻的置顶。还没置顶过（没有这条状态）就是空的。`pinned` 是标准的那份，每一句谁、什么时候
/// 置顶的在我们的字段里，只留两边都有的；`pinned` 里有、我们的字段里没有，说明状态不是我们写的，报错，
/// 免得写回时把它丢掉。
async fn read_pins(
    provisioner: &MatrixApplicationServiceProvisioner,
    room_id: &MatrixRoomId,
) -> MatrixResult<RoomPins> {
    let operation = MatrixOperation::ReadRoomState;
    let endpoint = endpoint_with_segments(
        &provisioner.homeserver_url,
        &[
            "_matrix",
            "client",
            "v3",
            "rooms",
            room_id.as_str(),
            "state",
            PINNED_EVENTS_TYPE,
        ],
        operation,
    )?;
    let response = provisioner
        .client
        .get(endpoint)
        .bearer_auth(provisioner.access_token.expose())
        .send()
        .await
        .map_err(|error| map_transport_error(operation, &error))?;
    let status = response.status();
    let body = read_limited_body(response, operation).await?;
    if status == StatusCode::NOT_FOUND {
        return Ok(RoomPins::default());
    }
    if !status.is_success() {
        let error = decode_matrix_error(&body, operation)?;
        return Err(map_matrix_error(operation, status, &error));
    }
    let state: PinnedEventsState = decode_json(&body, operation)?;
    let entries = state.pins.map(|field| field.entries).unwrap_or_default();
    let mut pins = Vec::with_capacity(entries.len());
    for entry in entries {
        if !state.pinned.contains(&entry.event_id) {
            continue;
        }
        pins.push(RoomPin {
            event_id: RoomPinEventId::parse(&entry.event_id)
                .map_err(|_| invalid_response(operation))?,
            pinned_by: entry.pinned_by,
            pinned_at: parse_time(&entry.pinned_at).ok_or_else(|| invalid_response(operation))?,
        });
    }
    let pins = RoomPins::new(pins);
    if state
        .pinned
        .iter()
        .any(|id| !pins.entries().iter().any(|pin| pin.event_id.as_str() == id))
    {
        return Err(invalid_response(operation));
    }
    Ok(pins)
}

fn content(pins: &RoomPins, change: &RoomPinsChange) -> MatrixResult<Value> {
    let entries = pins
        .entries()
        .iter()
        .map(|pin| {
            Ok(json!({
                "eventId": pin.event_id.as_str(),
                "pinnedBy": pin.pinned_by,
                "pinnedAt": format_time(pin.pinned_at)?,
            }))
        })
        .collect::<MatrixResult<Vec<_>>>()?;
    Ok(json!({
        "pinned": pins.entries().iter().map(|pin| pin.event_id.as_str()).collect::<Vec<_>>(),
        PINS_FIELD: {
            "schemaVersion": "1.0",
            "entries": entries,
            "updatedBy": change.actor.as_str(),
            "updatedAt": format_time(change.at)?,
            "change": {
                "kind": if change.pinned { "pinned" } else { "unpinned" },
                "eventId": change.event_id.as_str(),
            },
        },
    }))
}

fn format_time(at: UtcMillis) -> MatrixResult<String> {
    DateTime::from_timestamp_millis(at.value())
        .map(|time| time.to_rfc3339_opts(SecondsFormat::Millis, true))
        .ok_or_else(invalid_configuration)
}

fn parse_time(value: &str) -> Option<UtcMillis> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .and_then(|time| UtcMillis::new(time.timestamp_millis()).ok())
}

const fn invalid_configuration() -> MatrixFailure {
    MatrixFailure::new(
        MatrixOperation::SendStateEvent,
        MatrixFailureKind::InvalidConfiguration,
    )
}

#[cfg(test)]
mod tests;
