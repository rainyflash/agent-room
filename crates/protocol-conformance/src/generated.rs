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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentState {
    #[serde(rename = "idle")]
    Idle,
    #[serde(rename = "working")]
    Working,
    #[serde(rename = "waiting_for_user")]
    WaitingForUser,
    #[serde(rename = "blocked")]
    Blocked,
    #[serde(rename = "offline")]
    Offline,
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
    pub schema_version: String,
    pub signature: String,
    pub state: AgentState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, serde_json::Value>,
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
pub struct ContentRef {
    pub content_id: String,
    pub digest_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffRequestEvent {
    pub actor: ActorRef,
    pub content: ContentRef,
    pub correlation_id: String,
    pub created_at: String,
    pub event_type: String,
    pub expires_at: String,
    pub id: String,
    pub permissions: Vec<HandoffPermission>,
    pub schema_version: String,
    pub signature: String,
    pub target_instance_id: String,
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
    pub preview: String,
    pub room_id: String,
    pub schema_version: String,
    pub signature: String,
    #[serde(default, flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, serde_json::Value>,
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
