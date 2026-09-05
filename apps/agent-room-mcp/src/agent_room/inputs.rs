use agent_room_bridge_ipc::{
    IpcGetPresenceRequest, IpcHandoffRequest, IpcListHandoffsRequest, IpcListPreviewsRequest,
    IpcMessageProvenance, IpcMessageSensitivity, IpcOpenContentRequest, IpcPublishStatusRequest,
    IpcSendMessageRequest, IpcWorkStatus,
    limits::{
        EVENT_ID_BYTES, HANDOFF_PAGE_SIZE, INLINE_TEXT_BYTES, LANGUAGE_BYTES, MEDIA_TYPE_BYTES,
        PRESENCE_TARGETS, PREVIEW_PAGE_SIZE, PROGRESS_BASIS_POINTS, RISK_FLAG_BYTES, RISK_FLAGS,
        ROOM_ID_BYTES, SUMMARY_CHARACTERS, TASK_SUMMARY_CHARACTERS, TITLE_CHARACTERS,
        UUID_TEXT_CHARACTERS,
    },
};
use rmcp::schemars;
use serde::Deserialize;

const DEFAULT_PREVIEW_LIMIT: u16 = 20;
const DEFAULT_HANDOFF_LIMIT: u16 = 20;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListPreviewsInput {
    /// 增量读取此事件之后的消息，按到达顺序返回；不能与 beforeEventId 并用。
    #[schemars(length(max = EVENT_ID_BYTES))]
    pub after_event_id: Option<String>,
    /// 已开启对话时最多等待 25 秒；空房间首次等待可以省略 afterEventId。
    #[serde(default)]
    #[schemars(range(min = 0, max = 25))]
    pub wait_seconds: u8,
    /// 可选 Matrix 房间 ID；省略时读取当前大厅。
    #[schemars(length(max = ROOM_ID_BYTES))]
    pub room_id: Option<String>,
    /// 上一页末尾的事件 ID。
    #[schemars(length(max = EVENT_ID_BYTES))]
    pub before_event_id: Option<String>,
    /// 返回数量，范围 1 到 50。
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
        }
    }
}

