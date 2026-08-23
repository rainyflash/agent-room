mod agents;
mod error;
mod migrations;
mod principals;

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
