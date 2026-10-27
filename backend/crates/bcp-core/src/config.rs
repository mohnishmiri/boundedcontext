//! Environment-based configuration shared by the API and the worker. Secrets can be supplied either directly
//! (`NAME`) or as a path (`NAME_FILE`) — the latter is how Azure Key Vault CSI mounts reach the pods.

use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{bail, Context, Result};
use url::Url;

use crate::crypto::TokenCipher;

pub fn env_opt(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

pub fn env_req(name: &str) -> Result<String> {
    env_opt(name).with_context(|| format!("environment variable {name} is required"))
}

pub fn env_or(name: &str, default: &str) -> String {
    env_opt(name).unwrap_or_else(|| default.to_owned())
}

pub fn env_parse<T: FromStr>(name: &str, default: T) -> Result<T>
where
    T::Err: std::fmt::Display,
{
    match env_opt(name) {
        None => Ok(default),
        Some(v) => v.parse().map_err(|e| anyhow::anyhow!("invalid value for {name}: {e}")),
    }
}

pub fn env_bool(name: &str, default: bool) -> Result<bool> {
    match env_opt(name).as_deref().map(str::to_ascii_lowercase).as_deref() {
        None => Ok(default),
        Some("1" | "true" | "yes" | "on") => Ok(true),
        Some("0" | "false" | "no" | "off") => Ok(false),
        Some(other) => bail!("invalid boolean for {name}: {other}"),
    }
}

/// Reads `NAME`, or the contents of the file named by `NAME_FILE`.
pub fn env_secret(name: &str) -> Result<Option<String>> {
    if let Some(v) = env_opt(name) {
        return Ok(Some(v));
    }
    let file_var = format!("{name}_FILE");
    match env_opt(&file_var) {
        None => Ok(None),
        Some(path) => {
            let v = std::fs::read_to_string(&path).with_context(|| format!("reading {file_var}={path}"))?;
            let v = v.trim().to_owned();
            if v.is_empty() {
                bail!("{file_var} points at an empty file");
            }
            Ok(Some(v))
        }
    }
}

pub fn env_secret_req(name: &str) -> Result<String> {
    env_secret(name)?.with_context(|| format!("{name} or {name}_FILE is required"))
}

fn parse_url(name: &str, value: &str) -> Result<Url> {
    let url = Url::parse(value).with_context(|| format!("{name} is not a valid URL"))?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("{name} must be http(s)");
    }
    Ok(url)
}

#[derive(Debug, Clone)]
pub enum LogFormat {
    Json,
    Pretty,
}

/// GitHub endpoints and App credentials.
#[derive(Clone)]
pub struct GitHubSettings {
    pub api_url: Url,
    pub web_url: Url,
    pub oauth_url: Url,
    pub app_id: String,
    pub app_slug: String,
    pub client_id: String,
    pub client_secret: String,
    pub private_key_pem: String,
}

impl std::fmt::Debug for GitHubSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubSettings")
            .field("api_url", &self.api_url.as_str())
            .field("web_url", &self.web_url.as_str())
            .field("oauth_url", &self.oauth_url.as_str())
            .field("app_id", &self.app_id)
            .field("app_slug", &self.app_slug)
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

impl GitHubSettings {
    pub fn pat_from_env() -> Option<String> {
        env_secret("BCP_GITHUB_PAT").ok().flatten().or_else(|| env_opt("GITHUB_TOKEN"))
    }

    pub fn from_env() -> Result<Self> {
        let client_id = env_or("BCP_GITHUB_CLIENT_ID", "");
        let client_secret = env_secret("BCP_GITHUB_CLIENT_SECRET")?.unwrap_or_default();
        let use_mock = env_bool("BCP_USE_MOCK_GITHUB", false)?;

        let raw_api_url = env_or("BCP_GITHUB_API_URL", "https://api.github.com");
        let api_url = if !use_mock && raw_api_url == "http://mock-github:9000" {
            parse_url("BCP_GITHUB_API_URL", "https://api.github.com")?
        } else {
            parse_url("BCP_GITHUB_API_URL", &raw_api_url)?
        };

        let raw_web_url = env_or("BCP_GITHUB_WEB_URL", "https://github.com");
        let web_url = if !use_mock && raw_web_url.starts_with("http://localhost:") {
            parse_url("BCP_GITHUB_WEB_URL", "https://github.com")?
        } else {
            parse_url("BCP_GITHUB_WEB_URL", &raw_web_url)?
        };

        let oauth_url = match env_opt("BCP_GITHUB_OAUTH_URL") {
            Some(v) if !(!use_mock && v == "http://mock-github:9000") => parse_url("BCP_GITHUB_OAUTH_URL", &v)?,
            _ => web_url.clone(),
        };

        let raw_app_id = env_or("BCP_GITHUB_APP_ID", "");
        let app_id = if !use_mock && raw_app_id == "4242" { String::new() } else { raw_app_id };

        let private_key_pem =
            if app_id.is_empty() { String::new() } else { env_secret("BCP_GITHUB_PRIVATE_KEY")?.unwrap_or_default() };

        Ok(Self {
            api_url,
            web_url,
            oauth_url,
            app_id,
            app_slug: env_or("BCP_GITHUB_APP_SLUG", "bounded-context-portal"),
            client_id,
            client_secret,
            private_key_pem,
        })
    }
}

