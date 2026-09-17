use crate::{
    DeliveryRecord, ReceiverState, ReceptionFailure as Failure, ReceptionResult as Result,
};
use agent_room_bridge_ipc::{
    IpcMessageProvenance, IpcMessageSensitivity, IpcMethod, IpcSendMessageRequest,
};
use serde::Deserialize;

/// Model output is conversation data. Routing, grants and idempotency stay owned by reception.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostReply {
    body: String,
}

impl HostReply {
    /// 回复正文。契约检查据此比对金丝雀；接待本身不读取它。
    #[must_use]
    pub fn body(&self) -> &str {
        &self.body
    }

    /// # Errors
    /// Rejects content that is not a valid conversation message.
    pub fn new(body: String) -> Result<Self> {
        agent_room_bridge_core::messages::validate_chat(&body, &[])
            .map_err(|_| Failure::local("receiver.host_reply_invalid"))?;
        Ok(Self { body })
    }

    pub(crate) fn parse(text: &str) -> Result<Self> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct ReplyContent {
            body: String,
        }
        let reply: ReplyContent = serde_json::from_str(text)
            .map_err(|_| Failure::local("receiver.host_reply_invalid"))?;
        Self::new(reply.body)
    }

    pub(crate) fn into_request(
        self,
        state: &ReceiverState,
        record: &DeliveryRecord,
    ) -> Result<IpcMethod> {
        let run_id = state
            .execution
            .as_ref()
            .ok_or_else(|| Failure::local("reception.execution_missing"))?
            .run_id;
        Ok(IpcMethod::SendReceptionMessage {
            run_id,
            request: IpcSendMessageRequest {
                chat: true,
                mentions: vec![],
                submission_id: Some(record.submission_id.clone()),
                automation_grant_id: Some(state.binding.automation_grant_id.clone()),
                room_id: state.binding.policy.room_id.clone(),
                title: self.body.chars().take(120).collect(),
                summary: self.body.chars().take(280).collect(),
                body: self.body,
                media_type: "text/plain".into(),
                language: None,
                sensitivity: IpcMessageSensitivity::Normal,
                risk_flags: vec![],
                provenance: IpcMessageProvenance::AutonomousAgent,
                reply_to_message_id: Some(record.message_id.clone()),
            },
        })
    }
}

pub(crate) const SCHEMA: &str = r#"{"type":"object","properties":{"body":{"type":"string"}},"required":["body"],"additionalProperties":false}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_valid_reply_content_is_accepted() {
        assert_eq!(HostReply::parse(r#"{"body":"收到"}"#).unwrap().body, "收到");
        for input in [
            "{}",
            r#"{"body":""}"#,
            r#"{"body":4}"#,
            r#"{"body":"hello","roomId":"!other:test"}"#,
            "not json",
        ] {
            assert!(HostReply::parse(input).is_err());
        }
        assert!(HostReply::new("x".repeat(4001)).is_err());
    }
}
