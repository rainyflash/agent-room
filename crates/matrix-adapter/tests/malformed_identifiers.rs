//! RUSTSEC-2026-0588、RUSTSEC-2026-0589：ruma-identifiers-validation 0.13.0、0.14.0 以前，
//! 校验 `mxc://` 地址和密钥 ID 时把下标截断成 `u8`。服务器名超过 249 字节的 `mxc://`
//! 地址一取服务器名、媒体 ID 就 panic；算法那一段超过 255 字节又带非 ASCII 字符的密钥 ID
//! 一解析就 panic。密钥 ID 在别人上传的设备公钥和签名里：服务器上随便哪个账户都能上传一份，
//! Agent 查他的设备时就崩。
//!
//! `third_party/ruma-identifiers-validation` 补上了，太长的一律当不合法；上游的修复则接受
//! 合规的长服务器名和长算法名。所以这里只要求不 panic、不管解没解出来，升到 Matrix SDK
//! 0.19、删掉自带的那一份以后照样该过。

use matrix_sdk::ruma::{MxcUri, OwnedDeviceKeyId};
use matrix_sdk_base::crypto::types::DeviceKeys;
use serde_json::{Value, json};

const VALID: &str = "b29vb29vb29vb29vb29vb29vb29vb29vb29vb29vb28";

/// 冒号在第 300 字节，截断成 `u8` 是 44：旧版从 45 切，正好切在两字节的 "é" 中间。
fn long_non_ascii_key_id() -> String {
    format!("{}:MALLORY", "é".repeat(150))
}

fn device_keys(extra_key_id: &str) -> Value {
    json!({
        "user_id": "@mallory:example.org",
        "device_id": "MALLORY",
        "algorithms": ["m.olm.v1.curve25519-aes-sha2", "m.megolm.v1.aes-sha2"],
        "keys": {
            "curve25519:MALLORY": VALID,
            "ed25519:MALLORY": VALID,
            extra_key_id: VALID,
        },
        "signatures": {
            "@mallory:example.org": { extra_key_id: VALID },
        },
    })
}

#[test]
fn long_non_ascii_key_ids_do_not_panic() {
    let key_id = long_non_ascii_key_id();
    let _ = OwnedDeviceKeyId::try_from(key_id.as_str());
    assert!(OwnedDeviceKeyId::try_from("ed25519:MALLORY").is_ok());
}

#[test]
fn uploaded_device_keys_with_a_long_non_ascii_key_id_do_not_panic() {
    assert!(serde_json::from_value::<DeviceKeys>(device_keys("ed25519:OTHER")).is_ok());
    let _ = serde_json::from_value::<DeviceKeys>(device_keys(&long_non_ascii_key_id()));
}

#[test]
fn mxc_uris_with_long_server_names_do_not_panic() {
    for length in [249, 250, 255, 256, 261] {
        let uri = format!("mxc://{}/media", "a".repeat(length));
        let mxc = <&MxcUri>::from(uri.as_str());
        let _ = mxc.is_valid();
        let _ = mxc.parts();
        let _ = mxc.server_name();
        let _ = mxc.media_id();
    }
    let mxc = <&MxcUri>::from("mxc://example.org/media");
    assert_eq!(mxc.media_id(), Ok("media"));
}
