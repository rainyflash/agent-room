use std::fmt;

use agent_room_application::ports::{MatrixEventType, MatrixUserId, PortFuture};
use agent_room_bridge_core::handoffs::{
    DecryptedHandoffToDeviceEvent, EncryptedHandoffToDeviceEventSource,
    EncryptedHandoffToDeviceGateway, EncryptedHandoffToDeviceRequest, HANDOFF_RECEIPT_EVENT_TYPE,
    HANDOFF_REQUEST_EVENT_TYPE, HandoffTransportFailure, HandoffTransportFailureKind,
};
use matrix_sdk::{
    Client,
    deserialized_responses::{AlgorithmInfo, EncryptionInfo},
    ruma::{
        OwnedDeviceId, UserId,
        events::{AnyToDeviceEvent, AnyToDeviceEventContent},
        serde::Raw,
    },
};
use matrix_sdk_base::crypto::CollectStrategy;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{Mutex, mpsc};

const HANDOFF_INBOX_CAPACITY: usize = 256;
const MAX_TO_DEVICE_ENVELOPE_BYTES: usize = 72 * 1_024;

type InboundHandoff = Result<DecryptedHandoffToDeviceEvent, HandoffTransportFailure>;

/// 复用同一个 Matrix SDK 客户端完成加密交付的收发。
///
/// 接收端使用有界单消费者队列。队列满时 SDK 同步会施加背压，不会为了保持表面在线而
/// 静默丢掉一次性上下文交付命令。
pub struct MatrixSdkHandoffGateway {
    client: Client,
    inbound: Mutex<mpsc::Receiver<InboundHandoff>>,
}

impl fmt::Debug for MatrixSdkHandoffGateway {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatrixSdkHandoffGateway")
            .finish_non_exhaustive()
    }
}

impl MatrixSdkHandoffGateway {
    pub(crate) fn attach(client: Client) -> Self {
        let (sender, receiver) = mpsc::channel(HANDOFF_INBOX_CAPACITY);
        register_inbound_handler(&client, sender);
        Self {
            client,
            inbound: Mutex::new(receiver),
        }
    }

    async fn send_encrypted(
        &self,
        request: &EncryptedHandoffToDeviceRequest,
    ) -> Result<(), HandoffTransportFailure> {
        let target_user = UserId::parse(request.target().matrix_user_id().as_str())
            .map_err(|_| transport_failure(HandoffTransportFailureKind::Internal))?;
        let target_device = OwnedDeviceId::from(request.target().matrix_device_id().as_str());
        let encryption = self.client.encryption();

        // 主动刷新设备密钥，避免把控制平面给出的精确目标映射到 SDK 的陈旧设备缓存。
        encryption
            .request_user_identity(&target_user)
            .await
            .map_err(|_| transport_failure(HandoffTransportFailureKind::Unavailable))?;
        let device = encryption
            .get_device(&target_user, &target_device)
            .await
            .map_err(|_| transport_failure(HandoffTransportFailureKind::Internal))?
            .ok_or_else(|| transport_failure(HandoffTransportFailureKind::Rejected))?;
        let content = Raw::new(request.event().content())
            .map_err(|_| transport_failure(HandoffTransportFailureKind::Internal))?
            .cast_unchecked::<AnyToDeviceEventContent>();

        // 该实验 API 自己生成底层 HTTP transaction id，无法注入领域事务标识。
        // 因此端到端幂等由已签名 handoffId 保证；任何设备级发送失败都按未知提交处理。
        let failures = encryption
            .encrypt_and_send_raw_to_device(
                vec![&device],
                request.event().event_type().as_str(),
                content,
                CollectStrategy::AllDevices,
            )
            .await
            .map_err(|_| transport_failure(HandoffTransportFailureKind::Unavailable))?;
        if failures.is_empty() {
            Ok(())
        } else {
            Err(transport_failure(
                HandoffTransportFailureKind::UnknownCommit,
            ))
        }
    }
}

impl EncryptedHandoffToDeviceGateway for MatrixSdkHandoffGateway {
    fn send<'a>(
        &'a self,
        request: &'a EncryptedHandoffToDeviceRequest,
    ) -> PortFuture<'a, Result<(), HandoffTransportFailure>> {
        Box::pin(self.send_encrypted(request))
    }
}

impl EncryptedHandoffToDeviceEventSource for MatrixSdkHandoffGateway {
    fn receive(&self) -> PortFuture<'_, InboundHandoff> {
        Box::pin(async move {
            self.inbound
                .lock()
                .await
                .recv()
                .await
                .ok_or_else(|| transport_failure(HandoffTransportFailureKind::Unavailable))?
        })
    }
}

fn register_inbound_handler(client: &Client, sender: mpsc::Sender<InboundHandoff>) {
    client.add_event_handler(
        move |raw: Raw<AnyToDeviceEvent>, encryption_info: Option<EncryptionInfo>| {
            let sender = sender.clone();
            async move {
                let Some(candidate) = parse_candidate(&raw) else {
                    return;
                };
                let event = validate_decrypted_event(candidate, encryption_info.as_ref());
                let _ = sender.send(event).await;
            }
        },
    );
}

