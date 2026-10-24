use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;
use bcp_agent::{LogLevel, ProgressSink};
use bcp_core::redact::Redactor;
use bcp_store::Store;
use uuid::Uuid;

/// Writes redacted, capped log lines for one job into `job_events` (which notifies SSE subscribers).
pub struct JobLogger {
    store: Store,
    job_id: Uuid,
    stage: Mutex<String>,
    redactor: Redactor,
    written: AtomicI64,
    max_lines: i64,
}

impl JobLogger {
    pub fn new(store: Store, job_id: Uuid, already_written: i64, max_lines: i64, redactor: Redactor) -> Self {
        Self {
            store,
            job_id,
            stage: Mutex::new("queued".into()),
            redactor,
            written: AtomicI64::new(already_written),
            max_lines,
        }
    }

    pub fn set_stage(&self, stage: &str) {
        if let Ok(mut s) = self.stage.lock() {
            *s = stage.to_owned();
        }
    }

    pub async fn info(&self, message: &str) {
        self.log(LogLevel::Info, message).await;
    }

    pub async fn warn(&self, message: &str) {
        self.log(LogLevel::Warn, message).await;
    }

    pub async fn error(&self, message: &str) {
        self.log(LogLevel::Error, message).await;
    }
}

#[async_trait]
impl ProgressSink for JobLogger {
    async fn log(&self, level: LogLevel, message: &str) {
        let n = self.written.fetch_add(1, Ordering::Relaxed);
        if n > self.max_lines {
            return;
        }
        let stage = self.stage.lock().map(|s| s.clone()).unwrap_or_default();
        let (level, text) = if n == self.max_lines {
            (LogLevel::Warn, format!("Log limit of {} lines reached; further output is not stored.", self.max_lines))
        } else {
            (level, self.redactor.line(message))
        };
        tracing::debug!(job_id = %self.job_id, %stage, level = level.as_str(), "{text}");
        if let Err(e) = self.store.append_event(self.job_id, level.as_str(), &stage, &text).await {
            tracing::warn!(job_id = %self.job_id, error = %e, "failed to store job log line");
        }
    }
}
