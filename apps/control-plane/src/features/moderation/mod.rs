mod handlers;
mod models;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use agent_room_application::{
    authentication::AuthenticationUseCases, moderation::ModerationUseCases,
};

use crate::features::authentication::TrustedOrigins;
use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{delete, get},
};

const MAX_MODERATION_BODY_BYTES: usize = 16 * 1_024;

#[derive(Clone)]
pub(crate) struct ModerationHttpState {
    pub(super) moderation: Arc<dyn ModerationUseCases>,
    pub(super) authentication: Arc<dyn AuthenticationUseCases>,
    pub(super) trusted_origins: TrustedOrigins,
}

impl ModerationHttpState {
    pub(crate) fn new(
        moderation: Arc<dyn ModerationUseCases>,
        authentication: Arc<dyn AuthenticationUseCases>,
        frontend_origin: &url::Url,
        desktop_origin: &url::Url,
    ) -> Self {
        Self {
            moderation,
            authentication,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origin),
        }
    }
}

pub(crate) fn router(state: ModerationHttpState) -> Router {
    Router::new()
        .route(
            "/moderation/cases",
            get(handlers::list_cases).post(handlers::submit_report),
        )
        .route("/moderation/audit", get(handlers::list_audit))
        .route(
            "/rooms/{catalog_id}/moderation/cases",
            get(handlers::list_room_cases),
        )
        .route(
            "/rooms/{catalog_id}/moderation/capabilities",
            get(handlers::inspect_capabilities),
        )
        .route(
            "/rooms/{catalog_id}/moderation/actions",
            get(handlers::list_actions).post(handlers::apply_action),
        )
        .route(
            "/moderation/actions/{action_id}",
            delete(handlers::reverse_action),
        )
        .layer(DefaultBodyLimit::max(MAX_MODERATION_BODY_BYTES))
        .with_state(state)
}
