mod handlers;
mod models;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use agent_room_application::{
    authentication::AuthenticationUseCases, private_rooms::PrivateRoomUseCases,
};

use crate::features::authentication::TrustedOrigins;
use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{delete, get, post, put},
};

const MAX_PRIVATE_ROOM_BODY_BYTES: usize = 64 * 1_024;

#[derive(Clone)]
pub(crate) struct PrivateRoomHttpState {
    pub(super) rooms: Arc<dyn PrivateRoomUseCases>,
    pub(super) authentication: Arc<dyn AuthenticationUseCases>,
    pub(super) trusted_origins: TrustedOrigins,
}

impl PrivateRoomHttpState {
    pub(crate) fn new(
        rooms: Arc<dyn PrivateRoomUseCases>,
        authentication: Arc<dyn AuthenticationUseCases>,
        frontend_origin: &url::Url,
        desktop_origin: &url::Url,
    ) -> Self {
        Self {
            rooms,
            authentication,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origin),
        }
    }
}

pub(crate) fn router(state: PrivateRoomHttpState) -> Router {
    Router::new()
        .route("/private-rooms", get(handlers::list).post(handlers::create))
        .route(
            "/private-rooms/{catalog_id}",
            get(handlers::inspect).delete(handlers::archive),
        )
        .route(
            "/private-rooms/{catalog_id}/invitations",
            post(handlers::invite),
        )
        .route(
            "/private-rooms/{catalog_id}/membership/accept",
            post(handlers::accept),
        )
        .route(
            "/private-rooms/{catalog_id}/membership/decline",
            post(handlers::decline),
        )
        .route(
            "/private-rooms/{catalog_id}/membership/leave",
            post(handlers::leave_room),
        )
        .route(
            "/private-rooms/{catalog_id}/members/{principal_id}",
            delete(handlers::remove),
        )
        .route(
            "/private-rooms/{catalog_id}/members/{principal_id}/ban",
            post(handlers::ban),
        )
        .route(
            "/private-rooms/{catalog_id}/members/{principal_id}/permissions",
            put(handlers::update_permissions),
        )
        .route(
            "/private-rooms/{catalog_id}/owner",
            put(handlers::transfer_ownership),
        )
        .layer(DefaultBodyLimit::max(MAX_PRIVATE_ROOM_BODY_BYTES))
        .with_state(state)
}
