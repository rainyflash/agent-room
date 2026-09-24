mod accounts;
mod agent_cards;
mod agent_creation;
mod agent_instances;
mod agent_memberships;
mod agent_retirement;
mod agents;
mod authentication;
mod automation;
mod content;
mod devices;
mod error;
mod handoffs;
mod inbox;
mod migrations;
mod moderation;
mod network_agent_inbox;
mod network_agent_submissions;
mod network_agents;
mod outbox;
mod principals;
mod projections;
mod reception;
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
