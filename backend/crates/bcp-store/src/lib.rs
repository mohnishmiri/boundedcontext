//! PostgreSQL persistence: embedded migrations plus small, explicit query functions. Queries are checked at
//! run time (no compile-time database needed); the integration tests exercise every one of them.

mod audit;
mod events;
mod jobs;
mod tasks;
mod users;

use std::time::Duration;

use bcp_core::AppError;
use sqlx::postgres::{PgListener, PgPoolOptions};
use sqlx::PgPool;

pub use audit::{actions, NewAuditEvent};
pub use events::EventRow;
pub use jobs::{ArtifactRow, CancelOutcome, JobFilter, JobRow, NewArtifact, NewJob, PublishOutcome, PublishProgress};
pub use tasks::{ReapedTask, TaskRow};
pub use users::{store_tokens_on, tokens_for_update, SessionUser, TokenRow, UserRow};

pub use sqlx;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Channel carrying job ids whenever a job row or its log changes (see migrations).
pub const JOBS_CHANNEL: &str = "bcp_jobs";
/// Channel carrying the task kind whenever a task becomes pending.
pub const TASKS_CHANNEL: &str = "bcp_tasks";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("record not found")]
    NotFound,
    #[error("an active job already exists: {0}")]
    DuplicateJob(uuid::Uuid),
    #[error("conflict: {0}")]
    Conflict(&'static str),
    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

impl StoreError {
    pub fn unique_violation(&self) -> Option<&str> {
        match self {
            StoreError::Db(sqlx::Error::Database(db)) if db.code().as_deref() == Some("23505") => db.constraint(),
            _ => None,
        }
    }
}

impl From<StoreError> for AppError {
    fn from(err: StoreError) -> Self {
        match err {
            StoreError::NotFound => AppError::not_found("Record"),
            StoreError::DuplicateJob(id) => AppError::new(
                bcp_core::ErrorCode::DuplicateJob,
                "A generation for this repository, branch and model is already in progress.",
            )
            .with_details(serde_json::json!({ "job_id": id })),
            StoreError::Conflict(msg) => AppError::conflict(msg),
            other => {
                tracing::error!(error = %other, "database operation failed");
                AppError::internal("A database error occurred. Please try again.")
            }
        }
    }
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

#[derive(Clone, Debug)]
pub struct Store {
    pool: PgPool,
}

impl Store {
    pub async fn connect(url: &str, max_connections: u32) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .min_connections(1)
            .acquire_timeout(Duration::from_secs(5))
            .idle_timeout(Duration::from_secs(300))
            .max_lifetime(Duration::from_secs(1800))
            .connect(url)
            .await?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn migrate(&self) -> Result<()> {
        MIGRATOR.run(&self.pool).await?;
        Ok(())
    }

    /// Readiness: the database answers and every embedded migration has been applied.
    pub async fn ready(&self) -> Result<bool> {
        let expected = MIGRATOR.iter().map(|m| m.version).max().unwrap_or(0);
        let applied: Option<i64> = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&self.pool)
            .await
            .or_else(|e| match &e {
                // Table missing: nothing applied yet.
                sqlx::Error::Database(db) if db.code().as_deref() == Some("42P01") => Ok(None),
                _ => Err(e),
            })?;
        Ok(applied.unwrap_or(0) >= expected)
    }

    pub async fn listener(&self, channels: &[&str]) -> Result<PgListener> {
        let mut listener = PgListener::connect_with(&self.pool).await?;
        listener.listen_all(channels.iter().copied()).await?;
        Ok(listener)
    }
}
