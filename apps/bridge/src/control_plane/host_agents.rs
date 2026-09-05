use agent_room_bridge_core::onboarding::HostAgentRegistrationGateway;
use agent_room_domain::ids::AgentCreationRequestId;

use super::{
    BridgeDefaultAgent, ControlPlaneOnboardingFailureKind, ControlPlaneOnboardingResult,
    PortFuture, ReqwestControlPlaneOnboardingGateway, decode_default_agent_response,
    map_onboarding_session_failure, onboarding_failure, onboarding_transport_failure,
    signed_onboarding_request,
};

impl HostAgentRegistrationGateway for ReqwestControlPlaneOnboardingGateway {
    fn create_host_agent<'a>(
        &'a self,
        session_key: AgentCreationRequestId,
        display_name: &'a str,
    ) -> PortFuture<'a, ControlPlaneOnboardingResult<BridgeDefaultAgent>> {
        Box::pin(async move {
            let path = format!("/devices/current/host-agents/{session_key}");
            let body = serde_json::json!({ "displayName": display_name }).to_string();
            let authorized = self
                .authorizer
                .authorize("PUT", &path, &body)
                .await
                .map_err(map_onboarding_session_failure)?;
            let url = self
                .base_url
                .join(path.trim_start_matches('/'))
                .map_err(|_| onboarding_failure(ControlPlaneOnboardingFailureKind::Internal))?;
            let request =
                signed_onboarding_request(self.client.put(url), &authorized, "PUT", &path)?;
            let response = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body)
                .send()
                .await
                .map_err(onboarding_transport_failure)?;
            decode_default_agent_response(response).await
        })
    }
}
