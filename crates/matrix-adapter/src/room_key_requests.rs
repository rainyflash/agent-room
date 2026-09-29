//! 缺房间密钥时，请发送那台设备重发它建的会话（设计见 `specs/room-key-recovery/pre-join-history.md`）。
//!
//! Agent 加入房间前的消息用的会话没发给它，它解不开。这里记下解不开的事件，攒几秒按房间、
//! 发送者和发送设备合成一条请求，经 Olm 发给那台设备——对方是 Agent 还是人的网页端、桌面端都行，
//! 协议和第一期一样。收到应答核对来源后导入，并记下新导入的会话，由 Bridge 核心重读用到
//! 它们的隔离消息。
//!
//! SDK 对导入的会话一律判“来源不安全”：它没法证明会话真是那台设备建的。我们导入的会话来自核对
//! 过的应答——经 Olm 从那台设备送来、设备由主人签名、密钥与应答一致，这正是那个证明——所以
//! 重读时按那台设备此刻是否仍由主人签名来判信任。

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use agent_room_protocol_conformance::generated::{RoomKeyRequestEvent, RoomKeysEvent};
use chrono::{SecondsFormat, Utc};
use matrix_sdk::{
    Client,
    deserialized_responses::{
        AlgorithmInfo, DeviceLinkProblem, EncryptionInfo, TimelineEvent, TimelineEventKind,
        UnableToDecryptReason, VerificationLevel, VerificationState,
    },
    encryption::{Encryption, identities::Device},
    ruma::{
        OwnedDeviceId, OwnedRoomId, OwnedUserId, RoomId, UserId,
        events::{AnyToDeviceEvent, AnyToDeviceEventContent},
        serde::Raw,
    },
};
use matrix_sdk_base::crypto::{CollectStrategy, encrypt_room_key_export, olm::ExportedRoomKey};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    room_keys::{
        KEYS_PER_MESSAGE, MAX_REQUESTED_SESSIONS, ROOM_KEY_REQUEST_EVENT_TYPE,
        ROOM_KEYS_EVENT_TYPE, RequestOrigin, SCHEMA_VERSION, is_key_like, random_passphrase,
        request_origin,
    },
    trust::SenderTrustUpgrades,
};

/// 攒这么久再发：进房间时一批事件同时解不开，合成一条请求。
const FLUSH_DELAY: Duration = Duration::from_secs(3);
/// 同一个会话隔多久才再请求一次。
const REQUEST_INTERVAL: Duration = Duration::from_hours(1);
/// 等应答的请求留多久：人的设备可能过几天才上线。
const PENDING_LIFETIME: Duration = Duration::from_hours(7 * 24);
/// 最多记这么多条等应答的请求，多了先丢最早的。
const MAX_PENDING_REQUESTS: usize = 1_000;
/// 最多记这么多个凭应答导入的会话，多了先丢最早的；它们只在导入后马上重读时用得上。
const MAX_VOUCHED_SESSIONS: usize = 10_000;
/// 20 个密钥的应答不到 10 KiB；更大的不解析。
const MAX_ANSWER_ENVELOPE_BYTES: usize = 32 * 1_024;
/// 导入用的临时导出文件只在本机、用完即删，不需要 50 万轮的口令派生。
const IMPORT_EXPORT_ROUNDS: u32 = 1_000;

/// 一条请求发给谁：房间、发送者，以及加密内容里点名的发送设备（没有时发给这个用户的每台设备）。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RequestTarget {
    room: OwnedRoomId,
    sender: OwnedUserId,
    device: Option<OwnedDeviceId>,
}

#[derive(Debug, Clone)]
struct PendingRequest {
    target: RequestTarget,
    session_ids: BTreeSet<String>,
    sent_at: Instant,
}

/// 凭核对过的应答导入的会话：由哪个人的哪台设备重发、那台设备的 Curve25519。
#[derive(Debug, Clone)]
struct VouchedSession {
    user: OwnedUserId,
    device: OwnedDeviceId,
    curve25519: String,
    imported_at: Instant,
}

/// 请求重发、导入应答的状态；每个 Agent 的客户端一份。
#[derive(Debug, Default)]
pub(crate) struct RoomKeyRequester {
    queued: Mutex<BTreeMap<RequestTarget, BTreeSet<String>>>,
    flush_scheduled: AtomicBool,
    requested_at: Mutex<HashMap<(OwnedRoomId, String), Instant>>,
    pending: Mutex<HashMap<String, PendingRequest>>,
    recovered_sessions: Mutex<BTreeSet<(OwnedRoomId, String)>>,
    vouched: Mutex<HashMap<(OwnedRoomId, String), VouchedSession>>,
}

