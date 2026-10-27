use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Result};
use bcp_agent::Secret;
use bcp_core::config::{env_opt, env_or, env_parse, env_secret, CommonConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Long-running: claim tasks until shut down.
    Loop,
    /// Claim one task, run it, exit (KEDA ScaledJob).
    Once,
}

#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub common: CommonConfig,
    pub listen: SocketAddr,
    pub metrics_listen: SocketAddr,
    pub mode: Mode,
    pub concurrency: usize,
    pub worker_id: String,
    pub work_dir: PathBuf,
    pub copilot_bin: PathBuf,
    /// Shared Copilot CLI credential; when unset each job uses the requesting user's GitHub token.
    pub copilot_token: Option<Secret>,
    pub max_job_timeout: Duration,
    pub max_repo_bytes: u64,
    pub max_repo_files: u64,
    pub lease_secs: i64,
    pub generate_max_attempts: i32,
    pub publish_max_attempts: i32,
    pub shutdown_grace: Duration,
    pub idle_exit: Duration,
    pub max_log_lines: i64,
}

impl WorkerConfig {
    pub fn from_env() -> Result<Self> {
        let mode = match env_or("BCP_WORKER_MODE", "loop").as_str() {
            "loop" => Mode::Loop,
            "once" => Mode::Once,
            other => bail!("BCP_WORKER_MODE must be loop or once, got {other:?}"),
        };
        let lease_secs: i64 = env_parse("BCP_TASK_LEASE_SECS", 60)?;
        if lease_secs < 15 {
            bail!("BCP_TASK_LEASE_SECS must be at least 15");
        }
        Ok(Self {
            common: CommonConfig::from_env()?,
            listen: env_or("BCP_WORKER_LISTEN", "0.0.0.0:8081").parse()?,
            metrics_listen: env_or("BCP_WORKER_METRICS_LISTEN", "0.0.0.0:9091").parse()?,
            mode,
            concurrency: env_parse::<usize>("BCP_WORKER_CONCURRENCY", 1)?.max(1),
            worker_id: env_opt("BCP_WORKER_ID")
                .or_else(|| env_opt("HOSTNAME"))
                .unwrap_or_else(|| format!("worker-{}", uuid::Uuid::new_v4().simple())),
            work_dir: env_or("BCP_WORK_DIR", "/tmp/bcp-work").into(),
            copilot_bin: env_or("BCP_COPILOT_BIN", "copilot").into(),
            copilot_token: env_secret("BCP_COPILOT_GITHUB_TOKEN")?.map(Secret::new),
            max_job_timeout: Duration::from_secs(env_parse("BCP_MAX_JOB_TIMEOUT_SECS", 18000)?),
            max_repo_bytes: env_parse("BCP_MAX_REPO_BYTES", 500 * 1024 * 1024)?,
            max_repo_files: env_parse("BCP_MAX_REPO_FILES", 100_000)?,
            lease_secs,
            generate_max_attempts: env_parse::<i32>("BCP_GENERATE_MAX_ATTEMPTS", 2)?.max(1),
            publish_max_attempts: env_parse::<i32>("BCP_PUBLISH_MAX_ATTEMPTS", 5)?.max(1),
            shutdown_grace: Duration::from_secs(env_parse("BCP_SHUTDOWN_GRACE_SECS", 60)?),
            idle_exit: Duration::from_secs(env_parse("BCP_IDLE_EXIT_SECS", 30)?),
            max_log_lines: env_parse("BCP_MAX_LOG_LINES", 5000)?,
        })
    }
}
