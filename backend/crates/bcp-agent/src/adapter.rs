use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use bcp_core::{AppError, ErrorCode};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::bundle::InputSpec;

/// A secret string that never appears in `Debug` output.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Static description of the installed agent (served by `GET /api/agent`).
#[derive(Debug, Clone, Serialize)]
pub struct AgentDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub placeholder: bool,
    pub output_paths: Vec<String>,
    pub inputs: Vec<InputSpec>,
    #[serde(skip)]
    pub locked_model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AgentRunRequest {
    /// Extracted repository snapshot; the agent's working directory.
    pub workspace: PathBuf,
    /// Private per-run directory (agent home, logs, transcript). Never committed.
    pub scratch_dir: PathBuf,
    /// `None` when the agent file pins its own model.
    pub model: Option<String>,
    pub inputs: BTreeMap<String, String>,
    pub owner: String,
    pub repo: String,
    pub branch: String,
    pub sha: String,
    /// The requesting user's GitHub token; passed to the agent runtime through the environment only.
    pub copilot_token: Secret,
    pub timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct AgentRunOutcome {
    pub duration: Duration,
    pub transcript_path: Option<PathBuf>,
    pub cli_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

/// Receives redacted progress lines from a run.
#[async_trait]
pub trait ProgressSink: Send + Sync {
    async fn log(&self, level: LogLevel, message: &str);
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum AgentError {
    #[error("the run was cancelled")]
    Cancelled,
    #[error("the agent did not finish within {0:?}")]
    Timeout(Duration),
    #[error("{0}")]
    Failed(AppError),
}

impl AgentError {
    pub fn to_app_error(&self) -> AppError {
        match self {
            Self::Cancelled => AppError::new(ErrorCode::Cancelled, "The generation was cancelled.").retryable(false),
            Self::Timeout(d) => AppError::new(
                ErrorCode::AgentTimeout,
                format!("The agent did not finish within {} minutes.", d.as_secs().div_ceil(60)),
            ),
            Self::Failed(e) => e.clone(),
        }
    }
}

#[async_trait]
pub trait AgentAdapter: Send + Sync {
    fn describe(&self) -> AgentDescriptor;

    /// Runs the agent in `req.workspace`. Must not touch anything outside the workspace and scratch dir,
    /// streams progress through `sink`, and stops promptly when `cancel` fires.
    async fn run(
        &self,
        req: AgentRunRequest,
        sink: &dyn ProgressSink,
        cancel: CancellationToken,
    ) -> Result<AgentRunOutcome, AgentError>;

    /// Checks the runtime is installed and returns its version.
    async fn preflight(&self) -> Result<String, AgentError>;
}
