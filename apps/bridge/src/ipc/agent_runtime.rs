use std::sync::Arc;

use agent_room_application::ports::{MatrixEventId, MatrixRoomId};
use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    messages::{
        MessagePreviewQuery, MessageTimelineQueryFailure, MessageTimelineQueryFailureKind,
        MessageTimelineQueryRepository, ProjectedMessageActor, ProjectedMessagePreview,
    },
};
use agent_room_bridge_ipc::{
    IpcActorSummary, IpcAgentSummary, IpcContentReference, IpcErrorCategory,
    IpcListPreviewsRequest, IpcMessagePreviewSummary, IpcMessageProvenance, IpcMessageSensitivity,
    IpcResponse, IpcSelfSummary,
};
use agent_room_domain::messages::{MessageProvenance, MessageSensitivity};

use super::{BridgeIpcDispatchFailure, BridgeStatusReader, agent_runtime_unavailable};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BridgeAgentRuntimeSnapshot {
    identity: BridgeAgentIdentity,
    matrix_device_id: String,
    room_id: MatrixRoomId,
    granted_capabilities: Vec<String>,
}

impl BridgeAgentRuntimeSnapshot {
    pub(crate) fn new(
        identity: BridgeAgentIdentity,
        matrix_device_id: impl Into<String>,
        room_id: MatrixRoomId,
        granted_capabilities: impl IntoIterator<Item = &'static str>,
    ) -> Self {
        Self {
            identity,
            matrix_device_id: matrix_device_id.into(),
            room_id,
            granted_capabilities: granted_capabilities
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }
    }
}

pub(crate) trait BridgeAgentRuntimeReader: Send + Sync {
    fn read_agent_runtime(&self) -> Option<BridgeAgentRuntimeSnapshot>;
}

pub(super) struct AgentRuntimeIpcFacade {
    status_reader: Arc<dyn BridgeStatusReader>,
    runtime_reader: Arc<dyn BridgeAgentRuntimeReader>,
    previews: Arc<dyn MessageTimelineQueryRepository>,
}

impl AgentRuntimeIpcFacade {
    pub(super) fn new(
        status_reader: Arc<dyn BridgeStatusReader>,
        runtime_reader: Arc<dyn BridgeAgentRuntimeReader>,
        previews: Arc<dyn MessageTimelineQueryRepository>,
    ) -> Self {
        Self {
            status_reader,
            runtime_reader,
            previews,
        }
    }

    pub(super) fn get_self(&self) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        Ok(IpcResponse::SelfSummary {
            summary: IpcSelfSummary {
                agent: ipc_agent(&runtime.identity),
                instance_id: runtime.identity.agent_instance_id().to_string(),
                matrix_device_id: runtime.matrix_device_id,
                connection_state: self.status_reader.read_status().state,
                granted_capabilities: runtime.granted_capabilities,
            },
        })
    }

    pub(super) async fn list_previews(
        &self,
        request: IpcListPreviewsRequest,
    ) -> Result<IpcResponse, BridgeIpcDispatchFailure> {
        let runtime = self.runtime_snapshot()?;
        let room_id = requested_room(request.room_id, &runtime.room_id)?;
        let cursor = request
            .before_event_id
            .map(MatrixEventId::new)
            .transpose()
            .map_err(|_| invalid_request("bridge.ipc.event_id_invalid"))?;
        let query = MessagePreviewQuery::new(room_id, cursor, request.limit)
            .map_err(|_| invalid_request("bridge.ipc.preview_limit_invalid"))?;
        let page = self
            .previews
            .list_previews(&query)
            .await
            .map_err(map_preview_query_failure)?;
        Ok(IpcResponse::MessagePreviews {
            previews: page.previews().iter().map(ipc_preview).collect(),
            next_cursor: page.next_cursor().map(|cursor| cursor.as_str().to_owned()),
        })
    }

    fn runtime_snapshot(&self) -> Result<BridgeAgentRuntimeSnapshot, BridgeIpcDispatchFailure> {
        self.runtime_reader
            .read_agent_runtime()
            .ok_or_else(agent_runtime_unavailable)
    }
}

