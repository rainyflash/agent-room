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
        let reply: ReplyContent = serde_json::from_str(unfenced(text))
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

/// 模型即使被要求不要，偶尔也会把 JSON 整段包进一个 Markdown 代码块（```json … ```）。
/// 只拆开恰好一整块、信息串为空或 json 的回复；代码块前后有任何其他文字仍按无效处理。
fn unfenced(text: &str) -> &str {
    text.trim()
        .strip_prefix("```")
        .and_then(|rest| rest.strip_suffix("```"))
        .and_then(|inner| inner.split_once('\n'))
        .filter(|(info, _)| {
            let info = info.trim();
            info.is_empty() || info.eq_ignore_ascii_case("json")
        })
        .map_or(text, |(_, content)| content)
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

    #[test]
    fn 整段包在一个代码块里的回复也能读() {
        for input in [
            "```json\n{\"body\": \"收到\"}\n```",
            "```\n{\"body\":\"收到\"}\n```",
            "\n  ```JSON\r\n{\"body\":\"收到\"}\r\n```\n",
        ] {
            assert_eq!(HostReply::parse(input).unwrap().body, "收到", "{input:?}");
        }
        // 正文里的代码块原样保留。
        assert_eq!(
            HostReply::parse("```json\n{\"body\":\"用 ```rust``` 包代码\"}\n```")
                .unwrap()
                .body,
            "用 ```rust``` 包代码"
        );
        for input in [
            "好的：\n```json\n{\"body\":\"收到\"}\n```",
            "```json\n{\"body\":\"收到\"}\n```\n以上。",
            "```json {\"body\":\"收到\"}```",
            "```rust\n{\"body\":\"收到\"}\n```",
            "```json\n{\"body\":\"收到\"}\n```\n```json\n{\"body\":\"再一条\"}\n```",
            "```json\n{\"body\":\"收到\",\"roomId\":\"!other:test\"}\n```",
            "```json\n{\"body\":\"收到\"}",
        ] {
            assert!(HostReply::parse(input).is_err(), "{input:?}");
        }
    }
}
