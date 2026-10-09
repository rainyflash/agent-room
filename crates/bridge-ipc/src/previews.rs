//! 消息预览对外的形状。本机 Bridge 交给 CLI、MCP 的，与服务器网关交给网络 Agent 的是同一种，
//! 都从验签后的投影转换过来。

use agent_room_bridge_core::{
    agent_identity::BridgeAgentIdentity,
    messages::{ProjectedMessageActor, ProjectedMessagePreview},
};
use agent_room_domain::{
    ids::AgentId,
    messages::{MessageContentReference, MessageProvenance, MessageRelation, MessageSensitivity},
};
use serde_json::Value;

use crate::{
    IpcActorSummary, IpcAgentSummary, IpcContentReference, IpcConversationMessage,
    IpcMessagePreviewSummary, IpcMessageProvenance, IpcMessageSensitivity, IpcReplyExcerpt,
};

/// 一批新消息里，正文超过这么多字只给开头（维护者 2026-09-30 定）；按 ID 取时给全文。
pub const BATCH_TEXT_CHARACTERS: usize = 1_000;
/// 被回复的那条，原文开头取这么多字。
pub const REPLY_EXCERPT_CHARACTERS: usize = 120;

/// 读消息的是谁：用来标出它自己发的、提到它的。
#[derive(Debug, Clone, Copy)]
pub struct PreviewViewer<'a> {
    pub agent_id: AgentId,
    pub matrix_user_id: &'a str,
}

/// 消息所在的房间：房间名，和读消息的这个 Agent 什么时候加入的（Unix 毫秒）。不知道就没有。
#[derive(Debug, Clone, Copy, Default)]
pub struct PreviewRoom<'a> {
    pub name: Option<&'a str>,
    pub joined_at_ms: Option<i64>,
}

/// 正文给多少：一批新消息里长正文只给开头，按 ID 取时给全文。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewText {
    Batch,
    Full,
}

/// 交给某个 Agent 看的预览：标出它自己发的、提到它的、它加入房间之前的，附上房间名和被回复
/// 那条的开头。`replied` 是被回复的那条；调用方找不到时传 `None`，只是少了摘录。
/// 私人房间里的 @所有人 对除了发的人以外的每个人都算提到了它。
pub fn preview_for(
    preview: &ProjectedMessagePreview,
    viewer: PreviewViewer<'_>,
    room: PreviewRoom<'_>,
    replied: Option<&ProjectedMessagePreview>,
    text: PreviewText,
) -> IpcMessagePreviewSummary {
    let mut summary = preview_summary(preview);
    summary.from_me = is_viewer(&preview.actor, viewer);
    summary.room_name = room.name.map(str::to_owned);
    summary.before_join = room
        .joined_at_ms
        .is_some_and(|joined_at_ms| sent_at_ms(preview) < joined_at_ms);
    let replied_to_viewer = replied.is_some_and(|message| is_viewer(&message.actor, viewer));
    summary.mentions_me = replied_to_viewer
        || (summary.mentions_everyone && !summary.from_me)
        || summary.conversation.as_ref().is_some_and(|chat| {
            chat.mentions
                .iter()
                .any(|mention| mention == viewer.matrix_user_id)
        });
    summary.reply_to = replied.map(reply_excerpt);
    if text == PreviewText::Batch
        && let Some(chat) = summary.conversation.as_mut()
    {
        truncate_conversation(chat);
    }
    summary
}

/// 被回复的那条手边只有存下来的预览时（网络 Agent 的消息记录）：补上它的开头，回复的是读的人
/// 发的也算提到它。和 [`preview_for`] 给的一样。
pub fn attach_stored_reply(
    summary: &mut IpcMessagePreviewSummary,
    replied: &IpcMessagePreviewSummary,
) {
    let text = replied
        .conversation
        .as_ref()
        .map_or(replied.title.as_str(), |chat| chat.text.as_str());
    summary.reply_to = Some(IpcReplyExcerpt {
        message_id: replied.message_id.clone(),
        actor_name: actor_user_and_name(&replied.actor).1.to_owned(),
        excerpt: leading_characters(text, REPLY_EXCERPT_CHARACTERS),
    });
    summary.mentions_me |= replied.from_me;
}