fn parse_candidate(raw: &Raw<AnyToDeviceEvent>) -> Option<ToDeviceEnvelope> {
    let json = raw.json().get();
    if json.len() > MAX_TO_DEVICE_ENVELOPE_BYTES {
        return None;
    }
    let value = serde_json::from_str::<Value>(json).ok()?;
    let event_type = value.get("type")?.as_str()?;
    if !matches!(
        event_type,
        HANDOFF_REQUEST_EVENT_TYPE | HANDOFF_RECEIPT_EVENT_TYPE
    ) {
        return None;
    }
    serde_json::from_value(value).ok()
}

fn validate_decrypted_event(
    envelope: ToDeviceEnvelope,
    encryption_info: Option<&EncryptionInfo>,
) -> InboundHandoff {
    let Some(encryption_info) = encryption_info else {
        return Err(transport_failure(HandoffTransportFailureKind::Rejected));
    };
    if !matches!(
        encryption_info.algorithm_info,
        AlgorithmInfo::OlmV1Curve25519AesSha2 { .. }
    ) || encryption_info.sender.as_str() != envelope.sender
        || encryption_info.sender_device.is_none()
    {
        return Err(transport_failure(HandoffTransportFailureKind::Rejected));
    }
    let sender = MatrixUserId::new(envelope.sender)
        .map_err(|_| transport_failure(HandoffTransportFailureKind::Rejected))?;
    let event_type = MatrixEventType::new(envelope.event_type)
        .map_err(|_| transport_failure(HandoffTransportFailureKind::Rejected))?;
    DecryptedHandoffToDeviceEvent::new(sender, event_type, envelope.content)
        .map_err(|_| transport_failure(HandoffTransportFailureKind::Rejected))
}

const fn transport_failure(kind: HandoffTransportFailureKind) -> HandoffTransportFailure {
    HandoffTransportFailure::new(kind)
}

#[derive(Debug, Deserialize)]
struct ToDeviceEnvelope {
    sender: String,
    #[serde(rename = "type")]
    event_type: String,
    content: Value,
}

#[cfg(test)]
mod tests {
    use matrix_sdk::{
        deserialized_responses::{AlgorithmInfo, EncryptionInfo, VerificationState},
        ruma::{OwnedDeviceId, UserId, events::AnyToDeviceEvent, serde::Raw},
    };

    use super::{
        HANDOFF_REQUEST_EVENT_TYPE, HandoffTransportFailureKind, parse_candidate,
        validate_decrypted_event,
    };

    #[test]
    fn 只接收上下文交付协议事件() {
        let unrelated = raw_event("m.room_key", &serde_json::json!({"session_id":"secret"}));
        let handoff = raw_event(
            HANDOFF_REQUEST_EVENT_TYPE,
            &serde_json::json!({"schemaVersion":"1.0"}),
        );

        assert!(parse_candidate(&unrelated).is_none());
        let parsed = parse_candidate(&handoff).expect("交付事件应进入加密校验");
        assert_eq!(parsed.event_type, HANDOFF_REQUEST_EVENT_TYPE);
    }

    #[test]
    fn 超大_to_device_信封在反序列化前被丢弃() {
        let oversized = raw_event(
            HANDOFF_REQUEST_EVENT_TYPE,
            &serde_json::json!({"body":"x".repeat(72 * 1_024)}),
        );

        assert!(parse_candidate(&oversized).is_none());
    }

    #[test]
    fn 明文交付被拒绝而_olm_交付进入核心验证() {
        let raw = raw_event(
            HANDOFF_REQUEST_EVENT_TYPE,
            &serde_json::json!({"schemaVersion":"1.0"}),
        );
        let rejected =
            validate_decrypted_event(parse_candidate(&raw).expect("交付事件应进入加密校验"), None)
                .expect_err("明文交付必须拒绝");
        assert_eq!(rejected.kind(), HandoffTransportFailureKind::Rejected);

        let accepted = validate_decrypted_event(
            parse_candidate(&raw).expect("交付事件应进入加密校验"),
            Some(&olm_encryption_info()),
        )
        .expect("Olm 交付应进入核心协议验证");
        assert_eq!(accepted.event_type().as_str(), HANDOFF_REQUEST_EVENT_TYPE);
    }

    fn raw_event(event_type: &str, content: &serde_json::Value) -> Raw<AnyToDeviceEvent> {
        Raw::from_json_string(
            serde_json::json!({
                "sender": "@agent:example.org",
                "type": event_type,
                "content": content,
            })
            .to_string(),
        )
        .expect("测试事件 JSON 有效")
    }

    fn olm_encryption_info() -> EncryptionInfo {
        EncryptionInfo {
            sender: UserId::parse("@agent:example.org").expect("测试用户标识有效"),
            sender_device: Some(OwnedDeviceId::from("AGENT_DEVICE")),
            forwarder: None,
            algorithm_info: AlgorithmInfo::OlmV1Curve25519AesSha2 {
                curve25519_public_key_base64: "curve-key".to_owned(),
            },
            verification_state: VerificationState::Verified,
        }
    }
}
