use bcp_core::domain::{GenerationStage, JobStatus, PublishStage, PublishStatus, TaskKind};
use bcp_core::AppError;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{Result, Store, StoreError};

/// Columns selected for every job read, joined with the requester's login.
const JOB_COLUMNS: &str = r#"
    j.id, j.user_id, u.login AS requested_by, j.owner, j.repo, j.repo_id, j.installation_id, j.base_branch,
    j.model, j.inputs, j.can_push, j.retry_of, j.status, j.stage, j.stage_times, j.base_sha, j.agent_id,
    j.agent_version, j.agent_placeholder, j.cli_version, j.error_code, j.error_message, j.error_retryable,
    j.cancel_requested, j.output_primary, j.publish_status, j.publish_stage, j.publish_stage_times,
    j.publish_requested_by, j.publish_branch, j.publish_commit_sha, j.publish_pr_number, j.publish_pr_url,
    j.publish_error_code, j.publish_error_message, j.publish_error_retryable, j.publish_updated_at,
    j.created_at, j.started_at, j.finished_at, j.updated_at
"#;

#[derive(Debug, Clone, FromRow)]
pub struct JobRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub requested_by: String,
    pub owner: String,
    pub repo: String,
    pub repo_id: Option<i64>,
    pub installation_id: Option<i64>,
    pub base_branch: String,
    pub model: String,
    pub inputs: Value,
    pub can_push: bool,
    pub retry_of: Option<Uuid>,
    pub status: String,
    pub stage: String,
    pub stage_times: Value,
    pub base_sha: Option<String>,
    pub agent_id: String,
    pub agent_version: String,
    pub agent_placeholder: bool,
    pub cli_version: Option<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub error_retryable: Option<bool>,
    pub cancel_requested: bool,
    pub output_primary: Option<String>,
    pub publish_status: String,
    pub publish_stage: Option<String>,
    pub publish_stage_times: Value,
    pub publish_requested_by: Option<Uuid>,
    pub publish_branch: Option<String>,
    pub publish_commit_sha: Option<String>,
    pub publish_pr_number: Option<i32>,
    pub publish_pr_url: Option<String>,
    pub publish_error_code: Option<String>,
    pub publish_error_message: Option<String>,
    pub publish_error_retryable: Option<bool>,
    pub publish_updated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl JobRow {
    pub fn status(&self) -> JobStatus {
        JobStatus::parse(&self.status).unwrap_or(JobStatus::Failed)
    }
    pub fn stage(&self) -> GenerationStage {
        GenerationStage::parse(&self.stage).unwrap_or(GenerationStage::Queued)
    }
    pub fn publish_status(&self) -> PublishStatus {
        PublishStatus::parse(&self.publish_status).unwrap_or(PublishStatus::NotStarted)
    }
    pub fn publish_stage(&self) -> Option<PublishStage> {
        self.publish_stage.as_deref().and_then(PublishStage::parse)
    }
    pub fn short_id(&self) -> String {
        self.id.simple().to_string()[..8].to_owned()
    }
    /// The generation and its publish sub-state are both finished: nothing will change without user action.
    pub fn is_settled(&self) -> bool {
        self.status().is_terminal() && !self.publish_status().is_active()
    }
}

#[derive(Debug, Clone)]
pub struct NewJob {
    pub user_id: Uuid,
    pub owner: String,
    pub repo: String,
    pub repo_id: Option<i64>,
    pub installation_id: Option<i64>,
    pub base_branch: String,
    pub model: String,
    pub inputs: Value,
    pub can_push: bool,
    pub retry_of: Option<Uuid>,
    pub agent_id: String,
    pub agent_version: String,
    pub agent_placeholder: bool,
    pub max_attempts: i32,
}

