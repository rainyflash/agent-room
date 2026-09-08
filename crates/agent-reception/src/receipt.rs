use crate::{
    DeliveryRecord, ReceiverBinding, ReceptionFailure as Failure, ReceptionResult as Result,
};
use agent_room_agent_client::{BridgeToolClient, MessageReadMode, wait_for_messages};
use agent_room_bridge_ipc::{
    IpcActorSummary, IpcMessagePreviewSummary, IpcMessageProvenance, IpcResponse,
};
use std::time::Duration;

pub(crate) fn matches_reply(
    message: &IpcMessagePreviewSummary,
    binding: &ReceiverBinding,
    record: &DeliveryRecord,
    agent_id: &str,
) -> bool {
    message.room_id == binding.policy.room_id
        && message.message_id == record.submission_id
        && message.reply_to_message_id.as_deref() == Some(&record.message_id)
        && message.conversation.is_some()
        && matches!(&message.actor, IpcActorSummary::Agent {agent, provenance: IpcMessageProvenance::AutonomousAgent, ..}
            if agent.agent_id == agent_id)
}

// Verify the signed message projection, never a model's prose or a process exit code.
pub(crate) async fn verify(
    backend: &dyn BridgeToolClient,
    session_id: &str,
    binding: &ReceiverBinding,
    record: &DeliveryRecord,
    agent_id: &str,
    duration: Duration,
) -> Result<String> {
    let operation = async {
        let mut cursor = Some(record.event_id.clone());
        loop {
            let response = wait_for_messages(
                backend,
                session_id.to_owned(),
                binding.inbox_request(cursor.clone()),
                MessageReadMode::Inbox,
                1,
            )
            .await?;
            let IpcResponse::MessagePreviews { previews, .. } = response else {
                return Err(Failure::local("receiver.response_invalid"));
            };
            for message in &previews {
                if matches_reply(message, binding, record, agent_id) {
                    return Ok(message.event_id.clone());
                }
            }
            if let Some(last) = previews.last() {
                if cursor.as_deref() == Some(&last.event_id) {
                    return Err(Failure::local("receiver.receipt_cursor_stalled"));
                }
                cursor = Some(last.event_id.clone());
            }
        }
    };
    tokio::time::timeout(duration, operation)
        .await
        .map_err(|_| Failure::local("receiver.reply_unconfirmed"))?
}
