//! 房间第一次公开的那一刻（`room.listing.v1`，设计见 `specs/public-rooms/design.md`）。
//!
//! 加密的私密房间改成公开以后，这一刻以后进来的人不该读到以前私下说的话。房间密钥里没有时间，
//! 分不清哪个会话是公开以前建的，同一个会话还可能跨过这一刻接着用，所以一刀切：这一刻以后
//! 进来的人，别人不给他们重发房间密钥，他们自己也不去要。他们进来以后别人发的消息照常解得开，
//! 发送时就把密钥分给了在场的每个人。
//!
//! 这条状态只会让重发变少，写错了也不会多给谁，所以不核对是谁写的（只有应用服务够得着）。

use chrono::DateTime;
use matrix_sdk::{
    Room,
    deserialized_responses::RawAnySyncOrStrippedState,
    ruma::{MilliSecondsSinceUnixEpoch, UInt, UserId, events::StateEventType},
};
use serde_json::Value;

use agent_room_protocol_conformance::generated::RoomListingState;

pub(crate) const ROOM_LISTING_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.room.listing.v1";

/// 这个人此刻的成员事件是不是在房间第一次公开以后。房间没公开过返回 `false`。
///
/// 成员事件改过（比如改了名字）也按最新那条的时间算：宁可少给，不多给。
pub(crate) async fn joined_after_listing(
    room: &Room,
    user_id: &UserId,
) -> Result<bool, &'static str> {
    let Some(listed_at) = first_listed_at(room).await? else {
        return Ok(false);
    };
    let member = room
        .get_member_no_sync(user_id)
        .await
        .map_err(|_| "读不到房间成员")?
        .ok_or("不是房间成员")?;
    let joined_at = member
        .event()
        .origin_server_ts()
        .ok_or("看不出什么时候进的房间")?;
    Ok(joined_at >= listed_at)
}

async fn first_listed_at(room: &Room) -> Result<Option<MilliSecondsSinceUnixEpoch>, &'static str> {
    let Some(raw) = room
        .get_state_event(StateEventType::from(ROOM_LISTING_EVENT_TYPE), "")
        .await
        .map_err(|_| "读不到房间状态")?
    else {
        return Ok(None);
    };
    let content = match raw {
        RawAnySyncOrStrippedState::Sync(raw) => raw.get_field::<Value>("content"),
        RawAnySyncOrStrippedState::Stripped(raw) => raw.get_field::<Value>("content"),
    };
    // 读不出来的（被撤回、格式不对）当成一开始就公开了：一个都不重发。
    let listed_at = content
        .ok()
        .flatten()
        .and_then(listed_at_millis)
        .unwrap_or(MilliSecondsSinceUnixEpoch(UInt::MIN));
    Ok(Some(listed_at))
}

fn listed_at_millis(content: Value) -> Option<MilliSecondsSinceUnixEpoch> {
    let state = serde_json::from_value::<RoomListingState>(content).ok()?;
    let at = DateTime::parse_from_rfc3339(&state.first_listed_at).ok()?;
    UInt::try_from(at.timestamp_millis())
        .ok()
        .map(MilliSecondsSinceUnixEpoch)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn 第一次公开的时间按毫秒读() {
        let at = listed_at_millis(json!({
            "schemaVersion": "1.0",
            "eventType": ROOM_LISTING_EVENT_TYPE,
            "firstListedAt": "2026-11-02T08:00:00.250Z",
        }));
        assert_eq!(
            at,
            Some(MilliSecondsSinceUnixEpoch(UInt::new_saturating(
                1_793_606_400_250
            )))
        );
    }

    #[test]
    fn 读不出来的时间不算() {
        for content in [
            json!({}),
            json!({ "schemaVersion": "1.0", "eventType": ROOM_LISTING_EVENT_TYPE }),
            json!({
                "schemaVersion": "1.0",
                "eventType": ROOM_LISTING_EVENT_TYPE,
                "firstListedAt": "昨天",
            }),
            json!({
                "schemaVersion": "1.0",
                "eventType": ROOM_LISTING_EVENT_TYPE,
                "firstListedAt": "1969-12-31T23:59:59Z",
            }),
        ] {
            assert_eq!(listed_at_millis(content.clone()), None, "{content}");
        }
    }
}