impl RoomKeyRequester {
    /// 记下一个解不开的事件；值得请求的就排进下一批。
    pub(crate) fn note(self: &Arc<Self>, client: &Client, room_id: &RoomId, event: &TimelineEvent) {
        let Some(target_session) = requestable(client.user_id(), room_id, event) else {
            return;
        };
        if !self.first_request_in_interval(room_id, &target_session.1, Instant::now()) {
            return;
        }
        let (target, session_id) = target_session;
        self.enqueue(client, target, [session_id]);
    }

    /// 按存储里记下的发送者和会话直接请求（Bridge 重启后用）；同一会话一小时内只请求一次。
    pub(crate) fn request_sessions(
        self: &Arc<Self>,
        client: &Client,
        room_id: &RoomId,
        sender: OwnedUserId,
        sender_device: Option<OwnedDeviceId>,
        session_ids: &[String],
    ) {
        let now = Instant::now();
        let fresh = session_ids
            .iter()
            .filter(|session_id| is_key_like(session_id))
            .filter(|session_id| self.first_request_in_interval(room_id, session_id, now))
            .cloned()
            .collect::<Vec<_>>();
        if fresh.is_empty() || client.user_id() == Some(sender.as_ref()) {
            return;
        }
        let target = RequestTarget {
            room: room_id.to_owned(),
            sender,
            device: sender_device,
        };
        self.enqueue(client, target, fresh);
    }

    fn enqueue(
        self: &Arc<Self>,
        client: &Client,
        target: RequestTarget,
        session_ids: impl IntoIterator<Item = String>,
    ) {
        self.queued
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(target)
            .or_default()
            .extend(session_ids);
        if !self.flush_scheduled.swap(true, Ordering::AcqRel) {
            let requester = Arc::clone(self);
            let client = client.clone();
            tokio::spawn(async move {
                tokio::time::sleep(FLUSH_DELAY).await;
                requester.flush_scheduled.store(false, Ordering::Release);
                requester.flush(&client).await;
            });
        }
    }

    fn first_request_in_interval(&self, room_id: &RoomId, session_id: &str, now: Instant) -> bool {
        let mut requested = self
            .requested_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        requested.retain(|_, at| now.duration_since(*at) < REQUEST_INTERVAL);
        let key = (room_id.to_owned(), session_id.to_owned());
        if requested.contains_key(&key) {
            return false;
        }
        requested.insert(key, now);
        true
    }

    /// 自上次取走以来导入的、别人重发的会话。
    pub(crate) fn take_recovered_sessions(&self) -> Vec<(OwnedRoomId, String)> {
        std::mem::take(
            &mut *self
                .recovered_sessions
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
        .into_iter()
        .collect()
    }

    async fn flush(&self, client: &Client) {
        let batches =
            std::mem::take(&mut *self.queued.lock().unwrap_or_else(PoisonError::into_inner));
        let encryption = client.encryption();
        for (target, session_ids) in batches {
            if let Err(reason) = self.request(&encryption, &target, session_ids).await {
                tracing::debug!(
                    room = %target.room,
                    sender = %target.sender,
                    reason,
                    "没有请求重发房间密钥"
                );
            }
        }
    }

    async fn request(
        &self,
        encryption: &Encryption,
        target: &RequestTarget,
        session_ids: BTreeSet<String>,
    ) -> Result<(), &'static str> {
        let devices = target_devices(encryption, target).await?;
        let ids = session_ids.into_iter().collect::<Vec<_>>();
        for chunk in ids.chunks(MAX_REQUESTED_SESSIONS) {
            let event = request_event(&target.room, chunk);
            self.remember(&event.id, target, chunk);
            let content = Raw::new(&event)
                .map_err(|_| "编码请求失败")?
                .cast_unchecked::<AnyToDeviceEventContent>();
            // 和这台设备之间没有能用的 Olm 会话时，SDK 先领对方的一次性密钥建一条。
            let failures = encryption
                .encrypt_and_send_raw_to_device(
                    devices.iter().collect(),
                    ROOM_KEY_REQUEST_EVENT_TYPE,
                    content,
                    CollectStrategy::AllDevices,
                )
                .await
                .map_err(|_| "发送请求失败")?;
            if failures.len() == devices.len() {
                return Err("请求没能送到任何一台设备");
            }
        }
        tracing::info!(
            room = %target.room,
            sender = %target.sender,
            sessions = ids.len(),
            "请发送设备重发解不开的消息的房间密钥"
        );
        Ok(())
    }

