mod agent_creation;
mod agent_instances;
mod agent_memberships;
mod agents;
mod authentication;
mod devices;
mod error;
mod migrations;
mod outbox;
mod principals;
mod projections;
mod transaction;

use sqlx::PgPool;

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
