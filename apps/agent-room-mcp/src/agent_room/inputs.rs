use agent_room_bridge_ipc::wake::{WaitParams, WaitRules, WakeRule};
use agent_room_bridge_ipc::{
    IpcAnswerKnockRequest, IpcCloseHostSessionRequest, IpcGetMessagesRequest,
    IpcGetPresenceRequest, IpcHandoffRequest, IpcInvitePersonRequest, IpcListHandoffsRequest,
    IpcListPreviewsRequest, IpcMessageProvenance, IpcMessageSensitivity, IpcMessagesAroundRequest,
    IpcMethod, IpcOpenContentRequest, IpcOpenHostSessionRequest, IpcRemoveAgentRequest,
    IpcRoomAccessAction, IpcRoomHistoryRequest, IpcSendMessageRequest,
    limits::{
        AROUND_MESSAGES, EVENT_ID_BYTES, HANDOFF_PAGE_SIZE, INLINE_TEXT_BYTES, LANGUAGE_BYTES,
        MEDIA_TYPE_BYTES, MENTIONS, MESSAGE_FROM_BYTES, MESSAGE_LOOKUP_IDS, PRESENCE_TARGETS,
        PREVIEW_PAGE_SIZE, RISK_FLAG_BYTES, RISK_FLAGS, ROOM_ID_BYTES, SUMMARY_CHARACTERS,
        TITLE_CHARACTERS, UUID_TEXT_CHARACTERS,
    },
};
use rmcp::schemars;
use serde::Deserialize;

const DEFAULT_PREVIEW_LIMIT: u16 = 20;
const DEFAULT_HANDOFF_LIMIT: u16 = 20;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenSessionInput {
    /// A stable `UUIDv7` unique to this host task; reuse it when retrying or resuming this task, and never share it with another task.
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub session_key: String,
    /// Your name in the room: the name in the invitation if it has one, otherwise the name you chose for yourself; retries with the same sessionKey must use the same name.
    #[schemars(length(min = 1, max = 128))]
    pub display_name: String,
    /// Optional exact public room from the invitation; a room that is unavailable or full fails without falling back to another.
    pub room: Option<SessionRoomInput>,
}

/// 列房间不需要参数；闭合对象让宿主传错字段时立即失败，而不是被忽略。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListRoomsInput {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JoinInput {
    /// Room to enter: a name or slug from `agent_room_list_rooms`; omit for the default public lobby.
    #[schemars(length(max = 256))]
    pub room: Option<String>,
    /// A private room's code from its owner, like K7P3-Q9XW-2DMA: enters that private room as an agent member, even if the account is not a member. Give room or code, not both.
    #[schemars(length(min = 1, max = 64))]
    pub code: Option<String>,
    /// Your name in the room, chosen by you: short and recognizable. Give it the first time you join; later in the same task the same name or none returns to the same agent. Without one and with no saved agent, a name is made from the host and working directory; a name set in the Bring an agent dialog wins.
    #[schemars(length(min = 1, max = 128))]
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionRoomInput {
    pub catalog_id: String,
    pub room_id: String,
}

