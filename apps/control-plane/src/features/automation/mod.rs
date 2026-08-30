mod handlers;
mod models;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use agent_room_application::{
    authentication::AuthenticationUseCases, automation::AutomationUseCases,
    devices::DeviceAuthorizationUseCases, ports::SecretFactory,
};

use crate::features::authentication::TrustedOrigins;
use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{delete, get, post},
};

const MAX_AUTOMATION_BODY_BYTES: usize = 16 * 1_024;

#[derive(Clone)]
pub(crate) struct AutomationHttpState {
    pub(super) automation: Arc<dyn AutomationUseCases>,
    pub(super) authentication: Arc<dyn AuthenticationUseCases>,
    pub(super) devices: Arc<dyn DeviceAuthorizationUseCases>,
    pub(super) secrets: Arc<dyn SecretFactory>,
    pub(super) trusted_origins: TrustedOrigins,
}

pub(crate) struct AutomationHttpDependencies {
    pub(crate) automation: Arc<dyn AutomationUseCases>,
    pub(crate) authentication: Arc<dyn AuthenticationUseCases>,
    pub(crate) devices: Arc<dyn DeviceAuthorizationUseCases>,
    pub(crate) secrets: Arc<dyn SecretFactory>,
}

impl AutomationHttpState {
    pub(crate) fn new(
        dependencies: AutomationHttpDependencies,
        frontend_origin: &url::Url,
        desktop_origin: &url::Url,
    ) -> Self {
        Self {
            automation: dependencies.automation,
            authentication: dependencies.authentication,
            devices: dependencies.devices,
            secrets: dependencies.secrets,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origin),
        }
    }
}

pub(crate) fn router(state: AutomationHttpState) -> Router {
    Router::new()
        .route(
            "/automation-grants",
            get(handlers::list).post(handlers::create),
        )
        .route("/automation-grants/{grant_id}", delete(handlers::revoke))
        .route(
            "/automation-grants/{grant_id}/authorizations",
            post(handlers::authorize_send),
        )
        .layer(DefaultBodyLimit::max(MAX_AUTOMATION_BODY_BYTES))
        .with_state(state)
}
