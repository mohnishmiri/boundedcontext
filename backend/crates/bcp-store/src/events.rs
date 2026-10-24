use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{Result, Store};

/// One redacted log line. Serialises as the API's `LogEntry`.
#[derive(Debug, Clone, FromRow, Serialize, PartialEq)]
pub struct EventRow {
    pub seq: i64,
    pub ts: DateTime<Utc>,
    pub level: String,
    pub stage: String,
    pub message: String,
}

impl Store {
    /// Appends a log line. Callers must redact `message` first.
    pub async fn append_event(&self, job_id: Uuid, level: &str, stage: &str, message: &str) -> Result<i64> {
        Ok(sqlx::query_scalar(
            "INSERT INTO job_events (job_id, level, stage, message) VALUES ($1, $2, $3, $4) RETURNING seq",
        )
        .bind(job_id)
        .bind(level)
        .bind(stage)
        .bind(message)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn list_events(&self, job_id: Uuid, after: i64, limit: i64) -> Result<Vec<EventRow>> {
        Ok(sqlx::query_as::<_, EventRow>(
            "SELECT seq, ts, level, stage, message FROM job_events
              WHERE job_id = $1 AND seq > $2 ORDER BY seq LIMIT $3",
        )
        .bind(job_id)
        .bind(after)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// The most recent `limit` events, oldest first.
    pub async fn tail_events(&self, job_id: Uuid, limit: i64) -> Result<Vec<EventRow>> {
        let mut rows = sqlx::query_as::<_, EventRow>(
            "SELECT seq, ts, level, stage, message FROM job_events WHERE job_id = $1 ORDER BY seq DESC LIMIT $2",
        )
        .bind(job_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.reverse();
        Ok(rows)
    }

    pub async fn count_events(&self, job_id: Uuid) -> Result<i64> {
        Ok(sqlx::query_scalar("SELECT count(*) FROM job_events WHERE job_id = $1")
            .bind(job_id)
            .fetch_one(&self.pool)
            .await?)
    }
}
