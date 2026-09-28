//! 应别的设备请求，重发这台设备自己建的房间密钥。
//!
//! 人的设备因为缺密钥解不开 Agent 的消息时，会经 Olm 发来 `room_keys.request.v1`
//! （设计见 `specs/room-key-recovery/design.md`）。这里核对请求设备由主人交叉签名、此刻在
//! 房间里、房间历史可见性是 `shared` 或 `world_readable`，再把点名的、由这台设备建的会话导出来，
//! 只发回请求的那台设备。任何一条不满足都不回答，只记调试日志，免得被人用来探测。

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use agent_room_protocol_conformance::generated::{
    RoomKeyExport, RoomKeyRequestEvent, RoomKeysEvent,
};
use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use chrono::{SecondsFormat, Utc};
use matrix_sdk::{
    Client, RoomState,
    deserialized_responses::{AlgorithmInfo, EncryptionInfo},
    encryption::{Encryption, identities::Device},
    ruma::{
        OwnedDeviceId, OwnedRoomId, OwnedUserId, RoomId, UserId,
        events::{
            AnyToDeviceEvent, AnyToDeviceEventContent,
            room::{history_visibility::HistoryVisibility, member::MembershipState},
        },
        serde::Raw,
    },
};
use matrix_sdk_base::crypto::{
    CollectStrategy, decrypt_room_key_export, olm::ExportedRoomKey, vodozemac::Curve25519PublicKey,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::{Uuid, Variant};

pub(crate) const ROOM_KEY_REQUEST_EVENT_TYPE: &str =
    "io.github.rainyflash.agentroom.room_keys.request.v1";
pub(crate) const ROOM_KEYS_EVENT_TYPE: &str = "io.github.rainyflash.agentroom.room_keys.v1";
const SCHEMA_VERSION: &str = "1.0";
/// 100 个会话 ID 加上外层字段不到 6 KiB；更大的请求不解析。
const MAX_REQUEST_ENVELOPE_BYTES: usize = 16 * 1_024;
const MAX_REQUESTED_SESSIONS: usize = 100;
/// 每条应答最多带几个密钥，与协议的上限一致。
const KEYS_PER_MESSAGE: usize = 20;
/// 同一台请求设备在同一个房间里，隔多久才再答一次。
const REPEAT_INTERVAL: Duration = Duration::from_mins(10);
/// 这台设备每分钟最多答几次。
const ANSWERS_PER_MINUTE: usize = 20;
const MINUTE: Duration = Duration::from_mins(1);

/// 在客户端上挂好应答者。收到请求后另起任务处理：导出和读回各要做一次 50 万轮 PBKDF2，
/// 不能卡住同步。
pub(crate) fn attach(client: &Client) {
    let budget = Arc::new(Mutex::new(AnswerBudget::default()));
    client.add_event_handler(
        move |raw: Raw<AnyToDeviceEvent>,
              encryption_info: Option<EncryptionInfo>,
              client: Client| {
            let budget = Arc::clone(&budget);
            async move {
                let Some((sender, request)) = parse_request(&raw) else {
                    return;
                };
                let Some(origin) = request_origin(&sender, encryption_info.as_ref()) else {
                    tracing::debug!(
                        requester = %sender,
                        "不回答没经 Olm 加密、或看不出发送设备的房间密钥请求"
                    );
                    return;
                };
                tokio::spawn(async move {
                    if let Err(reason) = answer(&client, &budget, &origin, &request).await {
                        tracing::debug!(
                            room = %request.room_id,
                            requester = %origin.user_id,
                            device = %origin.device_id,
                            reason,
                            "没有重发房间密钥"
                        );
                    }
                });
            }
        },
    );
}

/// 通过了格式检查的请求。
#[derive(Debug, Clone, PartialEq, Eq)]
struct RoomKeyRequest {
    request_id: String,
    room_id: OwnedRoomId,
    session_ids: BTreeSet<String>,
}

/// 由 Olm 确定的请求来源。
#[derive(Debug, Clone, PartialEq, Eq)]
struct RequestOrigin {
    user_id: OwnedUserId,
    device_id: OwnedDeviceId,
    curve25519: String,
}

#[derive(Debug, Deserialize)]
struct ToDeviceEnvelope {
    sender: String,
    content: Value,
}

fn parse_request(raw: &Raw<AnyToDeviceEvent>) -> Option<(OwnedUserId, RoomKeyRequest)> {
    // 先只看类型：别的 to-device 事件（包括大块的房间密钥）不必整个解析。
    let event_type = raw.get_field::<String>("type").ok().flatten()?;
    if event_type != ROOM_KEY_REQUEST_EVENT_TYPE {
        return None;
    }
    let json = raw.json().get();
    if json.len() > MAX_REQUEST_ENVELOPE_BYTES {
        return None;
    }
    let envelope = serde_json::from_str::<ToDeviceEnvelope>(json).ok()?;
    let sender = UserId::parse(&envelope.sender).ok()?;
    let event = serde_json::from_value::<RoomKeyRequestEvent>(envelope.content).ok()?;
    Some((sender, validate_request(event)?))
}

fn validate_request(event: RoomKeyRequestEvent) -> Option<RoomKeyRequest> {
    if event.schema_version != SCHEMA_VERSION
        || event.event_type != ROOM_KEY_REQUEST_EVENT_TYPE
        || !is_uuid_v7(&event.id)
        || event.session_ids.is_empty()
        || event.session_ids.len() > MAX_REQUESTED_SESSIONS
        || !event.session_ids.iter().all(|id| is_key_like(id))
    {
        return None;
    }
    Some(RoomKeyRequest {
        request_id: event.id,
        room_id: RoomId::parse(&event.room_id).ok()?,
        session_ids: event.session_ids.into_iter().collect(),
    })
}

fn is_uuid_v7(value: &str) -> bool {
    Uuid::try_parse(value).is_ok_and(|id| {
        id.get_version_num() == 7
            && id.get_variant() == Variant::RFC4122
            && id.hyphenated().to_string() == value
    })
}

/// Megolm 会话 ID 和 Curve25519/Ed25519 公钥都是 32 字节，写成不补齐的 base64 是 43 个字符。
fn is_key_like(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/')
}

fn request_origin(
    sender: &UserId,
    encryption_info: Option<&EncryptionInfo>,
) -> Option<RequestOrigin> {
    let info = encryption_info?;
    let AlgorithmInfo::OlmV1Curve25519AesSha2 {
        curve25519_public_key_base64,
    } = &info.algorithm_info
    else {
        return None;
    };
    if info.sender != sender {
        return None;
    }
    Some(RequestOrigin {
        user_id: sender.to_owned(),
        device_id: info.sender_device.clone()?,
        curve25519: curve25519_public_key_base64.clone(),
    })
}

/// 只在房间历史对成员开放时回答：这时服务器本来就给成员看加入前的密文，从第 0 条导出不会多给什么。
fn shares_history(visibility: &HistoryVisibility) -> bool {
    matches!(
        visibility,
        HistoryVisibility::Shared | HistoryVisibility::WorldReadable
    )
}

/// 回答的频率：同一台设备在同一个房间 10 分钟一次，这台设备每分钟最多 20 次。
#[derive(Debug, Default)]
struct AnswerBudget {
    last_answer: HashMap<(OwnedUserId, OwnedDeviceId, OwnedRoomId), Instant>,
    recent: VecDeque<Instant>,
}

impl AnswerBudget {
    fn try_take(&mut self, origin: &RequestOrigin, room_id: &RoomId, now: Instant) -> bool {
        while self
            .recent
            .front()
            .is_some_and(|at| now.duration_since(*at) >= MINUTE)
        {
            self.recent.pop_front();
        }
        if self.recent.len() >= ANSWERS_PER_MINUTE {
            return false;
        }
        self.last_answer
            .retain(|_, at| now.duration_since(*at) < REPEAT_INTERVAL);
        let key = (
            origin.user_id.clone(),
            origin.device_id.clone(),
            room_id.to_owned(),
        );
        if self.last_answer.contains_key(&key) {
            return false;
        }
        self.last_answer.insert(key, now);
        self.recent.push_back(now);
        true
    }
}

/// 这台设备自己的两把公钥。
struct OwnKeys {
    curve25519: Curve25519PublicKey,
    ed25519: String,
}

async fn answer(
    client: &Client,
    budget: &Mutex<AnswerBudget>,
    origin: &RequestOrigin,
    request: &RoomKeyRequest,
) -> Result<(), &'static str> {
    let room = client
        .get_room(&request.room_id)
        .filter(|room| room.state() == RoomState::Joined)
        .ok_or("不在这个房间")?;
    if !shares_history(&room.history_visibility_or_default()) {
        return Err("房间历史不对成员开放");
    }
    // 后面要问服务器、做 PBKDF2，先扣频率，免得被一串请求放大。
    let admitted = budget
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .try_take(origin, &request.room_id, Instant::now());
    if !admitted {
        return Err("超出回答频率");
    }
    let member = room
        .get_member(&origin.user_id)
        .await
        .map_err(|_| "读不到房间成员")?
        .ok_or("请求者不是房间成员")?;
    if *member.membership() != MembershipState::Join {
        return Err("请求者此刻不在房间里");
    }
    let encryption = client.encryption();
    let device = requesting_device(&encryption, origin).await?;
    let own = OwnKeys {
        curve25519: encryption
            .curve25519_key()
            .await
            .ok_or("读不到自己的设备密钥")?,
        ed25519: encryption
            .ed25519_key()
            .await
            .ok_or("读不到自己的设备密钥")?,
    };
    let keys = export_own_sessions(&encryption, request, own.curve25519).await?;
    if keys.is_empty() {
        return Err("没有点名的、由这台设备建的会话");
    }
    send_keys(&encryption, &device, request, &own, &keys).await
}

