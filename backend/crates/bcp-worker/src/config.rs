use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Result};
use bcp_agent::Secret;
use bcp_core::config::{admin_logins_from_env, env_bool, env_opt, env_or, env_parse, env_secret, CommonConfig};

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
    /// Completion emails to the requester; `None` disables them.
    pub smtp: Option<SmtpConfig>,
}

/// Outgoing mail server for completion emails (`SMTP_*`).
#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<Secret>,
    pub from_address: String,
    pub from_name: String,
    /// STARTTLS on most ports, implicit TLS on 465; `false` sends in clear text (local test servers only).
    pub use_tls: bool,
    /// When GitHub exposes no email for the requester, mail `<login>@<domain>` (enterprise `_shortcode` removed).
    pub fallback_domain: Option<String>,
    /// Portal administrators (`BCP_ADMIN_LOGINS`), copied on every completion email.
    pub cc_admin_logins: Vec<String>,
}

impl SmtpConfig {
    /// Reads `SMTP_*`. Unset `SMTP_HOST` disables notifications; so does a user without a password (logged).
    pub fn from_env() -> Result<Option<Self>> {
        let Some(host) = env_opt("SMTP_HOST") else { return Ok(None) };
        let username = env_opt("SMTP_USER");
        let password = env_secret("SMTP_PASSWORD")?.map(Secret::new);
        if username.is_some() && password.is_none() {
            tracing::warn!("SMTP_USER is set but SMTP_PASSWORD(_FILE) is not; completion emails are disabled");
            return Ok(None);
        }
        let Some(from_address) = env_opt("SMTP_FROM_ADDRESS").or_else(|| username.clone()) else {
            bail!("SMTP_FROM_ADDRESS is required when SMTP_HOST is set");
        };
        Ok(Some(Self {
            host,
            port: env_parse("SMTP_PORT", 587)?,
            username,
            password,
            from_address,
            from_name: env_or("SMTP_FROM_NAME", "AT&T Bounded Context Portal"),
            use_tls: env_bool("SMTP_USE_TLS", true)?,
            fallback_domain: env_opt("BCP_NOTIFY_EMAIL_DOMAIN").map(|d| d.trim_start_matches('@').to_owned()),
            cc_admin_logins: admin_logins_from_env()?,
        }))
    }
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
            smtp: SmtpConfig::from_env()?,
        })
    }
}
