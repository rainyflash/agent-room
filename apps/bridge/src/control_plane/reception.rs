use super::{
    ControlPlaneHttpConfig, ControlPlaneHttpConfigurationError, configured_client,
    read_limited_response_body, signed_request_headers,
};
use agent_room_application::{
    ports::PortFuture,
    reception::{
        ReceptionControlFailure, ReceptionControlGateway, ReceptionRecord, ReceptionRequest,
    },
};
use agent_room_bridge_core::session::ControlPlaneRequestAuthorizer;
use std::sync::Arc;

pub struct HttpReceptionGateway {
    client: reqwest::Client,
    url: url::Url,
    authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
}
impl HttpReceptionGateway {
    /// # Errors
    /// Rejects invalid control-plane configuration before creating a signed transport.
    pub fn new(
        config: &ControlPlaneHttpConfig,
        authorizer: Arc<dyn ControlPlaneRequestAuthorizer>,
    ) -> Result<Self, ControlPlaneHttpConfigurationError> {
        let (client, url) = configured_client(config)?;
        Ok(Self {
            client,
            url,
            authorizer,
        })
    }
}
impl ReceptionControlGateway for HttpReceptionGateway {
    fn execute(
        &self,
        request: ReceptionRequest,
    ) -> PortFuture<'_, Result<ReceptionRecord, ReceptionControlFailure>> {
        Box::pin(async move {
            if !request.valid() {
                return Err(failure("reception.invalid_request", false));
            }
            let target = "/receptions/control";
            let body = serde_json::to_string(&request)
                .map_err(|_| failure("reception.invalid_request", false))?;
            let signed = self
                .authorizer
                .authorize("POST", target, &body)
                .await
                .map_err(|_| failure("reception.authorization_unavailable", true))?;
            let url = self
                .url
                .join(target.trim_start_matches('/'))
                .map_err(|_| failure("reception.invalid_request", false))?;
            let response = signed_request_headers(
                self.client
                    .post(url)
                    .header(reqwest::header::CONTENT_TYPE, "application/json"),
                &signed,
                "POST",
                target,
            )
            .map_err(|()| failure("reception.invalid_request", false))?
            .body(body)
            .send()
            .await
            .map_err(|_| failure("reception.unavailable", true))?;
            let status = response.status();
            if !status.is_success() {
                return Err(match status {
                    reqwest::StatusCode::CONFLICT => failure("reception.execution_conflict", false),
                    reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::UNAUTHORIZED => {
                        failure("reception.forbidden", false)
                    }
                    reqwest::StatusCode::NOT_FOUND => failure("reception.not_found", false),
                    _ => failure(
                        "reception.unavailable",
                        status.is_server_error()
                            || status == reqwest::StatusCode::TOO_MANY_REQUESTS,
                    ),
                });
            }
            let bytes = read_limited_response_body(response)
                .await
                .map_err(|()| failure("reception.invalid_response", false))?;
            let record: ReceptionRecord = serde_json::from_slice(&bytes)
                .map_err(|_| failure("reception.invalid_response", false))?;
            if record.agent_id != request.agent_id
                || record.catalog_id != request.catalog_id
                || record.room_id != request.room_id
                || record.instance_id != request.instance_id
                || record.run_id != request.run_id
                || !record.progress.valid()
            {
                return Err(failure("reception.invalid_response", false));
            }
            Ok(record)
        })
    }
}
fn failure(code: &'static str, retryable: bool) -> ReceptionControlFailure {
    ReceptionControlFailure { code, retryable }
}