    fn remember(&self, request_id: &str, target: &RequestTarget, session_ids: &[String]) {
        let now = Instant::now();
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        pending.retain(|_, request| now.duration_since(request.sent_at) < PENDING_LIFETIME);
        if pending.len() >= MAX_PENDING_REQUESTS
            && let Some(oldest) = pending
                .iter()
                .min_by_key(|(_, request)| request.sent_at)
                .map(|(id, _)| id.clone())
        {
            pending.remove(&oldest);
        }
        pending.insert(
            request_id.to_owned(),
            PendingRequest {
                target: target.clone(),
                session_ids: session_ids.iter().cloned().collect(),
                sent_at: now,
            },
        );
    }

    /// 核对一条应答，导入其中的房间密钥。返回导入的会话数。
    pub(crate) async fn accept(
        &self,
        client: &Client,
        raw: &Raw<AnyToDeviceEvent>,
        encryption_info: Option<&EncryptionInfo>,
    ) -> Result<usize, &'static str> {
        let (sender, answer) = parse_answer(raw).ok_or("不是合格的房间密钥应答")?;
        let origin = request_origin(&sender, encryption_info).ok_or("应答没经 Olm 加密")?;
        let request = self.matching_request(&answer, &origin)?;
        let encryption = client.encryption();
        verify_answering_device(&encryption, &origin, &answer).await?;
        let imported = import(&encryption, &answer).await?;
        self.vouch(&answer, &origin);
        self.settle(&answer);
        tracing::info!(
            room = %request.target.room,
            sender = %origin.user_id,
            device = %origin.device_id,
            sessions = imported,
            "导入了对方重发的房间密钥"
        );
        Ok(imported)
    }

    /// 应答必须对得上我们发过、还没过期的请求：同一个人、同一台设备、同一个房间、点名过的会话。
    fn matching_request(
        &self,
        answer: &RoomKeysEvent,
        origin: &RequestOrigin,
    ) -> Result<PendingRequest, &'static str> {
        let pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let request = pending
            .get(&answer.request_id)
            .filter(|request| request.sent_at.elapsed() < PENDING_LIFETIME)
            .ok_or("不是我们发过的请求")?;
        let target = &request.target;
        if target.sender != origin.user_id
            || target
                .device
                .as_ref()
                .is_some_and(|device| *device != origin.device_id)
            || target.room.as_str() != answer.room_id
            || answer.sender_key != origin.curve25519
            || !answer
                .keys
                .iter()
                .all(|key| request.session_ids.contains(&key.session_id))
        {
            return Err("应答和请求对不上");
        }
        Ok(request.clone())
    }

    /// 记下这些会话是哪台设备经核对过的应答重发的；已经有的会话（导入时被 SDK 丢弃）也记，
    /// Bridge 重启后重新请求、再次应答时靠它判信任。
    fn vouch(&self, answer: &RoomKeysEvent, origin: &RequestOrigin) {
        let Ok(room_id) = RoomId::parse(&answer.room_id) else {
            return;
        };
        let now = Instant::now();
        let mut vouched = self.vouched.lock().unwrap_or_else(PoisonError::into_inner);
        vouched.retain(|_, session| now.duration_since(session.imported_at) < PENDING_LIFETIME);
        for key in &answer.keys {
            if vouched.len() >= MAX_VOUCHED_SESSIONS
                && let Some(oldest) = vouched
                    .iter()
                    .min_by_key(|(_, session)| session.imported_at)
                    .map(|(key, _)| key.clone())
            {
                vouched.remove(&oldest);
            }
            vouched.insert(
                (room_id.clone(), key.session_id.clone()),
                VouchedSession {
                    user: origin.user_id.clone(),
                    device: origin.device_id.clone(),
                    curve25519: origin.curve25519.clone(),
                    imported_at: now,
                },
            );
        }
    }

    /// 这个事件用的会话是凭核对过的应答导入的、而且由事件发送者那台设备建：返回那台设备。
    fn vouching_device(
        &self,
        room_id: &RoomId,
        event: &TimelineEvent,
    ) -> Option<(OwnedUserId, OwnedDeviceId)> {
        let info = event.encryption_info()?;
        let AlgorithmInfo::MegolmV1AesSha2 {
            curve25519_key,
            session_id: Some(session_id),
            ..
        } = &info.algorithm_info
        else {
            return None;
        };
        let vouched = self.vouched.lock().unwrap_or_else(PoisonError::into_inner);
        let session = vouched.get(&(room_id.to_owned(), session_id.clone()))?;
        (session.user == info.sender && session.curve25519 == *curve25519_key)
            .then(|| (session.user.clone(), session.device.clone()))
    }

    /// 重读时用：凭核对过的应答导入的会话解开的事件，SDK 判“来源不安全”；重发它的设备此刻
    /// 仍由主人签名、主人的身份也没变过，就当作可信。
    pub(crate) async fn trust_vouched_sessions(
        &self,
        client: &Client,
        room_id: &RoomId,
        events: &[TimelineEvent],
        upgrades: &mut SenderTrustUpgrades,
    ) {
        let encryption = client.encryption();
        for event in events {
            let insecure_source = event.encryption_info().is_some_and(|info| {
                info.verification_state
                    == VerificationState::Unverified(VerificationLevel::None(
                        DeviceLinkProblem::InsecureSource,
                    ))
            });
            let (Some(event_id), true) = (event.event_id(), insecure_source) else {
                continue;
            };
            let Some((user, device)) = self.vouching_device(room_id, event) else {
                continue;
            };
            if owner_signed(&encryption, &user, &device).await {
                upgrades.insert(event_id);
            }
        }
    }

    /// 导入成功：点名的会话不再等，房间记为有新密钥。
    fn settle(&self, answer: &RoomKeysEvent) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(request) = pending.get_mut(&answer.request_id) {
            for key in &answer.keys {
                request.session_ids.remove(&key.session_id);
            }
            if request.session_ids.is_empty() {
                pending.remove(&answer.request_id);
            }
        }
        drop(pending);
        if let Ok(room_id) = RoomId::parse(&answer.room_id) {
            self.recovered_sessions
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(
                    answer
                        .keys
                        .iter()
                        .map(|key| (room_id.clone(), key.session_id.clone())),
                );
        }
    }
}