/// 作者的 Matrix 用户 ID 和名字：只看某个人时拿来比。
pub fn actor_user_and_name(actor: &IpcActorSummary) -> (&str, &str) {
    match actor {
        IpcActorSummary::Human {
            matrix_user_id,
            display_name,
            ..
        } => (matrix_user_id, display_name),
        IpcActorSummary::Agent { agent, .. } => (&agent.matrix_user_id, &agent.display_name),
    }
}

/// 正文超过 [`BATCH_TEXT_CHARACTERS`] 个字时只留开头，并标出全文多长。
pub fn truncate_conversation(chat: &mut IpcConversationMessage) {
    let length = chat.text.chars().count();
    if length <= BATCH_TEXT_CHARACTERS {
        return;
    }
    chat.text = leading_characters(&chat.text, BATCH_TEXT_CHARACTERS);
    chat.truncated = true;
    chat.full_length = u32::try_from(length).ok();
}

/// 与 [`truncate_conversation`] 相同，用在已经存成 JSON 的预览上（网络 Agent 的收件箱）。
pub fn truncate_preview_value(preview: &mut Value) {
    let Some(chat) = preview
        .get_mut("conversation")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    let Some(text) = chat.get("text").and_then(Value::as_str) else {
        return;
    };
    let length = text.chars().count();
    if length <= BATCH_TEXT_CHARACTERS {
        return;
    }
    let leading = leading_characters(text, BATCH_TEXT_CHARACTERS);
    chat.insert("text".to_owned(), Value::String(leading));
    chat.insert("truncated".to_owned(), Value::Bool(true));
    chat.insert("fullLength".to_owned(), Value::from(length));
}

/// 发出时间：有服务器收到的时间就用它（和加入时间是同一台服务器的钟），没有才用发送方自己写的。
pub fn sent_at_ms(preview: &ProjectedMessagePreview) -> i64 {
    preview
        .origin_server_timestamp
        .and_then(|value| i64::try_from(value).ok())
        .unwrap_or_else(|| preview.created_at.value())
}

fn is_viewer(actor: &ProjectedMessageActor, viewer: PreviewViewer<'_>) -> bool {
    match actor {
        ProjectedMessageActor::Agent { identity, .. } => {
            identity.agent_id() == viewer.agent_id
                || identity.matrix_user_id().as_str() == viewer.matrix_user_id
        }
        ProjectedMessageActor::Human { matrix_user_id, .. } => {
            matrix_user_id.as_str() == viewer.matrix_user_id
        }
    }
}

fn reply_excerpt(message: &ProjectedMessagePreview) -> IpcReplyExcerpt {
    let text = message
        .preview
        .conversation()
        .map_or_else(|| message.preview.title().as_str(), |chat| chat.text());
    IpcReplyExcerpt {
        message_id: message.message_id.to_string(),
        actor_name: match &message.actor {
            ProjectedMessageActor::Agent { identity, .. } => identity.display_name().to_owned(),
            ProjectedMessageActor::Human { display_name, .. } => display_name.clone(),
        },
        excerpt: leading_characters(text, REPLY_EXCERPT_CHARACTERS),
    }
}

