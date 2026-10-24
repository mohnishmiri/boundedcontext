use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use bcp_core::config::{env_bool, env_opt, env_or, env_parse, CommonConfig};
use url::Url;

#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub common: CommonConfig,
    pub listen: SocketAddr,
    pub metrics_listen: SocketAddr,
    pub session_ttl: Duration,
    pub cookie_secure: bool,
    pub rate_limit_per_minute: u32,
    pub job_create_limit_per_hour: u32,
    pub run_migrations: bool,
    /// Settled jobs older than this are deleted (0 = keep forever).
    pub job_retention_days: u32,
    /// Operator-maintained generator registry (curated generators and the user-addition allowlist).
    pub generators_file: PathBuf,
    /// Scratch space for generator repositories fetched to validate them.
    pub work_dir: PathBuf,
    /// Copilot API used to list the models the signed-in user can use; `None` = catalog only.
    pub copilot_api_url: Option<Url>,
    pub copilot_integration_id: String,
    /// Most repositories one batch request may cover.
    pub batch_max_repos: usize,
}

impl ApiConfig {
    pub fn from_env() -> Result<Self> {
        let common = CommonConfig::from_env()?;
        // Unset: ask the real Copilot API only when the portal talks to real github.com; a mock or GitHub
        // Enterprise Server endpoint has no matching Copilot API, so the catalog is used.
        let copilot_api_url = match env_opt("BCP_COPILOT_API_URL").as_deref() {
            Some("off" | "none" | "disabled") => None,
            Some(v) => Some(Url::parse(v).context("BCP_COPILOT_API_URL is not a valid URL")?),
            None if common.github.api_url.host_str() == Some("api.github.com") => {
                Some(Url::parse("https://api.githubcopilot.com")?)
            }
            None => None,
        };
        Ok(Self {
            common,
            listen: env_or("BCP_API_LISTEN", "0.0.0.0:8080").parse()?,
            metrics_listen: env_or("BCP_API_METRICS_LISTEN", "0.0.0.0:9090").parse()?,
            session_ttl: Duration::from_secs(env_parse::<u64>("BCP_SESSION_TTL_HOURS", 12)? * 3600),
            cookie_secure: env_bool("BCP_COOKIE_SECURE", true)?,
            rate_limit_per_minute: env_parse("BCP_RATE_LIMIT_PER_MINUTE", 120)?,
            job_create_limit_per_hour: env_parse("BCP_JOB_CREATE_LIMIT_PER_HOUR", 100)?,
            run_migrations: env_bool("BCP_RUN_MIGRATIONS", false)?,
            job_retention_days: env_parse("BCP_JOB_RETENTION_DAYS", 90)?,
            generators_file: env_or("BCP_GENERATORS_FILE", "/etc/bcp/generators.yaml").into(),
            work_dir: env_or("BCP_WORK_DIR", "/tmp/bcp-work").into(),
            copilot_api_url,
            copilot_integration_id: env_or("BCP_COPILOT_INTEGRATION_ID", "copilot-developer-cli"),
            batch_max_repos: env_parse("BCP_BATCH_MAX_REPOS", 25)?,
        })
    }

    /// OAuth callback registered on the GitHub App.
    pub fn redirect_uri(&self) -> String {
        format!("{}/api/auth/github/callback", self.common.public_url.as_str().trim_end_matches('/'))
    }
}