/// Settings common to both binaries.
#[derive(Debug, Clone)]
pub struct CommonConfig {
    pub database_url: String,
    pub database_max_connections: u32,
    pub public_url: Url,
    pub github: GitHubSettings,
    pub cipher: TokenCipher,
    pub agent_bundle_dir: PathBuf,
    pub models_file: PathBuf,
    pub models_show_unverified: bool,
    pub central_repository: Option<(String, String)>,
}

impl CommonConfig {
    pub fn from_env() -> Result<Self> {
        let key = env_secret_req("BCP_TOKEN_ENCRYPTION_KEY")?;
        let cipher = TokenCipher::from_base64(&key).context("BCP_TOKEN_ENCRYPTION_KEY")?;
        let central_repository = match env_opt("BCP_CENTRAL_REPO") {
            Some(value) => {
                let (owner, repo) = value.split_once('/').context("BCP_CENTRAL_REPO must be owner/repository")?;
                if owner.is_empty() || repo.is_empty() || repo.contains('/') {
                    bail!("BCP_CENTRAL_REPO must be owner/repository");
                }
                crate::validate::owner(owner).map_err(|e| anyhow::anyhow!("invalid BCP_CENTRAL_REPO owner: {e}"))?;
                crate::validate::repo(repo).map_err(|e| anyhow::anyhow!("invalid BCP_CENTRAL_REPO repository: {e}"))?;
                Some((owner.to_owned(), repo.to_owned()))
            }
            None => None,
        };
        Ok(Self {
            database_url: env_secret_req("BCP_DATABASE_URL")?,
            database_max_connections: env_parse("BCP_DATABASE_MAX_CONNECTIONS", 10)?,
            public_url: parse_url("BCP_PUBLIC_URL", &env_req("BCP_PUBLIC_URL")?)?,
            github: GitHubSettings::from_env()?,
            cipher,
            agent_bundle_dir: env_or("BCP_AGENT_BUNDLE_DIR", "/opt/agent-bundle").into(),
            models_file: env_or("BCP_MODELS_FILE", "/etc/bcp/models.yaml").into(),
            models_show_unverified: env_bool("BCP_MODELS_SHOW_UNVERIFIED", false)?,
            central_repository,
        })
    }

    /// `scheme://host[:port]` of the public URL, used for Origin checks.
    pub fn public_origin(&self) -> String {
        self.public_url.origin().ascii_serialization()
    }
}

pub fn log_format() -> LogFormat {
    match env_or("BCP_LOG_FORMAT", "json").as_str() {
        "pretty" | "text" => LogFormat::Pretty,
        _ => LogFormat::Json,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s");
        std::fs::write(&path, "  value-from-file\n").unwrap();
        // Unique names so parallel tests don't collide.
        std::env::set_var("BCP_TEST_SECRET_X_FILE", &path);
        assert_eq!(env_secret("BCP_TEST_SECRET_X").unwrap().as_deref(), Some("value-from-file"));
        std::env::set_var("BCP_TEST_SECRET_X", "direct");
        assert_eq!(env_secret("BCP_TEST_SECRET_X").unwrap().as_deref(), Some("direct"));
    }

    #[test]
    fn bools() {
        std::env::set_var("BCP_TEST_BOOL_Y", "TRUE");
        assert!(env_bool("BCP_TEST_BOOL_Y", false).unwrap());
        std::env::set_var("BCP_TEST_BOOL_Y", "maybe");
        assert!(env_bool("BCP_TEST_BOOL_Y", false).is_err());
        assert!(env_bool("BCP_TEST_BOOL_UNSET", true).unwrap());
    }
}
