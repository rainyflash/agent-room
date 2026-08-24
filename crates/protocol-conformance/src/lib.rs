pub mod generated;

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use serde_json::Value;

    use crate::generated::{
        AgentStatusEvent, CapabilityManifest, ErrorEnvelope, HandoffReceiptEvent,
        HandoffRequestEvent, MessagePreviewEvent, MessageRevisionEvent,
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
        if value.get("eventType") == Some(&Value::String("org.agentroom.message.preview.v1".into()))
        {
            serde_json::from_value::<MessagePreviewEvent>(value)
                .expect("消息预览必须符合生成的 Rust 类型");
            return;
        }
        if value.get("eventType")
            == Some(&Value::String("org.agentroom.message.revision.v1".into()))
        {
            serde_json::from_value::<MessageRevisionEvent>(value)
                .expect("消息修订必须符合生成的 Rust 类型");
            return;
        }
        if value.get("eventType") == Some(&Value::String("org.agentroom.agent.status.v1".into())) {
            serde_json::from_value::<AgentStatusEvent>(value)
                .expect("状态事件必须符合生成的 Rust 类型");
            return;
        }
        if value.get("eventType") == Some(&Value::String("org.agentroom.handoff.request.v1".into()))
        {
            serde_json::from_value::<HandoffRequestEvent>(value)
                .expect("交付事件必须符合生成的 Rust 类型");
            return;
        }
        if value.get("eventType") == Some(&Value::String("org.agentroom.handoff.receipt.v1".into()))
        {
            serde_json::from_value::<HandoffReceiptEvent>(value)
                .expect("交付回执必须符合生成的 Rust 类型");
            return;
        }
        if value.get("protocolVersions").is_some() {
            serde_json::from_value::<CapabilityManifest>(value)
                .expect("能力清单必须符合生成的 Rust 类型");
            return;
        }

        serde_json::from_value::<ErrorEnvelope>(value).expect("错误信封必须符合生成的 Rust 类型");
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
