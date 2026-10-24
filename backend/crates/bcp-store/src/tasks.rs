//! Durable task queue on PostgreSQL: `FOR UPDATE SKIP LOCKED` claims, heartbeat leases, a reaper for leases
//! of crashed workers, and rescheduling with a delay for transient failures.

use bcp_core::domain::TaskKind;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{Result, Store};

#[derive(Debug, Clone, FromRow)]
pub struct TaskRow {
    pub id: Uuid,
    pub job_id: Uuid,
    pub kind: String,
    pub state: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub run_after: DateTime<Utc>,
    pub lease_owner: Option<String>,
    pub lease_until: Option<DateTime<Utc>>,
}

impl TaskRow {
    pub fn kind(&self) -> Option<TaskKind> {
        TaskKind::parse(&self.kind)
    }
    pub fn attempts_left(&self) -> bool {
        self.attempts < self.max_attempts
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct ReapedTask {
    pub id: Uuid,
    pub job_id: Uuid,
    pub kind: String,
    /// true: returned to pending; false: attempts exhausted and failed.
    pub requeued: bool,
}

const TASK_COLUMNS: &str = "id, job_id, kind, state, attempts, max_attempts, run_after, lease_owner, lease_until";

impl Store {
    /// Claims the next runnable task, if any, and leases it to `worker_id`. The worker's configured attempt
    /// limits replace the defaults the API stored, so retries follow worker configuration.
    pub async fn claim_task(
        &self,
        worker_id: &str,
        lease_secs: i64,
        generate_max: i32,
        publish_max: i32,
    ) -> Result<Option<TaskRow>> {
        let sql = format!(
            r#"
            UPDATE job_tasks t
               SET state = 'running', attempts = t.attempts + 1, lease_owner = $1,
                   lease_until = now() + make_interval(secs => $2), updated_at = now(),
                   max_attempts = greatest(1, CASE t.kind WHEN 'generate' THEN $3 ELSE $4 END)
             WHERE t.id = (
                   SELECT id FROM job_tasks
                    WHERE state = 'pending' AND run_after <= now()
                    ORDER BY run_after, created_at
                    FOR UPDATE SKIP LOCKED
                    LIMIT 1)
            RETURNING {TASK_COLUMNS}
            "#
        );
        Ok(sqlx::query_as::<_, TaskRow>(&sql)
            .bind(worker_id)
            .bind(lease_secs as f64)
            .bind(generate_max)
            .bind(publish_max)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Extends the lease. `false` means the lease was lost (reaped, cancelled) and the work must stop.
    pub async fn heartbeat_task(&self, task_id: Uuid, worker_id: &str, lease_secs: i64) -> Result<bool> {
        let r = sqlx::query(
            "UPDATE job_tasks SET lease_until = now() + make_interval(secs => $3), updated_at = now()
              WHERE id = $1 AND lease_owner = $2 AND state = 'running'",
        )
        .bind(task_id)
        .bind(worker_id)
        .bind(lease_secs as f64)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() == 1)
    }

    /// Marks a task finished (`succeeded`, `failed` or `cancelled`).
    pub async fn finish_task(&self, task_id: Uuid, worker_id: &str, state: &str, error: Option<&str>) -> Result<()> {
        debug_assert!(matches!(state, "succeeded" | "failed" | "cancelled"));
        sqlx::query(
            "UPDATE job_tasks SET state = $3, last_error = coalesce($4, last_error), lease_owner = NULL,
                    lease_until = NULL, updated_at = now()
              WHERE id = $1 AND lease_owner = $2",
        )
        .bind(task_id)
        .bind(worker_id)
        .bind(state)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Returns a task to the queue after `delay_secs` (transient failure). The attempt stays counted.
    pub async fn reschedule_task(&self, task_id: Uuid, worker_id: &str, delay_secs: f64, error: &str) -> Result<()> {
        sqlx::query(
            "UPDATE job_tasks SET state = 'pending', run_after = now() + make_interval(secs => $3),
                    last_error = $4, lease_owner = NULL, lease_until = NULL, updated_at = now()
              WHERE id = $1 AND lease_owner = $2",
        )
        .bind(task_id)
        .bind(worker_id)
        .bind(delay_secs)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Hands a task back on graceful shutdown without consuming an attempt.
    pub async fn release_task(&self, task_id: Uuid, worker_id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE job_tasks SET state = 'pending', attempts = greatest(attempts - 1, 0), run_after = now(),
                    lease_owner = NULL, lease_until = NULL, updated_at = now()
              WHERE id = $1 AND lease_owner = $2 AND state = 'running'",
        )
        .bind(task_id)
        .bind(worker_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Recovers tasks whose worker stopped heart-beating. Requeues them while attempts remain; otherwise
    /// fails them. The caller updates the corresponding jobs.
    pub async fn reap_expired_tasks(&self) -> Result<Vec<ReapedTask>> {
        Ok(sqlx::query_as::<_, ReapedTask>(
            r#"
            UPDATE job_tasks
               SET state = CASE WHEN attempts < max_attempts THEN 'pending' ELSE 'failed' END,
                   last_error = 'lease expired (worker stopped responding)',
                   lease_owner = NULL, lease_until = NULL, run_after = now(), updated_at = now()
             WHERE id IN (SELECT id FROM job_tasks
                           WHERE state = 'running' AND lease_until < now()
                           FOR UPDATE SKIP LOCKED)
            RETURNING id, job_id, kind, state = 'pending' AS requeued
            "#,
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn pending_task_count(&self) -> Result<i64> {
        Ok(sqlx::query_scalar("SELECT count(*) FROM job_tasks WHERE state = 'pending'").fetch_one(&self.pool).await?)
    }

    pub async fn get_task(&self, task_id: Uuid) -> Result<Option<TaskRow>> {
        let sql = format!("SELECT {TASK_COLUMNS} FROM job_tasks WHERE id = $1");
        Ok(sqlx::query_as::<_, TaskRow>(&sql).bind(task_id).fetch_optional(&self.pool).await?)
    }

    pub async fn tasks_for_job(&self, job_id: Uuid) -> Result<Vec<TaskRow>> {
        let sql = format!("SELECT {TASK_COLUMNS} FROM job_tasks WHERE job_id = $1 ORDER BY created_at");
        Ok(sqlx::query_as::<_, TaskRow>(&sql).bind(job_id).fetch_all(&self.pool).await?)
    }
}
