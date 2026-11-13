//! RUSTSEC-2026-0510：vodozemac 0.11.2 以前，Ed25519 公钥的 base64 解出来是 31 或 33 字节时
//! 直接 panic。别人上传的设备公钥、发给 Agent 的 Olm 消息里都带着这种公钥：服务器上随便哪个
//! 账户都能发一条，让 Agent 一同步就崩，同一条消息下次同步还在，就一直收不到消息。
//! `third_party/vodozemac` 补上了上游的修复。这里照 Agent 真收到的两种内容各解一次，要求返回
//! 错误、不 panic；升到 Matrix SDK 0.19、删掉自带的那一份以后照样该过。

use matrix_sdk_base::crypto::{
    types::{DeviceKeys, events::olm_v1::AnyDecryptedOlmEvent},
    vodozemac::Ed25519PublicKey,
};
use serde_json::{Value, json};

/// 44 个字符、不带填充：长度和带填充的 32 字节公钥一样，过得了长度检查，解出来却是 33 字节。
const TOO_LONG: &str = "b29vb29vb29vb29vb29vb29vb29vb29vb29vb29vb288";
/// 带填充，解出来 31 字节。
const TOO_SHORT: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==";
const VALID: &str = "b29vb29vb29vb29vb29vb29vb29vb29vb29vb29vb28";

fn device_keys(ed25519: &str) -> Value {
    json!({
        "user_id": "@mallory:example.org",
        "device_id": "MALLORY",
        "algorithms": ["m.olm.v1.curve25519-aes-sha2", "m.megolm.v1.aes-sha2"],
        "keys": {
            "curve25519:MALLORY": VALID,
            "ed25519:MALLORY": ed25519,
        },
        "signatures": {},
    })
}

fn decrypted_olm_event(sender_ed25519: &str) -> Value {
    json!({
        "type": "m.dummy",
        "sender": "@mallory:example.org",
        "recipient": "@agent:example.org",
        "keys": { "ed25519": sender_ed25519 },
        "recipient_keys": { "ed25519": VALID },
        "content": {},
    })
}

#[test]
fn malformed_ed25519_keys_are_errors() {
    for key in [TOO_LONG, TOO_SHORT] {
        assert!(Ed25519PublicKey::from_base64(key).is_err(), "{key}");
    }
    assert!(Ed25519PublicKey::from_base64(VALID).is_ok());
}

#[test]
fn uploaded_device_keys_with_a_malformed_signing_key_are_rejected() {
    assert!(serde_json::from_value::<DeviceKeys>(device_keys(VALID)).is_ok());
    for key in [TOO_LONG, TOO_SHORT] {
        assert!(
            serde_json::from_value::<DeviceKeys>(device_keys(key)).is_err(),
            "{key}"
        );
    }
}

#[test]
fn olm_messages_with_a_malformed_sender_key_are_rejected() {
    assert!(serde_json::from_value::<AnyDecryptedOlmEvent>(decrypted_olm_event(VALID)).is_ok());
    for key in [TOO_LONG, TOO_SHORT] {
        let event = serde_json::from_value::<AnyDecryptedOlmEvent>(decrypted_olm_event(key));
        assert!(event.is_err(), "{key}");
    }
}
