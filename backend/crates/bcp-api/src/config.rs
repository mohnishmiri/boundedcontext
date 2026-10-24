use std::net::SocketAddr;
use std::time::Duration;

use anyhow::Result;
use bcp_core::config::{env_bool, env_or, env_parse, CommonConfig};

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
}

impl ApiConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            common: CommonConfig::from_env()?,
            listen: env_or("BCP_API_LISTEN", "0.0.0.0:8080").parse()?,
            metrics_listen: env_or("BCP_API_METRICS_LISTEN", "0.0.0.0:9090").parse()?,
            session_ttl: Duration::from_secs(env_parse::<u64>("BCP_SESSION_TTL_HOURS", 12)? * 3600),
            cookie_secure: env_bool("BCP_COOKIE_SECURE", true)?,
            rate_limit_per_minute: env_parse("BCP_RATE_LIMIT_PER_MINUTE", 120)?,
            job_create_limit_per_hour: env_parse("BCP_JOB_CREATE_LIMIT_PER_HOUR", 20)?,
            run_migrations: env_bool("BCP_RUN_MIGRATIONS", false)?,
            job_retention_days: env_parse("BCP_JOB_RETENTION_DAYS", 90)?,
        })
    }

    /// OAuth callback registered on the GitHub App.
    pub fn redirect_uri(&self) -> String {
        format!("{}/api/auth/github/callback", self.common.public_url.as_str().trim_end_matches('/'))
    }
}
