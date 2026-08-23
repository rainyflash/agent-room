mod agents;
mod authentication;
mod error;
mod migrations;
mod outbox;
mod principals;
mod projections;

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
