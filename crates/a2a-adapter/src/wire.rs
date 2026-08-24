use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireAgentCard {
    pub name: String,
    pub description: String,
    pub supported_interfaces: Vec<WireAgentInterface>,
    pub provider: Option<WireAgentProvider>,
    pub version: String,
    pub capabilities: WireAgentCapabilities,
    #[serde(default)]
    pub security_schemes: BTreeMap<String, WireSecurityScheme>,
    #[serde(default)]
    pub security_requirements: Vec<WireSecurityRequirement>,
    pub default_input_modes: Vec<String>,
    pub default_output_modes: Vec<String>,
    pub skills: Vec<WireAgentSkill>,
    #[serde(default)]
    pub signatures: Vec<WireAgentCardSignature>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireAgentInterface {
    pub url: String,
    pub protocol_binding: String,
    pub tenant: Option<String>,
    pub protocol_version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WireAgentProvider {
    pub url: String,
    pub organization: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireAgentCapabilities {
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub push_notifications: bool,
    #[serde(default)]
    pub extended_agent_card: bool,
    #[serde(default)]
    pub extensions: Vec<WireAgentExtension>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WireAgentExtension {
    pub uri: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireAgentSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    #[serde(default)]
    pub input_modes: Vec<String>,
    #[serde(default)]
    pub output_modes: Vec<String>,
    #[serde(default)]
    pub security_requirements: Vec<WireSecurityRequirement>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WireSecurityScheme {
    #[serde(flatten)]
    pub variants: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WireSecurityRequirement {
    pub schemes: BTreeMap<String, WireStringList>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WireStringList {
    #[serde(default)]
    pub list: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WireAgentCardSignature {
    pub protected: String,
    pub signature: String,
    #[serde(default)]
    pub header: Option<Value>,
}
