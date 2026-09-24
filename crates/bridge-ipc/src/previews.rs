//! 消息预览对外的形状。本机 Bridge 交给 CLI、MCP 的，与服务器网关交给网络 Agent 的是同一种，
//! 都从验签后的投影转换过来。

use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    messages::{ProjectedMessageActor, ProjectedMessagePreview},
};
use agent_room_domain::messages::{
    MessageContentReference, MessageProvenance, MessageRelation, MessageSensitivity,
};

use crate::{
    IpcActorSummary, IpcAgentSummary, IpcContentReference, IpcConversationMessage,
    IpcMessagePreviewSummary, IpcMessageProvenance, IpcMessageSensitivity,
};

/// 验签后的消息预览。
pub fn preview_summary(preview: &ProjectedMessagePreview) -> IpcMessagePreviewSummary {
    IpcMessagePreviewSummary {
        conversation: preview
            .preview
            .conversation()
            .map(|chat| IpcConversationMessage {
                attachment_name: chat.attachment_name().map(str::to_owned),
                text: chat.text().to_owned(),
                mentions: chat.mentions().to_vec(),
            }),
        reply_to_message_id: preview.relation.map(|relation| match relation {
            MessageRelation::ReplyTo(id) => id.to_string(),
        }),
        message_id: preview.message_id.to_string(),
        event_id: preview.event_id.as_str().to_owned(),
        room_id: preview.room_id.as_str().to_owned(),
        actor: actor_summary(&preview.actor),
        created_at_unix_ms: preview.created_at.value(),
        title: preview.preview.title().as_str().to_owned(),
        summary: preview.preview.summary().as_str().to_owned(),
        content: content_reference(&preview.content, preview.preview.content_type().as_str()),
        language: preview
            .preview
            .language()
            .map(|value| value.as_str().to_owned()),
        sensitivity: sensitivity(preview.preview.sensitivity()),
        risk_flags: preview
            .preview
            .risk_flags()
            .iter()
            .map(|flag| flag.as_str().to_owned())
            .collect(),
    }
}

/// 发这条消息的 Agent 或人。
pub fn actor_summary(actor: &ProjectedMessageActor) -> IpcActorSummary {
    match actor {
        ProjectedMessageActor::Agent {
            identity,
            provenance: actor_provenance,
            ..
        } => IpcActorSummary::Agent {
            agent: agent_summary(identity),
            instance_id: identity.agent_instance_id().to_string(),
            provenance: provenance(*actor_provenance),
        },
        ProjectedMessageActor::Human {
            principal_id,
            display_name,
            matrix_user_id,
            avatar_url,
        } => IpcActorSummary::Human {
            principal_id: principal_id.to_string(),
            display_name: display_name.clone(),
            matrix_user_id: matrix_user_id.as_str().to_owned(),
            avatar_url: avatar_url.clone(),
        },
    }
}

pub fn agent_summary(identity: &BridgeAgentIdentity) -> IpcAgentSummary {
    IpcAgentSummary {
        agent_id: identity.agent_id().to_string(),
        display_name: identity.display_name().to_owned(),
        matrix_user_id: identity.matrix_user_id().as_str().to_owned(),
        avatar_url: identity.avatar_url().map(str::to_owned),
    }
}

pub fn content_reference(
    content: &MessageContentReference,
    media_type: &str,
) -> IpcContentReference {
    IpcContentReference {
        content_id: content.content_id().to_string(),
        digest_sha256: encode_hex(content.digest().as_bytes()),
        media_type: media_type.to_owned(),
        size_bytes: content.size_bytes(),
    }
}

pub const fn provenance(value: MessageProvenance) -> IpcMessageProvenance {
    match value {
        MessageProvenance::Human => IpcMessageProvenance::Human,
        MessageProvenance::HumanConfirmedAgent => IpcMessageProvenance::HumanConfirmedAgent,
        MessageProvenance::AutonomousAgent => IpcMessageProvenance::AutonomousAgent,
    }
}

pub const fn sensitivity(value: MessageSensitivity) -> IpcMessageSensitivity {
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
