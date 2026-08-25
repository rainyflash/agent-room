use agent_room_application::{
    direct_sessions::{DirectContactView, DirectSessionView, OpenDirectSession},
    ports::DirectAgentProfile,
};
use agent_room_domain::{
    direct_sessions::{DirectContactPolicy, DirectPresenceDisclosure},
    ids::{AgentId, RoomCatalogId},
};
use serde::{Deserialize, Serialize};

use crate::features::resource_ids::parse_uuid_v7;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct OpenDirectSessionBody {
    target_agent_id: String,
}

impl OpenDirectSessionBody {
    pub(super) fn into_request(
        self,
        actor: agent_room_application::authentication::AuthenticatedPrincipal,
    ) -> Option<OpenDirectSession> {
        Some(OpenDirectSession {
            actor,
            target_agent_id: agent_id(&self.target_agent_id)?,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SetDirectBlockBody {
    pub(super) blocked: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DirectSessionResponse {
    catalog_id: String,
    room_instance_id: Option<String>,
    matrix_room_id: Option<String>,
    lifecycle: &'static str,
    version: i64,
    target: DirectAgentResponse,
    contact_policy: DirectContactPolicyResponse,
}

impl From<DirectSessionView> for DirectSessionResponse {
    fn from(view: DirectSessionView) -> Self {
        let instance = view.record.instance();
        Self {
            catalog_id: view.record.catalog().id().to_string(),
            room_instance_id: instance.map(|instance| instance.id().to_string()),
            matrix_room_id: instance.map(|instance| instance.matrix_room_id().as_str().to_owned()),
            lifecycle: view.record.session().lifecycle().as_str(),
            version: view.record.session().version().value(),
            target: DirectAgentResponse::from(view.target),
            contact_policy: DirectContactPolicyResponse::from(view.contact_policy),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DirectSessionListResponse {
    sessions: Vec<DirectSessionResponse>,
}

impl From<Vec<DirectSessionView>> for DirectSessionListResponse {
    fn from(sessions: Vec<DirectSessionView>) -> Self {
        Self {
            sessions: sessions
                .into_iter()
                .map(DirectSessionResponse::from)
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DirectContactResponse {
    target: DirectAgentResponse,
    contact_policy: DirectContactPolicyResponse,
}

impl From<DirectContactView> for DirectContactResponse {
    fn from(view: DirectContactView) -> Self {
        Self {
            target: DirectAgentResponse::from(view.target),
            contact_policy: DirectContactPolicyResponse::from(view.contact_policy),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectAgentResponse {
    agent_id: String,
    matrix_user_id: String,
    display_name: String,
    avatar_content_id: Option<String>,
}

impl From<DirectAgentProfile> for DirectAgentResponse {
    fn from(profile: DirectAgentProfile) -> Self {
        Self {
            agent_id: profile.agent_id.to_string(),
            matrix_user_id: profile.matrix_user_id.as_str().to_owned(),
            display_name: profile.display_name,
            avatar_content_id: profile
                .avatar_content_id
                .map(|content_id| content_id.to_string()),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectContactPolicyResponse {
    principal_blocks_agent: bool,
    agent_blocks_principal: bool,
    delivery_allowed: bool,
    presence_disclosure: &'static str,
}

impl From<DirectContactPolicy> for DirectContactPolicyResponse {
    fn from(policy: DirectContactPolicy) -> Self {
        let presence_disclosure = match policy.exact_presence_disclosure() {
            DirectPresenceDisclosure::Coarse => "coarse",
            DirectPresenceDisclosure::Hidden => "hidden",
        };
        Self {
            principal_blocks_agent: policy.principal_blocks_agent(),
            agent_blocks_principal: policy.agent_blocks_principal(),
            delivery_allowed: policy.delivery_allowed(),
            presence_disclosure,
        }
    }
}

pub(super) fn catalog_id(value: &str) -> Option<RoomCatalogId> {
    parse_uuid_v7(value).map(RoomCatalogId::from_uuid).ok()
}

pub(super) fn agent_id(value: &str) -> Option<AgentId> {
    parse_uuid_v7(value).map(AgentId::from_uuid).ok()
}
