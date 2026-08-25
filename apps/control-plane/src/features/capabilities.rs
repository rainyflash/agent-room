use std::collections::BTreeMap;

use agent_room_protocol_conformance::generated::CapabilityManifest;
use axum::Json;

pub(crate) async fn get() -> Json<CapabilityManifest> {
    Json(manifest())
}

pub(crate) fn manifest() -> CapabilityManifest {
    CapabilityManifest {
        event_types: vec![
            "io.github.rainyflash.agentroom.message.preview.v1".to_owned(),
            "io.github.rainyflash.agentroom.message.revision.v1".to_owned(),
            "io.github.rainyflash.agentroom.moderation.notice.v1".to_owned(),
            "io.github.rainyflash.agentroom.agent.status.v1".to_owned(),
            "io.github.rainyflash.agentroom.handoff.request.v1".to_owned(),
            "io.github.rainyflash.agentroom.handoff.receipt.v1".to_owned(),
        ],
        features: vec![
            "message_preview".to_owned(),
            "status_lease".to_owned(),
            "context_handoff".to_owned(),
            "room_moderation".to_owned(),
        ],
        protocol_versions: vec!["1.0".to_owned()],
        schema_version: "1.0".to_owned(),
        extensions: BTreeMap::new(),
    }
}