const fn default_preview_limit() -> u16 {
    DEFAULT_PREVIEW_LIMIT
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListHandoffsInput {
    /// 返回待处理云端交接的数量，范围 1 到 100；正文不会被打开。
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
    /// Matrix 房间 ID。
    #[schemars(length(max = ROOM_ID_BYTES))]
    pub room_id: String,
    /// 可选 Agent UUID 列表；空数组表示房间内全部在线 Agent。
    #[serde(default)]
    #[schemars(length(max = PRESENCE_TARGETS), inner(length(equal = UUID_TEXT_CHARACTERS)))]
    pub agent_ids: Vec<String>,
}

impl From<GetPresenceInput> for IpcGetPresenceRequest {
    fn from(input: GetPresenceInput) -> Self {
        Self {
            room_id: input.room_id,
            agent_ids: input.agent_ids,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenContentInput {
    /// 内容所属房间，省略时使用默认大厅。
    #[schemars(length(max = ROOM_ID_BYTES))]
    pub room_id: Option<String>,
    /// 消息预览给出的内容 UUID。
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
pub enum WorkStatusInput {
    Offline,
    Idle,
    Working,
    WaitingInput,
    Blocked,
    Completed,
}

impl From<WorkStatusInput> for IpcWorkStatus {
    fn from(status: WorkStatusInput) -> Self {
        match status {
            WorkStatusInput::Offline => Self::Offline,
            WorkStatusInput::Idle => Self::Idle,
            WorkStatusInput::Working => Self::Working,
            WorkStatusInput::WaitingInput => Self::WaitingInput,
            WorkStatusInput::Blocked => Self::Blocked,
            WorkStatusInput::Completed => Self::Completed,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublishStatusInput {
    /// Matrix 房间 ID。
    #[schemars(length(max = ROOM_ID_BYTES))]
    pub room_id: String,
    /// 要发布的工作状态。
    pub status: WorkStatusInput,
    /// 不含敏感细节的任务摘要。
    #[schemars(length(max = TASK_SUMMARY_CHARACTERS))]
    pub task_summary: Option<String>,
    /// 进度基点，0 到 10000；10000 表示 100%。
    #[schemars(range(min = 0, max = PROGRESS_BASIS_POINTS))]
    pub progress_basis_points: Option<u16>,
}

impl From<PublishStatusInput> for IpcPublishStatusRequest {
    fn from(input: PublishStatusInput) -> Self {
        Self {
            room_id: input.room_id,
            status: input.status.into(),
            task_summary: input.task_summary,
            progress_basis_points: input.progress_basis_points,
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
    /// 普通聊天设为 true：只需正文，标题和摘要由接入层生成。
    #[serde(default)]
    pub chat: bool,
    /// 提及的 Matrix 用户身份，最多 8 个；只有聊天可以携带。
    #[serde(default)]
    #[schemars(length(max = 8), inner(length(max = 255)))]
    pub mentions: Vec<String>,
    /// 可选 `UUIDv7` 幂等标识；结果未知或绑定待定时，重试必须复用同一值。
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub submission_id: Option<String>,
    /// 自主发送必填的自动发言授权 UUID；人工或逐次确认发送不得携带。
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub automation_grant_id: Option<String>,
    /// Matrix 房间 ID。
    #[schemars(length(max = ROOM_ID_BYTES))]
    pub room_id: String,
    /// 预览标题，最多 120 个字符。
    #[schemars(length(max = TITLE_CHARACTERS))]
    #[serde(default)]
    pub title: String,
    /// 预览摘要，最多 500 个字符。
    #[schemars(length(max = SUMMARY_CHARACTERS))]
    #[serde(default)]
    pub summary: String,
    /// 完整正文，当前单次最多 48 KiB。
    #[schemars(length(max = INLINE_TEXT_BYTES))]
    pub body: String,
    /// 正文媒体类型，例如 text/markdown。
    #[schemars(length(max = MEDIA_TYPE_BYTES))]
    #[serde(default = "default_chat_media_type")]
    pub media_type: String,
    /// 可选 BCP 47 语言标签。
    #[schemars(length(max = LANGUAGE_BYTES))]
    pub language: Option<String>,
    /// 内容敏感度。
    #[serde(default = "default_chat_sensitivity")]
    pub sensitivity: MessageSensitivityInput,
    /// 小写蛇形风险标记。
    #[serde(default)]
    #[schemars(length(max = RISK_FLAGS), inner(length(max = RISK_FLAG_BYTES)))]
    pub risk_flags: Vec<String>,
    /// 消息行为来源。
    pub provenance: MessageProvenanceInput,
    /// 可选被回复消息 UUID。
    #[schemars(length(equal = UUID_TEXT_CHARACTERS))]
    pub reply_to_message_id: Option<String>,
}

impl From<SendMessageInput> for IpcSendMessageRequest {
    fn from(input: SendMessageInput) -> Self {
        let summary: String = input
            .body
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(500)
            .collect();
        Self {
            chat: input.chat,
            mentions: input.mentions,
            submission_id: input.submission_id,
            automation_grant_id: input.automation_grant_id,
            room_id: input.room_id,
            title: if input.chat {
                summary.chars().take(120).collect()
            } else {
                input.title
            },
            summary: if input.chat { summary } else { input.summary },
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
    /// 待消费或拒绝的交接 UUID。
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
        PublishStatusInput, SendMessageInput,
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
            validate_if_parsed::<ListPreviewsInput, _>(&bytes, |input| IpcMethod::ListPreviews(input.into()));
            validate_if_parsed::<GetPresenceInput, _>(&bytes, |input| IpcMethod::GetPresence(input.into()));
            validate_if_parsed::<OpenContentInput, _>(&bytes, |input| IpcMethod::OpenContent(input.into()));
            validate_if_parsed::<PublishStatusInput, _>(&bytes, |input| IpcMethod::PublishStatus(input.into()));
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
            serde_json::json!({ "afterEventId": "$a", "beforeEventId": "$b" }),
        )
        .expect("JSON 有效");
        assert!(IpcMethod::ListPreviews(input.into()).validate().is_err());
    }
}
