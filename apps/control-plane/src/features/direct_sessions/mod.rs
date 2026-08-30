mod handlers;
mod models;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use agent_room_application::{
    authentication::AuthenticationUseCases, direct_sessions::DirectSessionUseCases,
};

use crate::features::authentication::TrustedOrigins;
use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, put},
};

const MAX_DIRECT_SESSION_BODY_BYTES: usize = 8 * 1_024;

#[derive(Clone)]
pub(crate) struct DirectSessionHttpState {
    pub(super) sessions: Arc<dyn DirectSessionUseCases>,
    pub(super) authentication: Arc<dyn AuthenticationUseCases>,
    pub(super) trusted_origins: TrustedOrigins,
}

impl DirectSessionHttpState {
    pub(crate) fn new(
        sessions: Arc<dyn DirectSessionUseCases>,
        authentication: Arc<dyn AuthenticationUseCases>,
        frontend_origin: &url::Url,
        desktop_origin: &url::Url,
    ) -> Self {
        Self {
            sessions,
            authentication,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origin),
        }
    }
}

pub(crate) fn router(state: DirectSessionHttpState) -> Router {
    Router::new()
        .route("/direct-sessions", get(handlers::list).post(handlers::open))
        .route("/direct-sessions/{catalog_id}", get(handlers::inspect))
        .route(
            "/direct-contacts/{agent_id}/block",
            put(handlers::set_block),
        )
        .layer(DefaultBodyLimit::max(MAX_DIRECT_SESSION_BODY_BYTES))
        .with_state(state)
}