/// 这个事件值得请求吗：缺会话或缺消息序号、不是自己发的，并且带着会话 ID。
fn requestable(
    own_user_id: Option<&UserId>,
    room_id: &RoomId,
    event: &TimelineEvent,
) -> Option<(RequestTarget, String)> {
    let TimelineEventKind::UnableToDecrypt { event, utd_info } = &event.kind else {
        return None;
    };
    if !matches!(
        utd_info.reason,
        UnableToDecryptReason::MissingMegolmSession { .. }
            | UnableToDecryptReason::UnknownMegolmMessageIndex
    ) {
        return None;
    }
    let session_id = utd_info.session_id.clone().filter(|id| is_key_like(id))?;
    let sender = event.get_field::<OwnedUserId>("sender").ok().flatten()?;
    // 自己别的设备建的会话（比如换过设备），那台设备多半已经不在了，不问。
    if own_user_id == Some(sender.as_ref()) {
        return None;
    }
    let device_id = event
        .get_field::<Value>("content")
        .ok()
        .flatten()
        .and_then(|content| content.get("device_id")?.as_str().map(OwnedDeviceId::from));
    Some((
        RequestTarget {
            room: room_id.to_owned(),
            sender,
            device: device_id,
        },
        session_id,
    ))
}

/// 请求发给哪些设备：点名的那台，或者这个用户的每台。先刷新一次对方的设备列表。
async fn target_devices(
    encryption: &Encryption,
    target: &RequestTarget,
) -> Result<Vec<Device>, &'static str> {
    encryption
        .request_user_identity(&target.sender)
        .await
        .map_err(|_| "取不到发送者的加密身份")?;
    let devices = match &target.device {
        Some(device_id) => encryption
            .get_device(&target.sender, device_id)
            .await
            .map_err(|_| "读不到发送设备")?
            .into_iter()
            .collect::<Vec<_>>(),
        None => encryption
            .get_user_devices(&target.sender)
            .await
            .map_err(|_| "读不到发送者的设备")?
            .devices()
            .collect(),
    };
    if devices.is_empty() {
        return Err("发送设备已经不在了");
    }
    Ok(devices)
}

fn request_event(room_id: &RoomId, session_ids: &[String]) -> RoomKeyRequestEvent {
    RoomKeyRequestEvent {
        created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        event_type: ROOM_KEY_REQUEST_EVENT_TYPE.to_owned(),
        id: Uuid::now_v7().to_string(),
        room_id: room_id.to_string(),
        schema_version: SCHEMA_VERSION.to_owned(),
        session_ids: session_ids.to_vec(),
        extensions: BTreeMap::new(),
    }
}

