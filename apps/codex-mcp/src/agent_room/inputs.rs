use agent_room_bridge_ipc::{
    IpcGetPresenceRequest, IpcHandoffRequest, IpcListPreviewsRequest, IpcMessageProvenance,
    IpcMessageSensitivity, IpcOpenContentRequest, IpcPublishStatusRequest, IpcSendMessageRequest,
    IpcWorkStatus,
};
use rmcp::schemars;
use serde::Deserialize;

const DEFAULT_PREVIEW_LIMIT: u16 = 20;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListPreviewsInput {
    /// 可选 Matrix 房间 ID；省略时读取当前大厅。
    pub room_id: Option<String>,
    /// 上一页末尾的事件 ID。
    pub before_event_id: Option<String>,
    /// 返回数量，范围 1 到 50。
    #[serde(default = "default_preview_limit")]
    pub limit: u16,
}

impl From<ListPreviewsInput> for IpcListPreviewsRequest {
    fn from(input: ListPreviewsInput) -> Self {
        Self {
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
pub struct GetPresenceInput {
    /// Matrix 房间 ID。
    pub room_id: String,
    /// 可选 Agent UUID 列表；空数组表示房间内全部在线 Agent。
    #[serde(default)]
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
    /// 消息预览给出的内容 UUID。
    pub content_id: String,
}

impl From<OpenContentInput> for IpcOpenContentRequest {
    fn from(input: OpenContentInput) -> Self {
        Self {
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
    pub room_id: String,
    /// 要发布的工作状态。
    pub status: WorkStatusInput,
    /// 不含敏感细节的任务摘要。
    pub task_summary: Option<String>,
    /// 进度基点，0 到 10000；10000 表示 100%。
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
    /// 可选 `UUIDv7` 幂等标识；结果未知或绑定待定时，重试必须复用同一值。
    pub submission_id: Option<String>,
    /// Matrix 房间 ID。
    pub room_id: String,
    /// 预览标题，最多 120 个字符。
    pub title: String,
    /// 预览摘要，最多 500 个字符。
    pub summary: String,
    /// 完整正文，当前单次最多 48 KiB。
    pub body: String,
    /// 正文媒体类型，例如 text/markdown。
    pub media_type: String,
    /// 可选 BCP 47 语言标签。
    pub language: Option<String>,
    /// 内容敏感度。
    pub sensitivity: MessageSensitivityInput,
    /// 小写蛇形风险标记。
    #[serde(default)]
    pub risk_flags: Vec<String>,
    /// 消息行为来源。
    pub provenance: MessageProvenanceInput,
    /// 可选被回复消息 UUID。
    pub reply_to_message_id: Option<String>,
}

impl From<SendMessageInput> for IpcSendMessageRequest {
    fn from(input: SendMessageInput) -> Self {
        Self {
            submission_id: input.submission_id,
            room_id: input.room_id,
            title: input.title,
            summary: input.summary,
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
    pub handoff_id: String,
}

impl From<HandoffInput> for IpcHandoffRequest {
    fn from(input: HandoffInput) -> Self {
        Self {
            handoff_id: input.handoff_id,
        }
    }
}