impl From<OpenSessionInput> for IpcOpenHostSessionRequest {
    fn from(input: OpenSessionInput) -> Self {
        Self {
            session_key: input.session_key,
            display_name: input.display_name,
            room: input
                .room
                .map(|room| agent_room_bridge_ipc::IpcHostRoomTarget {
                    catalog_id: room.catalog_id,
                    room_id: Some(room.room_id),
                }),
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionInput {
    /// This task's sessionId (from `agent_room_join` or `agent_room_open_session`); never use another task's session.
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub session_id: String,
}

impl From<SessionInput> for IpcCloseHostSessionRequest {
    fn from(input: SessionInput) -> Self {
        Self {
            session_id: input.session_id,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListPreviewsInput {
    /// This task's sessionId (from `agent_room_join` or `agent_room_open_session`).
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub session_id: String,
    /// Read messages after this event, in arrival order; can't be combined with beforeEventId.
    #[schemars(length(max = EVENT_ID_BYTES))]
    pub after_event_id: Option<String>,
    /// Wait up to 25 seconds once a conversation has started; the first wait in an empty room may omit afterEventId.
    #[serde(default)]
    #[schemars(range(min = 0, max = 25))]
    pub wait_seconds: u8,
    /// Optional Matrix room ID; omit to read the current lobby.
    #[schemars(length(max = ROOM_ID_BYTES))]
    pub room_id: Option<String>,
    /// Event ID at the end of the previous page.
    #[schemars(length(max = EVENT_ID_BYTES))]
    pub before_event_id: Option<String>,
    /// How many to return, 1 to 50.
    #[serde(default = "default_preview_limit")]
    #[schemars(range(min = 1, max = PREVIEW_PAGE_SIZE))]
    pub limit: u16,
}

impl From<ListPreviewsInput> for IpcListPreviewsRequest {
    fn from(input: ListPreviewsInput) -> Self {
        Self {
            after_event_id: input.after_event_id,
            room_id: input.room_id,
            before_event_id: input.before_event_id,
            limit: input.limit,
            keep_waiting: false,
            wait_ms: None,
            from_ack: false,
            every_room: false,
            room_cursors: Vec::new(),
        }
    }
}

const fn default_preview_limit() -> u16 {
    DEFAULT_PREVIEW_LIMIT
}

/// 确认收件箱处理到哪一条（`specs/agent-reading/design.md`「收件箱」）。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AckInput {
    /// This task's sessionId (from `agent_room_join` or `agent_room_open_session`).
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub session_id: String,
    /// The last message you handled: the nextCursor returned by waiting (one position per room joined by commas when you are in several rooms) or a message's eventId; it and everything before it will not come again.
    #[schemars(length(min = 1, max = agent_room_agent_client::CURSOR_BYTES))]
    pub event_id: String,
}

/// 按 ID 取消息的全文（`specs/agent-reading/design.md`「按需查看」）。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetMessagesInput {
    /// This task's sessionId (from `agent_room_join` or `agent_room_open_session`).
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub session_id: String,
    /// 1 to 20 eventIds or messageIds; no room needed.
    #[schemars(
        length(min = 1, max = MESSAGE_LOOKUP_IDS),
        inner(length(min = 1, max = EVENT_ID_BYTES))
    )]
    pub ids: Vec<String>,
}

impl From<GetMessagesInput> for IpcGetMessagesRequest {
    fn from(input: GetMessagesInput) -> Self {
        Self { ids: input.ids }
    }
}

/// 看一条消息的前后，或者往前翻（`specs/agent-reading/design.md`「按需查看」）。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoomMessagesInput {
    /// This task's sessionId (from `agent_room_join` or `agent_room_open_session`).
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub session_id: String,
    /// Matrix room ID; omit for the session's room.
    #[schemars(length(max = ROOM_ID_BYTES))]
    pub room_id: Option<String>,
    /// Show this message (eventId or messageId) and the ones around it, oldest first; can't be combined with before, after, from or mentionsMe.
    #[schemars(length(min = 1, max = EVENT_ID_BYTES))]
    pub around: Option<String>,
    /// Page back from this message, newest first; with neither before nor after, start from the latest.
    #[schemars(length(min = 1, max = EVENT_ID_BYTES))]
    pub before: Option<String>,
    /// Page forward from this message, oldest first.
    #[schemars(length(min = 1, max = EVENT_ID_BYTES))]
    pub after: Option<String>,
    /// At most this many, 1 to 50, default 20; with around, split between before and after (up to 20 each), plus the message itself.
    #[serde(default = "default_preview_limit")]
    #[schemars(range(min = 1, max = PREVIEW_PAGE_SIZE))]
    pub limit: u16,
    /// Only one person: a Matrix user ID (starting with @) or a name (case-insensitive).
    #[schemars(length(min = 1, max = MESSAGE_FROM_BYTES))]
    pub from: Option<String>,
    /// Only messages that mention or reply to you.
    #[serde(default)]
    pub mentions_me: bool,
}

impl RoomMessagesInput {
    /// 给了 `around` 就是看前后，否则往前（或往后）翻。
    ///
    /// # Errors
    ///
    /// 哪一项不对就返回它的名字，放进错误的 `details.field`。
    pub fn method(self) -> Result<IpcMethod, &'static str> {
        if !(1..=PREVIEW_PAGE_SIZE).contains(&self.limit) {
            return Err("limit");
        }
        let Some(id) = self.around else {
            return Ok(IpcMethod::RoomHistory(IpcRoomHistoryRequest {
                room_id: self.room_id,
                before: self.before,
                after: self.after,
                limit: self.limit,
                from: self.from,
                mentions_me: self.mentions_me,
            }));
        };
        if self.before.is_some() || self.after.is_some() || self.from.is_some() || self.mentions_me
        {
            return Err("around");
        }
        // 前面多给一条：被点名时，前面说了什么通常更要紧。
        let after = self.limit / 2;
        Ok(IpcMethod::MessagesAround(IpcMessagesAroundRequest {
            room_id: self.room_id,
            id,
            before: (self.limit - after).min(AROUND_MESSAGES),
            after: after.min(AROUND_MESSAGES),
        }))
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WaitMessagesInput {
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub session_id: String,
    #[schemars(length(max = agent_room_agent_client::CURSOR_BYTES))]
    pub after_event_id: Option<String>,
    #[schemars(length(max = ROOM_ID_BYTES))]
    pub room_id: Option<String>,
    #[serde(default = "default_preview_limit")]
    #[schemars(range(min = 1, max = PREVIEW_PAGE_SIZE))]
    pub limit: u16,
    /// Omit to block until messages arrive; 0 checks once; a positive number caps the wait in seconds (up to a day).
    #[serde(default)]
    #[schemars(range(min = 0, max = agent_room_agent_client::MAX_EXPLICIT_WAIT_SECONDS))]
    pub wait_seconds: Option<u32>,
    /// Which messages wake you: related (default, ones that concern you: people's messages unless they address someone else, agents' only when they mention or reply to you), mentions (ones that mention or reply to you) or all (anything from others).
    #[serde(default)]
    pub wake: Option<WakeInput>,
    /// Wake when any of these people speaks (Matrix user IDs, up to 200); overrides wake.
    #[serde(default)]
    #[schemars(length(max = MENTIONS))]
    pub from: Vec<String>,
    /// Wake once all of these people have spoken (Matrix user IDs, up to 200); just "mentioned" means the people your last message mentioned (@everyone doesn't count).
    #[serde(default)]
    #[schemars(length(max = MENTIONS))]
    pub wait_for: Vec<String>,
    /// Wake when someone replies to this message (messageId).
    #[serde(default)]
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub reply_to: Option<String>,
    /// Once something arrives, wait this many seconds of quiet before handing it over: 0 to 30, default 5; 0 hands it over at once.
    #[serde(default)]
    #[schemars(range(max = 30))]
    pub settle_seconds: Option<u64>,
    /// Hand over messages that didn't wake you once they have piled up this many minutes: 1 to 1440; off by default.
    #[serde(default)]
    #[schemars(range(min = 1, max = 1440))]
    pub digest_minutes: Option<u64>,
    /// Give only messages that mention or reply to you (the owner's too); the rest count in skipped. Can't be combined with wake, from, waitFor, replyTo or digestMinutes.
    #[serde(default)]
    pub mentions_only: bool,
}

impl WaitMessagesInput {
    /// 等消息的规则（`specs/agent-reading/waiting.md`）。
    ///
    /// # Errors
    ///
    /// 哪一项不对就返回它的名字，放进错误的 `details.field`。
    pub fn rules(&self) -> Result<WaitRules, &'static str> {
        WaitParams {
            wake: self.wake.map(WakeRule::from),
            from: self.from.clone(),
            wait_for: self.wait_for.clone(),
            reply_to: self.reply_to.clone(),
            settle_seconds: self.settle_seconds,
            digest_minutes: self.digest_minutes,
            mentions_only: self.mentions_only,
        }
        .parse()
    }
}

/// Which messages wake you.
#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WakeInput {
    Related,
    Mentions,
    All,
}

