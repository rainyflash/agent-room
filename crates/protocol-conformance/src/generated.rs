// 本文件由 tools/protocol-codegen.ts 生成，禁止手工修改。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorRef {
    pub agent: AgentRef,
    pub instance_id: String,
    pub provenance: Provenance,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRef {
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub display_name: String,
    pub matrix_user_id: String,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatusEvent {
    pub actor: ActorRef,
    pub correlation_id: String,
    pub created_at: String,
    pub event_type: String,
    pub id: String,
    pub lease_expires_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    pub schema_version: String,
    pub signature: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    pub status: AgentWorkStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_summary: Option<String>,
    pub visibility: AgentStatusVisibility,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentStatusVisibility {
    #[serde(rename = "coarse")]
    Coarse,
    #[serde(rename = "detailed")]
    Detailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentWorkStatus {
    #[serde(rename = "offline")]
    Offline,
    #[serde(rename = "idle")]
    Idle,
    #[serde(rename = "working")]
    Working,
    #[serde(rename = "waiting_input")]
    WaitingInput,
    #[serde(rename = "blocked")]
    Blocked,
    #[serde(rename = "completed")]
    Completed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityManifest {
    pub event_types: Vec<String>,
    pub features: Vec<String>,
    pub protocol_versions: Vec<String>,
    pub schema_version: String,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientContentEncryption {
    pub algorithm: String,
    pub context_id: String,
    pub key_base64_url: String,
    pub nonce_base64_url: String,
    pub plaintext_size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentRef {
    pub content_id: String,
    pub digest_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption: Option<ClientContentEncryption>,
    pub fetch_mode: String,
    pub media_type: String,
    pub size_bytes: u64,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCategory {
    #[serde(rename = "validation")]
    Validation,
    #[serde(rename = "authentication")]
    Authentication,
    #[serde(rename = "authorization")]
    Authorization,
    #[serde(rename = "conflict")]
    Conflict,
    #[serde(rename = "transient")]
    Transient,
    #[serde(rename = "unknown_commit")]
    UnknownCommit,
    #[serde(rename = "dependency_unavailable")]
    DependencyUnavailable,
    #[serde(rename = "incompatible_version")]
    IncompatibleVersion,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorEnvelope {
    pub category: ErrorCategory,
    pub code: String,
    pub correlation_id: String,
    pub details: BTreeMap<String, serde_json::Value>,
    pub message: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u64>,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandoffPermission {
    #[serde(rename = "read_text")]
    ReadText,
    #[serde(rename = "read_attachments")]
    ReadAttachments,
    #[serde(rename = "include_metadata")]
    IncludeMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandoffPurpose {
    #[serde(rename = "inspect")]
    Inspect,
    #[serde(rename = "summarize")]
    Summarize,
    #[serde(rename = "reply_draft")]
    ReplyDraft,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffReceiptEvent {
    pub actor: ActorRef,
    pub correlation_id: String,
    pub created_at: String,
    pub event_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    pub id: String,
    pub requester_instance_id: String,
    pub schema_version: String,
    pub signature: String,
    pub status: HandoffReceiptStatus,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandoffReceiptStatus {
    #[serde(rename = "delivered")]
    Delivered,
    #[serde(rename = "consumed")]
    Consumed,
    #[serde(rename = "declined")]
    Declined,
    #[serde(rename = "revoked")]
    Revoked,
    #[serde(rename = "expired")]
    Expired,
    #[serde(rename = "failed")]
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffRequestEvent {
    pub actor: ActorRef,
    pub approved_at: String,
    pub approved_by_principal_id: String,
    pub content: ContentRef,
    pub correlation_id: String,
    pub created_at: String,
    pub event_type: String,
    pub expires_at: String,
    pub id: String,
    pub permissions: Vec<HandoffPermission>,
    pub purpose: HandoffPurpose,
    pub risk_flags: Vec<String>,
    pub schema_version: String,
    pub signature: String,
    pub source: HandoffSource,
    pub target_agent_id: String,
    pub target_instance_id: String,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffSource {
    pub actor: ActorRef,
    pub event_id: String,
    pub message_id: String,
    pub room_id: String,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePreview {
    pub content_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub risk_flags: Vec<String>,
    pub sensitivity: MessageSensitivity,
    pub summary: String,
    pub title: String,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePreviewEvent {
    pub actor: ActorRef,
    pub content: ContentRef,
    pub correlation_id: String,
    pub created_at: String,
    pub event_type: String,
    pub id: String,
    pub preview: MessagePreview,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation: Option<MessageRelation>,
    pub room_id: String,
    pub schema_version: String,
    pub signature: String,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRelation {
    pub kind: String,
    pub target_message_id: String,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRevisionEvent {
    pub actor: ActorRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<ContentRef>,
    pub correlation_id: String,
    pub created_at: String,
    pub event_type: String,
    pub id: String,
    pub kind: MessageRevisionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<MessagePreview>,
    pub room_id: String,
    pub schema_version: String,
    pub signature: String,
    pub target_message_id: String,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRevisionKind {
    #[serde(rename = "replace")]
    Replace,
    #[serde(rename = "redact")]
    Redact,
    #[serde(rename = "moderate")]
    Moderate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageSensitivity {
    #[serde(rename = "normal")]
    Normal,
    #[serde(rename = "sensitive")]
    Sensitive,
    #[serde(rename = "restricted")]
    Restricted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provenance {
    #[serde(rename = "human")]
    Human,
    #[serde(rename = "human_confirmed_agent")]
    HumanConfirmedAgent,
    #[serde(rename = "autonomous_agent")]
    AutonomousAgent,
}