/// 按字取开头，不会切在一个字的中间。
fn leading_characters(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

/// 验签后的消息预览，不看是谁在读：不标自己发的、提到自己的，正文给全文。
pub fn preview_summary(preview: &ProjectedMessagePreview) -> IpcMessagePreviewSummary {
    IpcMessagePreviewSummary {
        conversation: preview
            .preview
            .conversation()
            .map(|chat| IpcConversationMessage {
                attachment_name: chat.attachment_name().map(str::to_owned),
                text: chat.text().to_owned(),
                mentions: chat.mentions().to_vec(),
                truncated: false,
                full_length: None,
            }),
        reply_to_message_id: preview.relation.map(|relation| match relation {
            MessageRelation::ReplyTo(id) => id.to_string(),
        }),
        reply_to: None,
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
        from_me: false,
        mentions_me: false,
        // 收的时候已经只认加密消息上的 @所有人，这里拿到的就是算数的。
        mentions_everyone: preview
            .preview
            .conversation()
            .is_some_and(agent_room_domain::messages::ConversationMessage::mentions_everyone),
        room_name: None,
        before_join: false,
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

#[cfg(test)]
mod tests {
    use agent_room_application::ports::{MatrixEventId, MatrixRoomId};
    use agent_room_bridge_core::{
        agent_identity::BridgeAgentIdentity,
        messages::{ProjectedMessageActor, ProjectedMessagePreview},
    };
    use agent_room_domain::{
        content::{ContentMediaType, Sha256Digest},
        ids::{AgentId, AgentInstanceId, ContentId, MessageId},
        messages::{
            ConversationMessage, MessageContentReference, MessagePreview, MessageProvenance,
            MessageRelation, MessageRiskFlags, MessageSensitivity, MessageSummary, MessageTitle,
        },
        time::UtcMillis,
    };
    use serde_json::json;
    use uuid::Uuid;

    use super::{
        BATCH_TEXT_CHARACTERS, PreviewRoom, PreviewText, PreviewViewer, preview_for,
        truncate_preview_value,
    };

    fn agent(name: &str, matrix_user_id: &str) -> BridgeAgentIdentity {
        BridgeAgentIdentity::new(
            AgentId::from_uuid(Uuid::now_v7()),
            name,
            matrix_user_id,
            AgentInstanceId::from_uuid(Uuid::now_v7()),
        )
        .expect("身份有效")
    }

    fn chat(
        author: &BridgeAgentIdentity,
        text: &str,
        mentions: Vec<String>,
        reply_to: Option<MessageId>,
    ) -> ProjectedMessagePreview {
        ProjectedMessagePreview {
            event_id: MatrixEventId::new(format!("${}:matrix.test", Uuid::now_v7().simple()))
                .expect("事件标识有效"),
            transaction_id: None,
            room_id: MatrixRoomId::new("!lobby:matrix.test").expect("房间标识有效"),
            message_id: MessageId::from_uuid(Uuid::now_v7()),
            created_at: UtcMillis::new(1_000).expect("时间有效"),
            origin_server_timestamp: Some(1_000),
            actor: ProjectedMessageActor::new(author.clone(), MessageProvenance::AutonomousAgent),
            preview: MessagePreview::new(
                MessageTitle::new("聊天").expect("标题有效"),
                MessageSummary::new("聊天").expect("摘要有效"),
                ContentMediaType::new("text/plain").expect("媒体类型有效"),
                None,
                MessageSensitivity::Normal,
                MessageRiskFlags::new([]).expect("风险标签有效"),
            )
            .with_conversation(ConversationMessage::new(text.into(), mentions).expect("对话有效")),
            content: MessageContentReference::new(
                ContentId::from_uuid(Uuid::now_v7()),
                Sha256Digest::from_bytes([7; 32]),
                16,
            )
            .expect("正文引用有效"),
            relation: reply_to.map(MessageRelation::ReplyTo),
        }
    }

    fn viewer(identity: &BridgeAgentIdentity) -> PreviewViewer<'_> {
        PreviewViewer {
            agent_id: identity.agent_id(),
            matrix_user_id: identity.matrix_user_id().as_str(),
        }
    }

    #[test]
    fn 带上房间名_标出加入之前的消息() {
        let me = agent("Scout", "@scout:matrix.test");
        let ada = agent("Ada", "@ada:matrix.test");
        let mut message = chat(&ada, "大家好", Vec::new(), None);
        let room = |joined_at_ms| PreviewRoom {
            name: Some("项目室"),
            joined_at_ms,
        };

        let before = preview_for(
            &message,
            viewer(&me),
            room(Some(2_000)),
            None,
            PreviewText::Batch,
        );
        assert_eq!(before.room_name.as_deref(), Some("项目室"));
        assert!(before.before_join, "服务器 1 秒收到，2 秒才加入");
        let after = preview_for(
            &message,
            viewer(&me),
            room(Some(1_000)),
            None,
            PreviewText::Batch,
        );
        assert!(!after.before_join, "同一时刻加入的不算之前");
        let unknown = preview_for(&message, viewer(&me), room(None), None, PreviewText::Batch);
        assert!(!unknown.before_join, "不知道什么时候加入的就不标");

        // 没有服务器时间时才用发送方写的时间。
        message.origin_server_timestamp = None;
        message.created_at = UtcMillis::new(3_000).expect("时间有效");
        let claimed = preview_for(
            &message,
            viewer(&me),
            room(Some(2_000)),
            None,
            PreviewText::Batch,
        );
        assert!(!claimed.before_join);

        let json = serde_json::to_value(&before).expect("能序列化");
        assert_eq!(json["roomName"], "项目室");
        assert_eq!(json["beforeJoin"], true);
        let nameless = preview_for(
            &message,
            viewer(&me),
            PreviewRoom::default(),
            None,
            PreviewText::Batch,
        );
        let json = serde_json::to_value(&nameless).expect("能序列化");
        assert!(json.get("roomName").is_none(), "没有房间名就不写");
        assert_eq!(json["beforeJoin"], false);
    }

    #[test]
    fn 标出自己发的和点名自己的() {
        let me = agent("Scout", "@scout:matrix.test");
        let ada = agent("Ada", "@ada:matrix.test");

        let own = preview_for(
            &chat(&me, "我先看看", Vec::new(), None),
            viewer(&me),
            PreviewRoom::default(),
            None,
            PreviewText::Batch,
        );
        assert!(own.from_me);
        assert!(!own.mentions_me);

        let named = chat(
            &ada,
            "Scout 你怎么看？",
            vec!["@scout:matrix.test".into()],
            None,
        );
        let named = preview_for(
            &named,
            viewer(&me),
            PreviewRoom::default(),
            None,
            PreviewText::Batch,
        );
        assert!(!named.from_me);
        assert!(named.mentions_me);

        let others = chat(&ada, "大家好", vec!["@mina:matrix.test".into()], None);
        assert!(
            !preview_for(
                &others,
                viewer(&me),
                PreviewRoom::default(),
                None,
                PreviewText::Batch
            )
            .mentions_me
        );
    }

    #[test]
    fn 所有人_对除了发的人以外的每个人都算提到了它() {
        let me = agent("Scout", "@scout:matrix.test");
        let ada = agent("Ada", "@ada:matrix.test");
        let mut everyone = chat(&ada, "大家看一下", Vec::new(), None);
        everyone.preview = everyone.preview.clone().with_conversation(
            ConversationMessage::new("大家看一下".into(), Vec::new())
                .expect("对话有效")
                .with_mentions_everyone(true),
        );

        let mine = preview_for(
            &everyone,
            viewer(&me),
            PreviewRoom::default(),
            None,
            PreviewText::Batch,
        );
        assert!(mine.mentions_me);
        assert!(mine.mentions_everyone);
        let json = serde_json::to_value(&mine).expect("能序列化");
        assert_eq!(json["mentionsEveryone"], true);

        let sender = preview_for(
            &everyone,
            viewer(&ada),
            PreviewRoom::default(),
            None,
            PreviewText::Batch,
        );
        assert!(sender.from_me);
        assert!(!sender.mentions_me, "自己发的不算提到自己");

        let plain = preview_for(
            &chat(&ada, "大家好", Vec::new(), None),
            viewer(&me),
            PreviewRoom::default(),
            None,
            PreviewText::Batch,
        );
        let json = serde_json::to_value(&plain).expect("能序列化");
        assert!(json.get("mentionsEveryone").is_none(), "没 @所有人 就不写");
    }

    #[test]
    fn 回复自己的消息也算点名_并附上被回复那条的开头() {
        let me = agent("Scout", "@scout:matrix.test");
        let ada = agent("Ada", "@ada:matrix.test");
        let long_question = "这是一句很长的问题，".repeat(30);
        let mine = chat(&me, &long_question, Vec::new(), None);
        let reply = chat(&ada, "同意", Vec::new(), Some(mine.message_id));

        let summary = preview_for(
            &reply,
            viewer(&me),
            PreviewRoom::default(),
            Some(&mine),
            PreviewText::Batch,
        );
        assert!(summary.mentions_me, "回复的是我发的");
        let excerpt = summary.reply_to.expect("附上被回复的那条");
        assert_eq!(excerpt.message_id, mine.message_id.to_string());
        assert_eq!(excerpt.actor_name, "Scout");
        assert_eq!(
            excerpt.excerpt.chars().count(),
            super::REPLY_EXCERPT_CHARACTERS
        );
        assert!(long_question.starts_with(&excerpt.excerpt));

        // 回复的是别人的：附上摘录，但不算点名我。
        let lena = agent("Lena", "@lena:matrix.test");
        let hers = chat(&lena, "我来", Vec::new(), None);
        let reply = chat(&ada, "好", Vec::new(), Some(hers.message_id));
        let summary = preview_for(
            &reply,
            viewer(&me),
            PreviewRoom::default(),
            Some(&hers),
            PreviewText::Batch,
        );
        assert!(!summary.mentions_me);
        assert_eq!(summary.reply_to.expect("有摘录").excerpt, "我来");
    }

    #[test]
    fn 一批里长正文只给开头_按_id_取时给全文() {
        let me = agent("Scout", "@scout:matrix.test");
        let ada = agent("Ada", "@ada:matrix.test");
        let text = "字".repeat(1_500);
        let long = chat(&ada, &text, Vec::new(), None);

        let batch = preview_for(
            &long,
            viewer(&me),
            PreviewRoom::default(),
            None,
            PreviewText::Batch,
        );
        let chat_batch = batch.conversation.expect("有正文");
        assert_eq!(chat_batch.text.chars().count(), BATCH_TEXT_CHARACTERS);
        assert!(chat_batch.truncated);
        assert_eq!(chat_batch.full_length, Some(1_500));

        let full = preview_for(
            &long,
            viewer(&me),
            PreviewRoom::default(),
            None,
            PreviewText::Full,
        );
        let chat_full = full.conversation.expect("有正文");
        assert_eq!(chat_full.text, text);
        assert!(!chat_full.truncated);
        assert_eq!(chat_full.full_length, None);

        // 正好 1000 字不截断，JSON 里也不出现这两个字段。
        let exact = chat(&ada, &"字".repeat(BATCH_TEXT_CHARACTERS), Vec::new(), None);
        let exact = preview_for(
            &exact,
            viewer(&me),
            PreviewRoom::default(),
            None,
            PreviewText::Batch,
        );
        let value = serde_json::to_value(&exact).expect("可编码");
        assert!(value["conversation"].get("truncated").is_none());
        assert!(value["conversation"].get("fullLength").is_none());
        assert_eq!(value["fromMe"], false);
        assert_eq!(value["mentionsMe"], false);
    }

    #[test]
    fn 存成_json_的预览也按同样的规则截断() {
        let mut preview = json!({"conversation": {"text": "字".repeat(1_200), "mentions": []}});
        truncate_preview_value(&mut preview);
        assert_eq!(
            preview["conversation"]["text"]
                .as_str()
                .map(|text| text.chars().count()),
            Some(BATCH_TEXT_CHARACTERS)
        );
        assert_eq!(preview["conversation"]["truncated"], true);
        assert_eq!(preview["conversation"]["fullLength"], 1_200);

        let mut short = json!({"conversation": {"text": "短", "mentions": []}});
        truncate_preview_value(&mut short);
        assert_eq!(
            short,
            json!({"conversation": {"text": "短", "mentions": []}})
        );

        let mut document = json!({"conversation": null, "title": "长文资料"});
        truncate_preview_value(&mut document);
        assert_eq!(document, json!({"conversation": null, "title": "长文资料"}));
    }
}
