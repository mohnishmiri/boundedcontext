use serde_json::Value;
use uuid::Uuid;

use crate::{Result, Store};

/// Audit actions recorded by the API and worker.
pub mod actions {
    pub const USER_SIGNED_IN: &str = "user_signed_in";
    pub const USER_SIGNED_OUT: &str = "user_signed_out";
    pub const GENERATION_REQUESTED: &str = "generation_requested";
    pub const GENERATION_CANCELLED: &str = "generation_cancelled";
    pub const GENERATION_SUCCEEDED: &str = "generation_succeeded";
    pub const GENERATION_FAILED: &str = "generation_failed";
    pub const PUBLISH_REQUESTED: &str = "publish_requested";
    pub const COMMIT_CREATED: &str = "commit_created";
    pub const BRANCH_CREATED: &str = "branch_created";
    pub const PULL_REQUEST_CREATED: &str = "pull_request_created";
    pub const PUBLISH_FAILED: &str = "publish_failed";
}

#[derive(Debug, Clone, Default)]
pub struct NewAuditEvent<'a> {
    pub actor_user_id: Option<Uuid>,
    pub actor_login: Option<&'a str>,
    pub action: &'a str,
    pub job_id: Option<Uuid>,
    pub owner: Option<&'a str>,
    pub repo: Option<&'a str>,
    pub details: Value,
    pub request_id: Option<&'a str>,
}

impl Store {
    /// Appends an audit record. Failures are logged by the caller but never block the user action.
    pub async fn audit(&self, e: NewAuditEvent<'_>) -> Result<()> {
        let details = if e.details.is_null() { Value::Object(Default::default()) } else { e.details };
        sqlx::query(
            "INSERT INTO audit_events (actor_user_id, actor_login, action, job_id, owner, repo, details, request_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(e.actor_user_id)
        .bind(e.actor_login)
        .bind(e.action)
        .bind(e.job_id)
        .bind(e.owner)
        .bind(e.repo)
        .bind(details)
        .bind(e.request_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Like [`Store::audit`] but logs instead of returning errors.
    pub async fn audit_best_effort(&self, e: NewAuditEvent<'_>) {
        let action = e.action;
        if let Err(err) = self.audit(e).await {
            tracing::error!(error = %err, action, "failed to write audit event");
        }
    }

    pub async fn audit_actions_for_job(&self, job_id: Uuid) -> Result<Vec<String>> {
        Ok(sqlx::query_scalar("SELECT action FROM audit_events WHERE job_id = $1 ORDER BY id")
            .bind(job_id)
            .fetch_all(&self.pool)
            .await?)
    }
}
