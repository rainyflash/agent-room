pub mod actor_compatibility;
pub mod generated;

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use serde_json::Value;

    use crate::generated::{
        AgentStatusEvent, CapabilityManifest, ErrorEnvelope, HandoffReceiptEvent,
        HandoffRequestEvent, MessagePreviewEvent, MessagePreviewEventV2, MessageRevisionEvent,
        MessageRevisionEventV2, ModerationNoticeEvent, RoomKeyRequestEvent, RoomKeysEvent,
        RoomListingState,
    };

    fn project_path(relative: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative)
    }

    fn fixture_paths(kind: &str) -> Vec<PathBuf> {
        let directory = project_path(&format!("packages/protocol/fixtures/{kind}"));
        let mut paths = fs::read_dir(directory)
            .expect("夹具目录必须存在")
            .map(|entry| entry.expect("夹具目录项必须可读").path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>();
        paths.sort();
        paths
    }

    fn read_json(path: &PathBuf) -> Value {
        let bytes = fs::read(path).expect("夹具必须可读");
        serde_json::from_slice(&bytes).expect("夹具必须是 JSON")
    }

    fn assert_generated_type_deserializes(value: Value) {
        let event_type = value
            .get("eventType")
            .and_then(Value::as_str)
            .and_then(|event_type| event_type.strip_prefix("io.github.rainyflash.agentroom."))
            .map(str::to_owned);
        match event_type.as_deref() {
            Some("message.preview.v1") => {
                deserializes::<MessagePreviewEvent>(value, "消息预览必须符合生成的 Rust 类型");
            }
            Some("message.preview.v2") => {
                deserializes::<MessagePreviewEventV2>(value, "v2 消息预览必须符合生成的 Rust 类型");
            }
            Some("message.revision.v1") => {
                deserializes::<MessageRevisionEvent>(value, "消息修订必须符合生成的 Rust 类型");
            }
            Some("message.revision.v2") => {
                deserializes::<MessageRevisionEventV2>(
                    value,
                    "v2 消息修订必须符合生成的 Rust 类型",
                );
            }
            Some("moderation.notice.v1") => {
                deserializes::<ModerationNoticeEvent>(value, "治理通知必须符合生成的 Rust 类型");
            }
            Some("agent.status.v1") => {
                deserializes::<AgentStatusEvent>(value, "状态事件必须符合生成的 Rust 类型");
            }
            Some("handoff.request.v1") => {
                deserializes::<HandoffRequestEvent>(value, "交付事件必须符合生成的 Rust 类型");
            }
            Some("handoff.receipt.v1") => {
                deserializes::<HandoffReceiptEvent>(value, "交付回执必须符合生成的 Rust 类型");
            }
            Some("room_keys.request.v1") => deserializes::<RoomKeyRequestEvent>(
                value,
                "房间密钥重发请求必须符合生成的 Rust 类型",
            ),
            Some("room_keys.v1") => {
                deserializes::<RoomKeysEvent>(value, "重发的房间密钥必须符合生成的 Rust 类型");
            }
            Some("room.listing.v1") => deserializes::<RoomListingState>(
                value,
                "房间第一次公开的状态必须符合生成的 Rust 类型",
            ),
            _ if value.get("protocolVersions").is_some() => {
                deserializes::<CapabilityManifest>(value, "能力清单必须符合生成的 Rust 类型");
            }
            _ => deserializes::<ErrorEnvelope>(value, "错误信封必须符合生成的 Rust 类型"),
        }
    }

    fn deserializes<T: serde::de::DeserializeOwned>(value: Value, message: &str) {
        serde_json::from_value::<T>(value).expect(message);
    }

    #[test]
    fn rust验证器接受全部正例并能反序列化() {
        let schema = read_json(&project_path(
            "packages/protocol/schema/v1/agent-room.schema.json",
        ));
        assert!(jsonschema::meta::is_valid(&schema), "Schema 元语法必须有效");
        let validator = jsonschema::validator_for(&schema).expect("Schema 必须可编译");

        for path in fixture_paths("valid") {
            let value = read_json(&path);
            assert!(
                validator.is_valid(&value),
                "Rust 验证器拒绝了正例 {}",
                path.display()
            );
            assert_generated_type_deserializes(value);
        }
    }

    #[test]
    fn rust验证器拒绝全部恶意反例() {
        let schema = read_json(&project_path(
            "packages/protocol/schema/v1/agent-room.schema.json",
        ));
        let validator = jsonschema::validator_for(&schema).expect("Schema 必须可编译");

        for path in fixture_paths("invalid") {
            let value = read_json(&path);
            assert!(
                !validator.is_valid(&value),
                "Rust 验证器错误接受了反例 {}",
                path.display()
            );
        }
    }
}
