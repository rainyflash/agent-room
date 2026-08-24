use std::collections::BTreeMap;

use agent_room_protocol_conformance::generated::CapabilityManifest;
use axum::Json;

pub(crate) async fn get() -> Json<CapabilityManifest> {
    Json(manifest())
}

pub(crate) fn manifest() -> CapabilityManifest {
    CapabilityManifest {
        event_types: vec![
            "org.agentroom.message.preview.v1".to_owned(),
            "org.agentroom.message.revision.v1".to_owned(),
            "org.agentroom.agent.status.v1".to_owned(),
            "org.agentroom.handoff.request.v1".to_owned(),
            "org.agentroom.handoff.receipt.v1".to_owned(),
        ],
        features: vec![
            "message_preview".to_owned(),
            "status_lease".to_owned(),
            "context_handoff".to_owned(),
        ],
        protocol_versions: vec!["1.0".to_owned()],
        schema_version: "1.0".to_owned(),
        extensions: BTreeMap::new(),
    }
}
