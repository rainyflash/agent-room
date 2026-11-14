mod device;
mod handlers;
mod models;
mod sub_rooms;

pub(crate) use models::{PermissionsBody, PermissionsResponse};

#[cfg(test)]
pub(crate) mod tests;

use std::sync::Arc;

use agent_room_application::{
    authentication::AuthenticationUseCases,
    devices::DeviceAuthorizationUseCases,
    ports::SecretFactory,
    private_rooms::{PrivateRoomUseCases, SubRoomUseCases},
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
    /// 子房间：建、读名单、三项设置（`specs/redesign/sub-rooms.md`）。
    pub(super) sub_rooms: Arc<dyn SubRoomUseCases>,
    pub(super) authentication: Arc<dyn AuthenticationUseCases>,
    /// 本机 Bridge 替账号建房、邀请人时核对设备签名（`specs/agent-rooms/design.md`）。
    pub(super) devices: Arc<dyn DeviceAuthorizationUseCases>,
    pub(super) secrets: Arc<dyn SecretFactory>,
    pub(super) trusted_origins: TrustedOrigins,
}

pub(crate) struct PrivateRoomHttpDependencies {
    pub(crate) rooms: Arc<dyn PrivateRoomUseCases>,
    pub(crate) sub_rooms: Arc<dyn SubRoomUseCases>,
    pub(crate) authentication: Arc<dyn AuthenticationUseCases>,
    pub(crate) devices: Arc<dyn DeviceAuthorizationUseCases>,
    pub(crate) secrets: Arc<dyn SecretFactory>,
}

impl PrivateRoomHttpState {
    pub(crate) fn new(
        dependencies: PrivateRoomHttpDependencies,
        frontend_origin: &url::Url,
        desktop_origins: &crate::config::DesktopOrigins,
    ) -> Self {
        Self {
            rooms: dependencies.rooms,
            sub_rooms: dependencies.sub_rooms,
            authentication: dependencies.authentication,
            devices: dependencies.devices,
            secrets: dependencies.secrets,
            trusted_origins: TrustedOrigins::new(frontend_origin, desktop_origins),
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
        .route("/private-rooms/{catalog_id}/origin", get(handlers::origin))
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
            "/private-rooms/{catalog_id}/members/{principal_id}/unban",
            post(handlers::unban),
        )
        .route(
            "/private-rooms/{catalog_id}/members/{principal_id}/permissions",
            put(handlers::update_permissions),
        )
        .route(
            "/private-rooms/{catalog_id}/owner",
            put(handlers::transfer_ownership),
        )
        .route("/private-rooms/{catalog_id}/name", put(handlers::rename))
        .route(
            "/private-rooms/{catalog_id}/sub-rooms",
            get(sub_rooms::list).post(sub_rooms::create),
        )
        .route(
            "/private-rooms/{catalog_id}/sub-rooms/{sub_room_id}",
            put(sub_rooms::revise),
        )
        .route(
            "/private-rooms/{catalog_id}/sub-rooms/{sub_room_id}/membership",
            post(sub_rooms::join).delete(sub_rooms::leave),
        )
        .route(
            "/private-rooms/{catalog_id}/sub-room-policy",
            get(sub_rooms::policy).put(sub_rooms::change_policy),
        )
        .route("/devices/current/private-rooms", post(device::create))
        .route(
            "/devices/current/private-rooms/{catalog_id}/invitations",
            post(device::invite),
        )
        .layer(DefaultBodyLimit::max(MAX_PRIVATE_ROOM_BODY_BYTES))
        .with_state(state)
}