#[derive(Debug, Clone, Default)]
pub struct JobFilter {
    pub status: Option<JobStatus>,
    pub owner: Option<String>,
    pub repo: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct ArtifactRow {
    pub path: String,
    pub change_type: String,
    pub allowed: bool,
    pub base_content: Option<String>,
    pub new_content: Option<String>,
    pub unified_diff: Option<String>,
    pub size_bytes: i64,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewArtifact {
    pub path: String,
    pub change_type: String,
    pub allowed: bool,
    pub base_content: Option<String>,
    pub new_content: Option<String>,
    pub unified_diff: Option<String>,
    pub size_bytes: i64,
    pub sha256: Option<String>,
}

#[derive(Debug)]
pub enum CancelOutcome {
    /// Was still queued; now cancelled.
    Cancelled(JobRow),
    /// Running; the worker will stop it.
    Requested(JobRow),
}

#[derive(Debug)]
pub enum PublishOutcome {
    Queued(JobRow),
    /// Publishing is already queued, running or done: nothing changed.
    Unchanged(JobRow),
}

/// Fields the publish pipeline persists step by step so a retry resumes instead of repeating.
#[derive(Debug, Default, Clone)]
pub struct PublishProgress {
    pub branch: Option<String>,
    pub commit_sha: Option<String>,
    pub pr_number: Option<i32>,
    pub pr_url: Option<String>,
}

impl Store {
    pub async fn get_job(&self, id: Uuid) -> Result<Option<JobRow>> {
        let sql = format!("SELECT {JOB_COLUMNS} FROM jobs j JOIN users u ON u.id = j.user_id WHERE j.id = $1");
        Ok(sqlx::query_as::<_, JobRow>(&sql).bind(id).fetch_optional(&self.pool).await?)
    }

    /// A job visible to `user_id` (jobs are private to their requester).
    pub async fn get_user_job(&self, user_id: Uuid, id: Uuid) -> Result<JobRow> {
        match self.get_job(id).await? {
            Some(job) if job.user_id == user_id => Ok(job),
            _ => Err(StoreError::NotFound),
        }
    }

    pub async fn active_job_for(
        &self,
        user_id: Uuid,
        owner: &str,
        repo: &str,
        branch: &str,
        model: &str,
    ) -> Result<Option<Uuid>> {
        Ok(sqlx::query_scalar(
            "SELECT id FROM jobs WHERE user_id = $1 AND lower(owner) = lower($2) AND lower(repo) = lower($3)
               AND base_branch = $4 AND model = $5 AND status IN ('queued', 'running') LIMIT 1",
        )
        .bind(user_id)
        .bind(owner)
        .bind(repo)
        .bind(branch)
        .bind(model)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// Inserts a job and its generate task atomically. A concurrent duplicate fails with `DuplicateJob`.
    pub async fn create_job(&self, new: NewJob) -> Result<JobRow> {
        let id = Uuid::new_v4();
        let mut tx = self.pool.begin().await?;
        let inserted = sqlx::query(
            r#"
            INSERT INTO jobs (id, user_id, owner, repo, repo_id, installation_id, base_branch, model, inputs,
                              can_push, retry_of, agent_id, agent_version, agent_placeholder)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            "#,
        )
        .bind(id)
        .bind(new.user_id)
        .bind(&new.owner)
        .bind(&new.repo)
        .bind(new.repo_id)
        .bind(new.installation_id)
        .bind(&new.base_branch)
        .bind(&new.model)
        .bind(&new.inputs)
        .bind(new.can_push)
        .bind(new.retry_of)
        .bind(&new.agent_id)
        .bind(&new.agent_version)
        .bind(new.agent_placeholder)
        .execute(&mut *tx)
        .await;
        if let Err(e) = inserted {
            let err = StoreError::from(e);
            if err.unique_violation() == Some("jobs_active_unique") {
                drop(tx);
                let existing = self
                    .active_job_for(new.user_id, &new.owner, &new.repo, &new.base_branch, &new.model)
                    .await?
                    .ok_or(StoreError::Conflict("duplicate job vanished"))?;
                return Err(StoreError::DuplicateJob(existing));
            }
            return Err(err);
        }
        sqlx::query("INSERT INTO job_tasks (id, job_id, kind, max_attempts) VALUES ($1, $2, $3, $4)")
            .bind(Uuid::new_v4())
            .bind(id)
            .bind(TaskKind::Generate.as_str())
            .bind(new.max_attempts.max(1))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.get_job(id).await?.ok_or(StoreError::NotFound)
    }

    pub async fn list_jobs(
        &self,
        user_id: Uuid,
        filter: &JobFilter,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<JobRow>, i64)> {
        let offset = i64::from(page.saturating_sub(1)) * i64::from(per_page);
        let where_sql = "j.user_id = $1 AND ($2::text IS NULL OR j.status = $2)
                         AND ($3::text IS NULL OR lower(j.owner) = lower($3))
                         AND ($4::text IS NULL OR lower(j.repo) = lower($4))";
        let sql = format!(
            "SELECT {JOB_COLUMNS} FROM jobs j JOIN users u ON u.id = j.user_id WHERE {where_sql}
             ORDER BY j.created_at DESC, j.id LIMIT $5 OFFSET $6"
        );
        let status = filter.status.map(|s| s.as_str());
        let rows = sqlx::query_as::<_, JobRow>(&sql)
            .bind(user_id)
            .bind(status)
            .bind(filter.owner.as_deref())
            .bind(filter.repo.as_deref())
            .bind(i64::from(per_page))
            .bind(offset)
            .fetch_all(&self.pool)
            .await?;
        let total: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM jobs j WHERE {where_sql}"))
            .bind(user_id)
            .bind(status)
            .bind(filter.owner.as_deref())
            .bind(filter.repo.as_deref())
            .fetch_one(&self.pool)
            .await?;
        Ok((rows, total))
    }

    /// Deletes settled jobs (with their artifacts, logs and tasks) created before `cutoff`. Audit events are
    /// kept. Returns the number of jobs removed.
    pub async fn purge_jobs_before(&self, cutoff: DateTime<Utc>) -> Result<u64> {
        let r = sqlx::query(
            "DELETE FROM jobs WHERE created_at < $1 AND status IN ('succeeded', 'failed', 'cancelled')
               AND publish_status NOT IN ('queued', 'running')
               AND NOT EXISTS (SELECT 1 FROM jobs newer WHERE newer.retry_of = jobs.id)",
        )
        .bind(cutoff)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected())
    }

    pub async fn count_jobs_since(&self, user_id: Uuid, since: DateTime<Utc>) -> Result<i64> {
        Ok(sqlx::query_scalar("SELECT count(*) FROM jobs WHERE user_id = $1 AND created_at >= $2")
            .bind(user_id)
            .bind(since)
            .fetch_one(&self.pool)
            .await?)
    }

    /// Cancels a queued job immediately, or flags a running job for the worker.
    pub async fn request_cancel(&self, user_id: Uuid, job_id: Uuid) -> Result<CancelOutcome> {
        let mut tx = self.pool.begin().await?;
        let row: Option<(Uuid, String)> = sqlx::query_as("SELECT user_id, status FROM jobs WHERE id = $1 FOR UPDATE")
            .bind(job_id)
            .fetch_optional(&mut *tx)
            .await?;
        let Some((owner_id, status)) = row else { return Err(StoreError::NotFound) };
        if owner_id != user_id {
            return Err(StoreError::NotFound);
        }
        let queued = match JobStatus::parse(&status) {
            Some(JobStatus::Queued) => true,
            Some(JobStatus::Running) => false,
            _ => return Err(StoreError::Conflict("The job has already finished.")),
        };
        if queued {
            sqlx::query(
                "UPDATE jobs SET status = 'cancelled', cancel_requested = true, finished_at = now(), updated_at = now(),
                        error_code = 'cancelled', error_message = 'Cancelled before it started.', error_retryable = false
                  WHERE id = $1",
            )
            .bind(job_id)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE job_tasks SET state = 'cancelled', updated_at = now()
                  WHERE job_id = $1 AND kind = 'generate' AND state IN ('pending', 'running')",
            )
            .bind(job_id)
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query("UPDATE jobs SET cancel_requested = true, updated_at = now() WHERE id = $1")
                .bind(job_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        let job = self.get_job(job_id).await?.ok_or(StoreError::NotFound)?;
        Ok(if queued { CancelOutcome::Cancelled(job) } else { CancelOutcome::Requested(job) })
    }

    pub async fn is_cancel_requested(&self, job_id: Uuid) -> Result<bool> {
        Ok(sqlx::query_scalar("SELECT cancel_requested FROM jobs WHERE id = $1")
            .bind(job_id)
            .fetch_optional(&self.pool)
            .await?
            .unwrap_or(true))
    }

    /// Moves a queued job to running. Returns `None` when it is no longer runnable (e.g. cancelled).
    pub async fn start_job(&self, job_id: Uuid, stage_times: &Value) -> Result<Option<JobRow>> {
        let updated = sqlx::query(
            "UPDATE jobs SET status = 'running', stage = 'preparing', stage_times = $2,
                    started_at = coalesce(started_at, now()), updated_at = now(),
                    error_code = NULL, error_message = NULL, error_retryable = NULL
              WHERE id = $1 AND status IN ('queued', 'running') AND NOT cancel_requested",
        )
        .bind(job_id)
        .bind(stage_times)
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() == 0 {
            return Ok(None);
        }
        self.get_job(job_id).await
    }

    pub async fn set_job_stage(&self, job_id: Uuid, stage: GenerationStage, stage_times: &Value) -> Result<()> {
        sqlx::query(
            "UPDATE jobs SET stage = $2, stage_times = $3, updated_at = now() WHERE id = $1 AND status = 'running'",
        )
        .bind(job_id)
        .bind(stage.as_str())
        .bind(stage_times)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_job_source(
        &self,
        job_id: Uuid,
        base_sha: &str,
        repo_id: i64,
        installation_id: i64,
        can_push: bool,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE jobs SET base_sha = $2, repo_id = $3, installation_id = $4, can_push = $5, updated_at = now()
              WHERE id = $1",
        )
        .bind(job_id)
        .bind(base_sha)
        .bind(repo_id)
        .bind(installation_id)
        .bind(can_push)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_job_cli_version(&self, job_id: Uuid, version: &str) -> Result<()> {
        sqlx::query("UPDATE jobs SET cli_version = $2, updated_at = now() WHERE id = $1")
            .bind(job_id)
            .bind(version)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Stores the generated artifacts and marks the job succeeded, atomically.
    pub async fn complete_job(
        &self,
        job_id: Uuid,
        output_primary: &str,
        artifacts: &[NewArtifact],
        stage_times: &Value,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM job_artifacts WHERE job_id = $1").bind(job_id).execute(&mut *tx).await?;
        for a in artifacts {
            sqlx::query(
                "INSERT INTO job_artifacts (job_id, path, change_type, allowed, base_content, new_content,
                                            unified_diff, size_bytes, sha256)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(job_id)
            .bind(&a.path)
            .bind(&a.change_type)
            .bind(a.allowed)
            .bind(&a.base_content)
            .bind(&a.new_content)
            .bind(&a.unified_diff)
            .bind(a.size_bytes)
            .bind(&a.sha256)
            .execute(&mut *tx)
            .await?;
        }
        let updated = sqlx::query(
            "UPDATE jobs SET status = 'succeeded', stage = 'completed', stage_times = $3, output_primary = $2,
                    finished_at = now(), updated_at = now(), error_code = NULL, error_message = NULL,
                    error_retryable = NULL
              WHERE id = $1 AND status = 'running'",
        )
        .bind(job_id)
        .bind(output_primary)
        .bind(stage_times)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() == 0 {
            return Err(StoreError::Conflict("job is no longer running"));
        }
        tx.commit().await?;
        Ok(())
    }

    /// Terminal failure or cancellation of the generation.
    pub async fn finish_job_with_error(
        &self,
        job_id: Uuid,
        status: JobStatus,
        err: &AppError,
        stage_times: &Value,
    ) -> Result<()> {
        debug_assert!(matches!(status, JobStatus::Failed | JobStatus::Cancelled));
        sqlx::query(
            "UPDATE jobs SET status = $2, stage_times = $3, error_code = $4, error_message = $5,
                    error_retryable = $6, finished_at = now(), updated_at = now()
              WHERE id = $1 AND status IN ('queued', 'running')",
        )
        .bind(job_id)
        .bind(status.as_str())
        .bind(stage_times)
        .bind(err.code.as_str())
        .bind(&err.message)
        .bind(err.retryable)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Returns a running job to the queue (transient failure or worker shutdown).
    pub async fn requeue_job(&self, job_id: Uuid) -> Result<()> {
        sqlx::query(
            "UPDATE jobs SET status = 'queued', stage = 'queued', updated_at = now()
              WHERE id = $1 AND status = 'running'",
        )
        .bind(job_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_artifacts(&self, job_id: Uuid) -> Result<Vec<ArtifactRow>> {
        Ok(sqlx::query_as::<_, ArtifactRow>(
            "SELECT path, change_type, allowed, base_content, new_content, unified_diff, size_bytes, sha256
               FROM job_artifacts WHERE job_id = $1 ORDER BY allowed DESC, path",
        )
        .bind(job_id)
        .fetch_all(&self.pool)
        .await?)
    }

    // ----------------------------------------------------------------------------------------------------
    // Publishing
    // ----------------------------------------------------------------------------------------------------

    pub async fn request_publish(
        &self,
        user_id: Uuid,
        job_id: Uuid,
        max_attempts: i32,
        stage_times: &Value,
    ) -> Result<PublishOutcome> {
        let mut tx = self.pool.begin().await?;
        let row: Option<(Uuid, String, String)> =
            sqlx::query_as("SELECT user_id, status, publish_status FROM jobs WHERE id = $1 FOR UPDATE")
                .bind(job_id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some((owner_id, status, publish_status)) = row else { return Err(StoreError::NotFound) };
        if owner_id != user_id {
            return Err(StoreError::NotFound);
        }
        if JobStatus::parse(&status) != Some(JobStatus::Succeeded) {
            return Err(StoreError::Conflict("Only a successfully generated job can be published."));
        }
        let publish_status = PublishStatus::parse(&publish_status).unwrap_or(PublishStatus::NotStarted);
        if matches!(publish_status, PublishStatus::Queued | PublishStatus::Running | PublishStatus::Succeeded) {
            drop(tx);
            let job = self.get_job(job_id).await?.ok_or(StoreError::NotFound)?;
            return Ok(PublishOutcome::Unchanged(job));
        }
        sqlx::query(
            "UPDATE jobs SET publish_status = 'queued', publish_stage = 'queued', publish_stage_times = $3,
                    publish_requested_by = $2, publish_error_code = NULL, publish_error_message = NULL,
                    publish_error_retryable = NULL, publish_updated_at = now(), updated_at = now()
              WHERE id = $1",
        )
        .bind(job_id)
        .bind(user_id)
        .bind(stage_times)
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO job_tasks (id, job_id, kind, max_attempts) VALUES ($1, $2, 'publish', $3)")
            .bind(Uuid::new_v4())
            .bind(job_id)
            .bind(max_attempts.max(1))
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        let job = self.get_job(job_id).await?.ok_or(StoreError::NotFound)?;
        Ok(PublishOutcome::Queued(job))
    }

    pub async fn set_publish_stage(&self, job_id: Uuid, stage: PublishStage, stage_times: &Value) -> Result<()> {
        sqlx::query(
            "UPDATE jobs SET publish_status = 'running', publish_stage = $2, publish_stage_times = $3,
                    publish_updated_at = now(), updated_at = now()
              WHERE id = $1 AND publish_status IN ('queued', 'running')",
        )
        .bind(job_id)
        .bind(stage.as_str())
        .bind(stage_times)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Persists whichever publish results are set; `None` fields are left unchanged.
    pub async fn save_publish_progress(&self, job_id: Uuid, p: &PublishProgress) -> Result<()> {
        sqlx::query(
            "UPDATE jobs SET publish_branch = coalesce($2, publish_branch),
                    publish_commit_sha = coalesce($3, publish_commit_sha),
                    publish_pr_number = coalesce($4, publish_pr_number),
                    publish_pr_url = coalesce($5, publish_pr_url),
                    publish_updated_at = now(), updated_at = now()
              WHERE id = $1",
        )
        .bind(job_id)
        .bind(&p.branch)
        .bind(&p.commit_sha)
        .bind(p.pr_number)
        .bind(&p.pr_url)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn complete_publish(&self, job_id: Uuid, stage_times: &Value) -> Result<()> {
        sqlx::query(
            "UPDATE jobs SET publish_status = 'succeeded', publish_stage = 'completed', publish_stage_times = $2,
                    publish_error_code = NULL, publish_error_message = NULL, publish_error_retryable = NULL,
                    publish_updated_at = now(), updated_at = now()
              WHERE id = $1",
        )
        .bind(job_id)
        .bind(stage_times)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn fail_publish(&self, job_id: Uuid, err: &AppError, stage_times: &Value) -> Result<()> {
        sqlx::query(
            "UPDATE jobs SET publish_status = 'failed', publish_stage_times = $2, publish_error_code = $3,
                    publish_error_message = $4, publish_error_retryable = $5, publish_updated_at = now(),
                    updated_at = now()
              WHERE id = $1",
        )
        .bind(job_id)
        .bind(stage_times)
        .bind(err.code.as_str())
        .bind(&err.message)
        .bind(err.retryable)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Back to queued after a transient publish failure (the task is rescheduled separately).
    pub async fn requeue_publish(&self, job_id: Uuid) -> Result<()> {
        sqlx::query(
            "UPDATE jobs SET publish_status = 'queued', publish_updated_at = now(), updated_at = now()
              WHERE id = $1 AND publish_status = 'running'",
        )
        .bind(job_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