/// 请求设备必须由它的主人交叉签名，而且就是经 Olm 发来请求的那一台。
async fn requesting_device(
    encryption: &Encryption,
    origin: &RequestOrigin,
) -> Result<Device, &'static str> {
    // 先刷新请求者的身份：它可能刚刚才给这台设备签名。
    encryption
        .request_user_identity(&origin.user_id)
        .await
        .map_err(|_| "取不到请求者的加密身份")?;
    let device = encryption
        .get_device(&origin.user_id, &origin.device_id)
        .await
        .map_err(|_| "读不到请求设备")?
        .ok_or("不认识请求设备")?;
    if !device.is_cross_signed_by_owner() {
        return Err("请求设备没有由主人签名");
    }
    if device.curve25519_key().map(|key| key.to_base64()) != Some(origin.curve25519.clone()) {
        return Err("请求设备的密钥与发来请求的不一致");
    }
    Ok(device)
}

/// matrix-sdk 0.18 只能把房间密钥导出成加密文件，所以写到临时目录、再用同一个随机口令读回来。
/// 口令只在内存里，文件读完就随目录删掉；万一没删掉，里面也是加密的。
async fn export_own_sessions(
    encryption: &Encryption,
    request: &RoomKeyRequest,
    own_curve25519: Curve25519PublicKey,
) -> Result<Vec<ExportedRoomKey>, &'static str> {
    let directory = tempfile::tempdir().map_err(|_| "建不了临时目录")?;
    let path = directory.path().join("room-keys.export");
    let passphrase = random_passphrase().ok_or("生成不了导出口令")?;
    let room_id = request.room_id.clone();
    let wanted = request.session_ids.clone();
    encryption
        .export_room_keys(path.clone(), &passphrase, move |session| {
            session.room_id() == &*room_id
                && session.sender_key() == own_curve25519
                && wanted.contains(session.session_id())
        })
        .await
        .map_err(|_| "导出房间密钥失败")?;
    let mut keys = tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&path).ok()?;
        decrypt_room_key_export(file, &passphrase).ok()
    })
    .await
    .ok()
    .flatten()
    .ok_or("读不回导出的房间密钥")?;
    drop(directory);
    // 导出时已经按条件筛过，这里再核一遍：绝不把别的房间或别的设备的会话发出去。
    keys.retain(|key| {
        key.room_id == request.room_id
            && key.sender_key == own_curve25519
            && request.session_ids.contains(&key.session_id)
    });
    Ok(keys)
}