#[derive(Debug, Deserialize)]
struct ToDeviceEnvelope {
    sender: String,
    content: Value,
}

fn parse_answer(raw: &Raw<AnyToDeviceEvent>) -> Option<(OwnedUserId, RoomKeysEvent)> {
    if raw.get_field::<String>("type").ok().flatten()? != ROOM_KEYS_EVENT_TYPE {
        return None;
    }
    let json = raw.json().get();
    if json.len() > MAX_ANSWER_ENVELOPE_BYTES {
        return None;
    }
    let envelope = serde_json::from_str::<ToDeviceEnvelope>(json).ok()?;
    let sender = OwnedUserId::try_from(envelope.sender).ok()?;
    let answer = serde_json::from_value::<RoomKeysEvent>(envelope.content).ok()?;
    valid_answer(&answer).then_some((sender, answer))
}

fn valid_answer(answer: &RoomKeysEvent) -> bool {
    answer.schema_version == SCHEMA_VERSION
        && answer.event_type == ROOM_KEYS_EVENT_TYPE
        && !answer.keys.is_empty()
        && answer.keys.len() <= KEYS_PER_MESSAGE
        && is_key_like(&answer.sender_key)
        && is_key_like(&answer.sender_ed25519_key)
        && answer.keys.iter().all(|key| {
            is_key_like(&key.session_id)
                && (16..=1_024).contains(&key.session_key.len())
                && key
                    .session_key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
        })
}

/// 应答的设备必须由主人签名，两把公钥也要和应答里的一致：它就是这些会话的创建者。
async fn verify_answering_device(
    encryption: &Encryption,
    origin: &RequestOrigin,
    answer: &RoomKeysEvent,
) -> Result<(), &'static str> {
    let device = encryption
        .get_device(&origin.user_id, &origin.device_id)
        .await
        .map_err(|_| "读不到应答设备")?
        .ok_or("不认识应答设备")?;
    if !device.is_cross_signed_by_owner() {
        return Err("应答设备没有由主人签名");
    }
    if device
        .curve25519_key()
        .map(|key| key.to_base64())
        .as_deref()
        != Some(&answer.sender_key)
        || device.ed25519_key().map(|key| key.to_base64()).as_deref()
            != Some(&answer.sender_ed25519_key)
    {
        return Err("应答设备的密钥与应答里的不一致");
    }
    Ok(())
}

/// 这台设备此刻由主人签名，主人的身份也没在核对过之后换过（和同步时判可信的标准一致）。
async fn owner_signed(encryption: &Encryption, user: &UserId, device: &OwnedDeviceId) -> bool {
    let signed = encryption
        .get_device(user, device)
        .await
        .ok()
        .flatten()
        .is_some_and(|device| device.is_cross_signed_by_owner());
    signed
        && !encryption
            .get_user_identity(user)
            .await
            .ok()
            .flatten()
            .is_some_and(|identity| identity.has_verification_violation())
}

/// matrix-sdk 0.18 只能从加密导出文件导入房间密钥：拼回导出格式、写进临时目录、导入后随目录删掉。
async fn import(encryption: &Encryption, answer: &RoomKeysEvent) -> Result<usize, &'static str> {
    let keys = answer
        .keys
        .iter()
        .map(|key| {
            serde_json::from_value::<ExportedRoomKey>(json!({
                "algorithm": "m.megolm.v1.aes-sha2",
                "room_id": answer.room_id,
                "sender_key": answer.sender_key,
                "session_id": key.session_id,
                "session_key": key.session_key,
                "sender_claimed_keys": { "ed25519": answer.sender_ed25519_key },
                "forwarding_curve25519_key_chain": [],
            }))
            .map_err(|_| "应答里的密钥拼不回导出格式")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let passphrase = random_passphrase().ok_or("生成不了导入口令")?;
    let export = encrypt_room_key_export(&keys, &passphrase, IMPORT_EXPORT_ROUNDS)
        .map_err(|_| "写不成导出格式")?;
    let directory = tempfile::tempdir().map_err(|_| "建不了临时目录")?;
    let path = directory.path().join("room-keys.import");
    std::fs::write(&path, export).map_err(|_| "写不了临时导出文件")?;
    let result = encryption
        .import_room_keys(path, &passphrase)
        .await
        .map_err(|_| "导入房间密钥失败")?;
    drop(directory);
    Ok(result.imported_count)
}

#[cfg(test)]
mod tests;