fn requested_room(
    requested: Option<String>,
    active: &MatrixRoomId,
) -> Result<MatrixRoomId, BridgeIpcDispatchFailure> {
    let requested = requested
        .map(MatrixRoomId::new)
        .transpose()
        .map_err(|_| invalid_request("bridge.ipc.room_id_invalid"))?
        .unwrap_or_else(|| active.clone());
    if requested != *active {
        return Err(BridgeIpcDispatchFailure::new(
            "bridge.room_not_joined",
            IpcErrorCategory::Authorization,
            false,
        ));
    }
    Ok(requested)
}

fn ipc_agent(identity: &BridgeAgentIdentity) -> IpcAgentSummary {
    IpcAgentSummary {
        agent_id: identity.agent_id().to_string(),
        display_name: identity.display_name().to_owned(),
        matrix_user_id: identity.matrix_user_id().as_str().to_owned(),
        avatar_url: identity.avatar_url().map(str::to_owned),
    }
}

fn ipc_actor(actor: &ProjectedMessageActor) -> IpcActorSummary {
    IpcActorSummary {
        agent: ipc_agent(actor.identity()),
        instance_id: actor.identity().agent_instance_id().to_string(),
        provenance: ipc_provenance(actor.provenance()),
    }
}

fn ipc_preview(preview: &ProjectedMessagePreview) -> IpcMessagePreviewSummary {
    IpcMessagePreviewSummary {
        message_id: preview.message_id.to_string(),
        event_id: preview.event_id.as_str().to_owned(),
        room_id: preview.room_id.as_str().to_owned(),
        actor: ipc_actor(&preview.actor),
        created_at_unix_ms: preview.created_at.value(),
        title: preview.preview.title().as_str().to_owned(),
        summary: preview.preview.summary().as_str().to_owned(),
        content: IpcContentReference {
            content_id: preview.content.content_id().to_string(),
            digest_sha256: encode_hex(preview.content.digest().as_bytes()),
            media_type: preview.preview.content_type().as_str().to_owned(),
            size_bytes: preview.content.size_bytes(),
        },
        language: preview
            .preview
            .language()
            .map(|value| value.as_str().to_owned()),
        sensitivity: ipc_sensitivity(preview.preview.sensitivity()),
        risk_flags: preview
            .preview
            .risk_flags()
            .iter()
            .map(|flag| flag.as_str().to_owned())
            .collect(),
    }
}

const fn ipc_provenance(value: MessageProvenance) -> IpcMessageProvenance {
    match value {
        MessageProvenance::Human => IpcMessageProvenance::Human,
        MessageProvenance::HumanConfirmedAgent => IpcMessageProvenance::HumanConfirmedAgent,
        MessageProvenance::AutonomousAgent => IpcMessageProvenance::AutonomousAgent,
    }
}

const fn ipc_sensitivity(value: MessageSensitivity) -> IpcMessageSensitivity {
    match value {
        MessageSensitivity::Normal => IpcMessageSensitivity::Normal,
        MessageSensitivity::Sensitive => IpcMessageSensitivity::Sensitive,
        MessageSensitivity::Restricted => IpcMessageSensitivity::Restricted,
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

fn map_preview_query_failure(failure: MessageTimelineQueryFailure) -> BridgeIpcDispatchFailure {
    match failure.kind() {
        MessageTimelineQueryFailureKind::Unavailable => BridgeIpcDispatchFailure::new(
            "bridge.message_projection_unavailable",
            IpcErrorCategory::DependencyUnavailable,
            true,
        ),
        MessageTimelineQueryFailureKind::CursorNotFound => {
            invalid_request("bridge.preview_cursor_not_found")
        }
        MessageTimelineQueryFailureKind::Corrupt => BridgeIpcDispatchFailure::new(
            "bridge.message_projection_corrupt",
            IpcErrorCategory::Internal,
            false,
        ),
    }
}

const fn invalid_request(code: &'static str) -> BridgeIpcDispatchFailure {
    BridgeIpcDispatchFailure::new(code, IpcErrorCategory::Validation, false)
}