fn random_passphrase() -> Option<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).ok()?;
    Some(STANDARD_NO_PAD.encode(bytes))
}

async fn send_keys(
    encryption: &Encryption,
    device: &Device,
    request: &RoomKeyRequest,
    own: &OwnKeys,
    keys: &[ExportedRoomKey],
) -> Result<(), &'static str> {
    for chunk in keys.chunks(KEYS_PER_MESSAGE) {
        let content = Raw::new(&response_event(request, own, chunk))
            .map_err(|_| "编码应答失败")?
            .cast_unchecked::<AnyToDeviceEventContent>();
        // SDK 用和这台设备之间最新建的 Olm 会话加密，也就是对方刚才发请求时建的那条。
        let failures = encryption
            .encrypt_and_send_raw_to_device(
                vec![device],
                ROOM_KEYS_EVENT_TYPE,
                content,
                CollectStrategy::AllDevices,
            )
            .await
            .map_err(|_| "发送应答失败")?;
        if !failures.is_empty() {
            return Err("发送应答失败");
        }
    }
    tracing::info!(
        room = %request.room_id,
        requester = %device.user_id(),
        device = %device.device_id(),
        sessions = keys.len(),
        "按请求重发了这台设备的房间密钥"
    );
    Ok(())
}

fn response_event(
    request: &RoomKeyRequest,
    own: &OwnKeys,
    keys: &[ExportedRoomKey],
) -> RoomKeysEvent {
    RoomKeysEvent {
        created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        event_type: ROOM_KEYS_EVENT_TYPE.to_owned(),
        id: Uuid::now_v7().to_string(),
        keys: keys
            .iter()
            .map(|key| RoomKeyExport {
                session_id: key.session_id.clone(),
                session_key: key.session_key.to_base64(),
            })
            .collect(),
        request_id: request.request_id.clone(),
        room_id: request.room_id.to_string(),
        schema_version: SCHEMA_VERSION.to_owned(),
        sender_ed25519_key: own.ed25519.clone(),
        sender_key: own.curve25519.to_base64(),
        extensions: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use matrix_sdk::{
        deserialized_responses::{AlgorithmInfo, EncryptionInfo, VerificationState},
        ruma::{
            OwnedDeviceId, RoomId, UserId,
            events::{AnyToDeviceEvent, room::history_visibility::HistoryVisibility},
            serde::Raw,
        },
    };
    use serde_json::json;

    use super::{
        AnswerBudget, MAX_REQUESTED_SESSIONS, ROOM_KEY_REQUEST_EVENT_TYPE, RequestOrigin,
        parse_request, request_origin, shares_history,
    };

    const SESSION_A: &str = "wXgcS1YzXVKqDl1eDfWfxgrUJs1Xp8mSw3XYZzdh0Ew";
    const SESSION_B: &str = "aPq+/7Zk3mX9f1QvJt2cL0yH8sWbNdRe4uFgIoKj5Vw";
    const REQUEST_ID: &str = "01990d9e-8400-7000-8000-000000000101";

    fn request_content() -> serde_json::Value {
        json!({
            "schemaVersion": "1.0",
            "eventType": ROOM_KEY_REQUEST_EVENT_TYPE,
            "id": REQUEST_ID,
            "createdAt": "2026-09-28T14:00:00Z",
            "roomId": "!private:agent-room.example",
            "sessionIds": [SESSION_A, SESSION_B, SESSION_A],
        })
    }

    fn raw_event(event_type: &str, content: &serde_json::Value) -> Raw<AnyToDeviceEvent> {
        Raw::from_json_string(
            json!({
                "sender": "@human:agent-room.example",
                "type": event_type,
                "content": content,
            })
            .to_string(),
        )
        .expect("测试事件 JSON 有效")
    }

    fn olm_info(sender: &str, device: Option<&str>) -> EncryptionInfo {
        EncryptionInfo {
            sender: UserId::parse(sender).expect("测试用户标识有效"),
            sender_device: device.map(OwnedDeviceId::from),
            forwarder: None,
            algorithm_info: AlgorithmInfo::OlmV1Curve25519AesSha2 {
                curve25519_public_key_base64: "curve-key".to_owned(),
            },
            verification_state: VerificationState::Verified,
        }
    }

    #[test]
    fn 合格的请求去重后交出房间和会话() {
        let (sender, request) =
            parse_request(&raw_event(ROOM_KEY_REQUEST_EVENT_TYPE, &request_content()))
                .expect("合格的请求应当通过");

        assert_eq!(sender.as_str(), "@human:agent-room.example");
        assert_eq!(request.request_id, REQUEST_ID);
        assert_eq!(request.room_id.as_str(), "!private:agent-room.example");
        assert_eq!(
            request
                .session_ids
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [SESSION_B, SESSION_A]
        );
    }

    #[test]
    fn 别的类型超大或不合格式的请求一律不理() {
        assert!(parse_request(&raw_event("m.room_key", &request_content())).is_none());

        let mut oversized = request_content();
        oversized["padding"] = json!("x".repeat(16 * 1_024));
        assert!(parse_request(&raw_event(ROOM_KEY_REQUEST_EVENT_TYPE, &oversized)).is_none());

        let mutations: [(&str, serde_json::Value); 7] = [
            ("schemaVersion", json!("2.0")),
            ("id", json!("01990D9E-8400-7000-8000-000000000101")),
            ("id", json!("01990d9e-8400-4000-8000-000000000101")),
            ("roomId", json!("#alias:agent-room.example")),
            ("sessionIds", json!([])),
            ("sessionIds", json!(["not a megolm session id"])),
            (
                "sessionIds",
                json!(vec![SESSION_A; MAX_REQUESTED_SESSIONS + 1]),
            ),
        ];
        for (field, value) in mutations {
            let mut content = request_content();
            content[field] = value;
            assert!(
                parse_request(&raw_event(ROOM_KEY_REQUEST_EVENT_TYPE, &content)).is_none(),
                "{field} 不合格时必须拒绝"
            );
        }
    }

    #[test]
    fn 只认经_olm_送达且能确定发送设备的请求() {
        let sender = UserId::parse("@human:agent-room.example").expect("测试用户标识有效");

        assert!(request_origin(&sender, None).is_none());
        assert!(
            request_origin(&sender, Some(&olm_info("@human:agent-room.example", None))).is_none()
        );
        assert!(
            request_origin(
                &sender,
                Some(&olm_info("@other:agent-room.example", Some("DEVICE")))
            )
            .is_none()
        );
        let mut megolm = olm_info("@human:agent-room.example", Some("DEVICE"));
        megolm.algorithm_info = AlgorithmInfo::MegolmV1AesSha2 {
            curve25519_key: "curve-key".to_owned(),
            sender_claimed_keys: std::collections::BTreeMap::new(),
            session_id: Some(SESSION_A.to_owned()),
        };
        assert!(request_origin(&sender, Some(&megolm)).is_none());

        let origin = request_origin(
            &sender,
            Some(&olm_info("@human:agent-room.example", Some("DEVICE"))),
        )
        .expect("Olm 送达、设备确定的请求应当通过");
        assert_eq!(origin.device_id.as_str(), "DEVICE");
        assert_eq!(origin.curve25519, "curve-key");
    }

    #[test]
    fn 只在房间历史对成员开放时回答() {
        assert!(shares_history(&HistoryVisibility::Shared));
        assert!(shares_history(&HistoryVisibility::WorldReadable));
        assert!(!shares_history(&HistoryVisibility::Joined));
        assert!(!shares_history(&HistoryVisibility::Invited));
    }

    #[test]
    fn 同一设备同一房间十分钟答一次且每分钟最多二十次() {
        let origin = |device: &str| RequestOrigin {
            user_id: UserId::parse("@human:agent-room.example").expect("测试用户标识有效"),
            device_id: OwnedDeviceId::from(device),
            curve25519: "curve-key".to_owned(),
        };
        let room = RoomId::parse("!private:agent-room.example").expect("测试房间标识有效");
        let other_room = RoomId::parse("!other:agent-room.example").expect("测试房间标识有效");
        let start = Instant::now();
        let mut budget = AnswerBudget::default();

        assert!(budget.try_take(&origin("A"), &room, start));
        assert!(!budget.try_take(&origin("A"), &room, start + Duration::from_mins(9)));
        assert!(budget.try_take(&origin("A"), &other_room, start));
        assert!(budget.try_take(&origin("A"), &room, start + Duration::from_mins(10)));

        let mut busy = AnswerBudget::default();
        for index in 0..20 {
            assert!(busy.try_take(&origin(&format!("D{index}")), &room, start));
        }
        assert!(!busy.try_take(&origin("D20"), &room, start + Duration::from_secs(59)));
        assert!(busy.try_take(&origin("D20"), &room, start + Duration::from_mins(1)));
    }
}
