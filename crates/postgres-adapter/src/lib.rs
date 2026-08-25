mod agent_cards;
mod agent_creation;
mod agent_instances;
mod agent_memberships;
mod agents;
mod authentication;
mod automation;
mod content;
mod devices;
mod error;
mod handoffs;
mod migrations;
mod moderation;
mod outbox;
mod principals;
mod projections;
mod rooms;
mod transaction;

use sqlx::PgPool;

pub use content::{
    ContentDownloadLimitPolicy, ContentDownloadLimitPolicyError, PostgresContentDownloadLimiter,
};
pub use error::MigrationFailure;
pub use migrations::run_migrations;

#[derive(Clone)]
pub struct PostgresRepositories {
    pool: PgPool,
}

impl PostgresRepositories {
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }
}