impl From<WakeInput> for WakeRule {
    fn from(wake: WakeInput) -> Self {
        match wake {
            WakeInput::Related => Self::Related,
            WakeInput::Mentions => Self::Mentions,
            WakeInput::All => Self::All,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListHandoffsInput {
    /// This task's sessionId (from `agent_room_join` or `agent_room_open_session`).
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub session_id: String,
    /// How many pending cloud handoffs to return, 1 to 100; their content is not opened.
    #[serde(default = "default_handoff_limit")]
    #[schemars(range(min = 1, max = HANDOFF_PAGE_SIZE))]
    pub limit: u16,
}

impl From<ListHandoffsInput> for IpcListHandoffsRequest {
    fn from(input: ListHandoffsInput) -> Self {
        Self { limit: input.limit }
    }
}

const fn default_handoff_limit() -> u16 {
    DEFAULT_HANDOFF_LIMIT
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetPresenceInput {
    /// This task's sessionId (from `agent_room_join` or `agent_room_open_session`).
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub session_id: String,
    /// Matrix room ID.
    #[schemars(length(max = ROOM_ID_BYTES))]
    pub room_id: String,
    /// Optional list of agent UUIDs; an empty list gives the usual roster (including recently offline agents).
    #[serde(default)]
    #[schemars(length(max = PRESENCE_TARGETS), inner(length(equal = UUID_TEXT_CHARACTERS)))]
    pub agent_ids: Vec<String>,
    /// Also show automatically archived identities; they come back on their own with a new connection.
    #[serde(default)]
    pub include_archived: bool,
    /// The nextCursor from the previous page.
    pub after_agent_id: Option<String>,
    #[serde(default = "default_presence_limit")]
    #[schemars(range(min = 1, max = 100))]
    pub limit: u16,
}

const fn default_presence_limit() -> u16 {
    100
}

impl From<GetPresenceInput> for IpcGetPresenceRequest {
    fn from(input: GetPresenceInput) -> Self {
        Self {
            room_id: input.room_id,
            agent_ids: input.agent_ids,
            include_archived: input.include_archived,
            after_agent_id: input.after_agent_id,
            limit: input.limit,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[expect(
    clippy::struct_field_names,
    reason = "这些字段均为标识，名称须与公开 MCP 参数保持一致"
)]
pub struct OpenContentInput {
    /// This task's sessionId (from `agent_room_join` or `agent_room_open_session`).
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub session_id: String,
    /// The room the content belongs to; omit for the default lobby.
    #[schemars(length(max = ROOM_ID_BYTES))]
    pub room_id: Option<String>,
    /// The content UUID from a message preview.
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub content_id: String,
}

impl From<OpenContentInput> for IpcOpenContentRequest {
    fn from(input: OpenContentInput) -> Self {
        Self {
            room_id: input.room_id,
            content_id: input.content_id,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageSensitivityInput {
    Normal,
    Sensitive,
    Restricted,
}

impl From<MessageSensitivityInput> for IpcMessageSensitivity {
    fn from(sensitivity: MessageSensitivityInput) -> Self {
        match sensitivity {
            MessageSensitivityInput::Normal => Self::Normal,
            MessageSensitivityInput::Sensitive => Self::Sensitive,
            MessageSensitivityInput::Restricted => Self::Restricted,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageProvenanceInput {
    Human,
    HumanConfirmedAgent,
    AutonomousAgent,
}

impl From<MessageProvenanceInput> for IpcMessageProvenance {
    fn from(provenance: MessageProvenanceInput) -> Self {
        match provenance {
            MessageProvenanceInput::Human => Self::Human,
            MessageProvenanceInput::HumanConfirmedAgent => Self::HumanConfirmedAgent,
            MessageProvenanceInput::AutonomousAgent => Self::AutonomousAgent,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SendMessageInput {
    /// This task's sessionId (from `agent_room_join` or `agent_room_open_session`).
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub session_id: String,
    /// true for ordinary chat: only body is needed, and the title and summary are generated for you.
    #[serde(default)]
    pub chat: bool,
    /// Matrix user IDs to mention, up to 200; chat messages only.
    #[serde(default)]
    #[schemars(length(max = MENTIONS), inner(length(max = 255)))]
    pub mentions: Vec<String>,
    /// @everyone: every person and agent in the room counts as mentioned. Chat messages only, and only in private rooms.
    #[serde(default)]
    pub mentions_everyone: bool,
    /// Optional `UUIDv7` idempotency key; when the result is unknown or the binding is pending, retries must reuse the same value.
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub submission_id: Option<String>,
    /// The automation grant UUID, required for autonomous sends; leave it out for manual or individually confirmed sends.
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub automation_grant_id: Option<String>,
    /// Matrix room ID.
    #[schemars(length(max = ROOM_ID_BYTES))]
    pub room_id: String,
    /// Preview title, up to 120 characters.
    #[schemars(length(max = TITLE_CHARACTERS))]
    #[serde(default)]
    pub title: String,
    /// Preview summary, up to 500 characters.
    #[schemars(length(max = SUMMARY_CHARACTERS))]
    #[serde(default)]
    pub summary: String,
    /// The full body, currently up to 48 KiB per message.
    #[schemars(length(max = INLINE_TEXT_BYTES))]
    pub body: String,
    /// Body media type, for example text/markdown.
    #[schemars(length(max = MEDIA_TYPE_BYTES))]
    #[serde(default = "default_chat_media_type")]
    pub media_type: String,
    /// Optional BCP 47 language tag.
    #[schemars(length(max = LANGUAGE_BYTES))]
    pub language: Option<String>,
    /// Content sensitivity.
    #[serde(default = "default_chat_sensitivity")]
    pub sensitivity: MessageSensitivityInput,
    /// Risk flags in lower `snake_case`.
    #[serde(default)]
    #[schemars(length(max = RISK_FLAGS), inner(length(max = RISK_FLAG_BYTES)))]
    pub risk_flags: Vec<String>,
    /// Who is behind the message: a person, an agent the person confirmed, or an autonomous agent.
    pub provenance: MessageProvenanceInput,
    /// Optional UUID of the message you are replying to.
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub reply_to_message_id: Option<String>,
}

impl From<SendMessageInput> for IpcSendMessageRequest {
    fn from(input: SendMessageInput) -> Self {
        let (title, summary) = if input.chat {
            Self::chat_title_and_summary(&input.body)
        } else {
            (input.title, input.summary)
        };
        Self {
            chat: input.chat,
            mentions: input.mentions,
            mentions_everyone: input.mentions_everyone,
            submission_id: input.submission_id,
            automation_grant_id: input.automation_grant_id,
            room_id: input.room_id,
            title,
            summary,
            body: input.body,
            media_type: input.media_type,
            language: input.language,
            sensitivity: input.sensitivity.into(),
            risk_flags: input.risk_flags,
            provenance: input.provenance.into(),
            reply_to_message_id: input.reply_to_message_id,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HandoffInput {
    /// This task's sessionId (from `agent_room_join` or `agent_room_open_session`).
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub session_id: String,
    /// UUID of the handoff to consume or decline.
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub handoff_id: String,
}

impl From<HandoffInput> for IpcHandoffRequest {
    fn from(input: HandoffInput) -> Self {
        Self {
            handoff_id: input.handoff_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use agent_room_bridge_ipc::IpcMethod;
    use proptest::prelude::*;
    use rmcp::schemars;
    use serde_json::Value;

    use super::{
        GetPresenceInput, HandoffInput, ListHandoffsInput, ListPreviewsInput, OpenContentInput,
        OpenSessionInput, SendMessageInput, SessionInput,
    };

    #[test]
    fn 工具_schema_公开闭合对象与关键输入上限() {
        let send = serde_json::to_value(schemars::schema_for!(SendMessageInput))
            .expect("发送 Schema 可序列化");
        let list = serde_json::to_value(schemars::schema_for!(ListPreviewsInput))
            .expect("列表 Schema 可序列化");

        assert_eq!(send["additionalProperties"], Value::Bool(false));
        assert_eq!(send["properties"]["title"]["maxLength"], 120);
        assert_eq!(send["properties"]["summary"]["maxLength"], 500);
        assert_eq!(send["properties"]["body"]["maxLength"], 48 * 1_024);
        assert_eq!(send["properties"]["riskFlags"]["maxItems"], 16);
        assert_eq!(send["properties"]["riskFlags"]["items"]["maxLength"], 64);
        assert_eq!(list["properties"]["limit"]["minimum"], 1);
        assert_eq!(list["properties"]["limit"]["maximum"], 50);
    }

    proptest! {
        #[test]
        fn 任意_mcp_json_只能被拒绝或进入_ipc_二次校验(bytes in prop::collection::vec(any::<u8>(), 0..8_192)) {
            validate_if_parsed::<OpenSessionInput, _>(&bytes, |input| IpcMethod::OpenHostSession(input.into()));
            validate_if_parsed::<SessionInput, _>(&bytes, |input| IpcMethod::CloseHostSession(input.into()));
            validate_if_parsed::<ListPreviewsInput, _>(&bytes, |input| IpcMethod::ListPreviews(input.into()));
            validate_if_parsed::<GetPresenceInput, _>(&bytes, |input| IpcMethod::GetPresence(input.into()));
            validate_if_parsed::<OpenContentInput, _>(&bytes, |input| IpcMethod::OpenContent(input.into()));
            validate_if_parsed::<SendMessageInput, _>(&bytes, |input| IpcMethod::SendMessage(input.into()));
            validate_if_parsed::<ListHandoffsInput, _>(&bytes, |input| IpcMethod::ListHandoffs(input.into()));
            validate_if_parsed::<HandoffInput, _>(&bytes, |input| IpcMethod::ConsumeHandoff(input.into()));
        }
    }

    fn validate_if_parsed<T, F>(bytes: &[u8], into_method: F)
    where
        T: serde::de::DeserializeOwned,
        F: FnOnce(T) -> IpcMethod,
    {
        if let Ok(input) = serde_json::from_slice::<T>(bytes) {
            let _ = into_method(input).validate();
        }
    }
}

fn default_chat_media_type() -> String {
    "text/plain".to_owned()
}
const fn default_chat_sensitivity() -> MessageSensitivityInput {
    MessageSensitivityInput::Normal
}

#[cfg(test)]
mod conversation_tests {
    use super::{ListPreviewsInput, SendMessageInput};
    use agent_room_bridge_ipc::{IpcMethod, IpcSendMessageRequest};

    #[test]
    fn 普通聊天可省略标题摘要并保留稳定提及和回复() {
        let input: SendMessageInput = serde_json::from_value(serde_json::json!({
            "sessionId": "01990d9e-8400-7000-8000-000000000010",
            "chat": true, "body": "你好\n一起讨论", "roomId": "!room:matrix.test", "mentions": ["@agent:matrix.test"],
            "replyToMessageId": "01990d9e-8400-7000-8000-000000000003", "provenance": "human_confirmed_agent"
        })).expect("聊天入参有效");
        let request: IpcSendMessageRequest = input.into();
        assert_eq!(request.title, "你好 一起讨论");
        assert_eq!(request.media_type, "text/plain");
        assert_eq!(request.mentions, ["@agent:matrix.test"]);
        assert!(request.reply_to_message_id.is_some());
        IpcMethod::SendMessage(request)
            .validate()
            .expect("聊天可发送");
    }

    #[test]
    fn 双向游标不能同时使用() {
        let input: ListPreviewsInput = serde_json::from_value(
            serde_json::json!({ "sessionId": "01990d9e-8400-7000-8000-000000000010", "afterEventId": "$a", "beforeEventId": "$b" }),
        )
        .expect("JSON 有效");
        assert!(IpcMethod::ListPreviews(input.into()).validate().is_err());
    }
}

/// Offering a task never enables automatic execution; the desktop owner must bind it.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterReceptionInput {
    #[serde(default)]
    pub host_type: ReceptionHostInput,
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub session_id: String,
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub task_id: Option<String>,
    #[schemars(length(min = 1, max = 4096))]
    pub workspace: String,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReceptionHostInput {
    #[default]
    Codex,
    ClaudeCode,
}
impl From<ReceptionHostInput> for agent_room_bridge_ipc::IpcReceptionHost {
    fn from(value: ReceptionHostInput) -> Self {
        match value {
            ReceptionHostInput::Codex => Self::Codex,
            ReceptionHostInput::ClaudeCode => Self::ClaudeCode,
        }
    }
}

/// 建私人房间（`specs/agent-rooms/design.md`）：用户要建房间时才调用。
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateRoomInput {
    /// Room name, 1 to 128 characters. Calling again in the same task with the same name returns to that room instead of creating another.
    #[schemars(length(min = 1, max = 128))]
    pub name: String,
    /// Your name in this room, with the same rules as `agent_room_join`'s displayName.
    #[schemars(length(min = 1, max = 128))]
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoomAccessInput {
    /// This task's sessionId (from `agent_room_join` or `agent_room_open_session`).
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub session_id: String,
    /// Which room: a room name (a name or slug from `agent_room_list_rooms`, or a message's roomName) or a message's roomId; omit for the room your session opened in (the first one this task joined).
    #[schemars(length(min = 1, max = ROOM_ID_BYTES))]
    pub room: Option<String>,
    /// Omit or show: only look, creating a code if there is none. replace: issue a new code; the old one stops working at once. disable: turn the code off.
    pub action: Option<RoomAccessActionInput>,
}

#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoomAccessActionInput {
    Show,
    Replace,
    Disable,
}

impl From<RoomAccessActionInput> for IpcRoomAccessAction {
    fn from(value: RoomAccessActionInput) -> Self {
        match value {
            RoomAccessActionInput::Show => Self::Show,
            RoomAccessActionInput::Replace => Self::Replace,
            RoomAccessActionInput::Disable => Self::Disable,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnswerKnockInput {
    /// This task's sessionId (from `agent_room_join` or `agent_room_open_session`).
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub session_id: String,
    /// Which room: a room name (a name or slug from `agent_room_list_rooms`, or a message's roomName) or a message's roomId; omit for the room your session opened in (the first one this task joined).
    #[schemars(length(min = 1, max = ROOM_ID_BYTES))]
    pub room: Option<String>,
    /// agentId of the knocking agent, copied exactly from the knocks of `agent_room_room_access`.
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub agent_id: String,
    /// true lets it in, false keeps it out.
    pub admit: bool,
}

impl From<AnswerKnockInput> for IpcAnswerKnockRequest {
    fn from(input: AnswerKnockInput) -> Self {
        Self {
            agent_id: input.agent_id,
            admit: input.admit,
            catalog_id: None,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InvitePersonInput {
    /// This task's sessionId (from `agent_room_join` or `agent_room_open_session`).
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub session_id: String,
    /// Which room: a room name (a name or slug from `agent_room_list_rooms`, or a message's roomName) or a message's roomId; omit for the room your session opened in (the first one this task joined).
    #[schemars(length(min = 1, max = ROOM_ID_BYTES))]
    pub room: Option<String>,
    /// The person's account ID: the one they copy from "Switch room" in Agent Room and send you; never guess it.
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub account_id: String,
}

impl From<InvitePersonInput> for IpcInvitePersonRequest {
    fn from(input: InvitePersonInput) -> Self {
        Self {
            account_id: input.account_id.to_ascii_lowercase(),
            catalog_id: None,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoveAgentInput {
    /// This task's sessionId (from `agent_room_join` or `agent_room_open_session`).
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub session_id: String,
    /// Which room: a room name (a name or slug from `agent_room_list_rooms`, or a message's roomName) or a message's roomId; omit for the room your session opened in (the first one this task joined).
    #[schemars(length(min = 1, max = ROOM_ID_BYTES))]
    pub room: Option<String>,
    /// agentId of the agent to remove: one that came in with the code or by knocking.
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub agent_id: String,
}

impl From<RemoveAgentInput> for IpcRemoveAgentRequest {
    fn from(input: RemoveAgentInput) -> Self {
        Self {
            agent_id: input.agent_id,
            catalog_id: None,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaveRoomInput {
    /// This task's sessionId (from `agent_room_join` or `agent_room_open_session`).
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub session_id: String,
    /// Room to leave: a room name (a name or slug from `agent_room_list_rooms`, or a message's roomName) or a message's roomId.
    #[schemars(length(min = 1, max = ROOM_ID_BYTES))]
    pub room: String,
}
