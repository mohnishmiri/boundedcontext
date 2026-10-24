use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stable, machine-readable error codes shared by the API contract (docs/api.md), persisted job errors
/// and the worker. Adding a variant is backwards compatible; renaming one is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Unauthenticated,
    CsrfRejected,
    Forbidden,
    NotFound,
    ValidationFailed,
    DuplicateJob,
    Conflict,
    RateLimited,
    PayloadTooLarge,
    GithubUnavailable,
    GithubAuthExpired,
    GithubRateLimited,
    RepositoryAccessDenied,
    AppNotInstalled,
    ModelUnavailable,
    ModelLockedByAgent,
    GeneratorNotAllowed,
    GeneratorInvalid,
    CopilotUnauthorized,
    AgentFailed,
    AgentTimeout,
    AgentUnavailable,
    OutputMissing,
    OutputEmpty,
    OutputInvalid,
    OutputTooLarge,
    RepositoryTooLarge,
    BaseBranchChanged,
    BranchConflict,
    Cancelled,
    Internal,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unauthenticated => "unauthenticated",
            Self::CsrfRejected => "csrf_rejected",
            Self::Forbidden => "forbidden",
            Self::NotFound => "not_found",
            Self::ValidationFailed => "validation_failed",
            Self::DuplicateJob => "duplicate_job",
            Self::Conflict => "conflict",
            Self::RateLimited => "rate_limited",
            Self::PayloadTooLarge => "payload_too_large",
            Self::GithubUnavailable => "github_unavailable",
            Self::GithubAuthExpired => "github_auth_expired",
            Self::GithubRateLimited => "github_rate_limited",
            Self::RepositoryAccessDenied => "repository_access_denied",
            Self::AppNotInstalled => "app_not_installed",
            Self::ModelUnavailable => "model_unavailable",
            Self::ModelLockedByAgent => "model_locked_by_agent",
            Self::GeneratorNotAllowed => "generator_not_allowed",
            Self::GeneratorInvalid => "generator_invalid",
            Self::CopilotUnauthorized => "copilot_unauthorized",
            Self::AgentFailed => "agent_failed",
            Self::AgentTimeout => "agent_timeout",
            Self::AgentUnavailable => "agent_unavailable",
            Self::OutputMissing => "output_missing",
            Self::OutputEmpty => "output_empty",
            Self::OutputInvalid => "output_invalid",
            Self::OutputTooLarge => "output_too_large",
            Self::RepositoryTooLarge => "repository_too_large",
            Self::BaseBranchChanged => "base_branch_changed",
            Self::BranchConflict => "branch_conflict",
            Self::Cancelled => "cancelled",
            Self::Internal => "internal",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        serde_json::from_value(Value::String(s.to_owned())).ok()
    }

    pub fn http_status(self) -> u16 {
        match self {
            Self::ValidationFailed => 400,
            Self::Unauthenticated | Self::GithubAuthExpired => 401,
            Self::Forbidden
            | Self::CsrfRejected
            | Self::RepositoryAccessDenied
            | Self::GeneratorNotAllowed
            | Self::CopilotUnauthorized => 403,
            Self::NotFound | Self::AppNotInstalled => 404,
            Self::DuplicateJob
            | Self::Conflict
            | Self::ModelLockedByAgent
            | Self::BaseBranchChanged
            | Self::BranchConflict
            | Self::Cancelled => 409,
            Self::PayloadTooLarge | Self::RepositoryTooLarge | Self::OutputTooLarge => 413,
            Self::ModelUnavailable
            | Self::GeneratorInvalid
            | Self::OutputMissing
            | Self::OutputEmpty
            | Self::OutputInvalid => 422,
            Self::RateLimited | Self::GithubRateLimited => 429,
            Self::GithubUnavailable => 502,
            Self::AgentUnavailable | Self::AgentFailed | Self::AgentTimeout => 503,
            Self::Internal => 500,
        }
    }

    /// Whether repeating the same operation later may succeed without the user changing anything.
    pub fn default_retryable(self) -> bool {
        matches!(
            self,
            Self::RateLimited
                | Self::GithubUnavailable
                | Self::GithubRateLimited
                | Self::AgentUnavailable
                | Self::AgentTimeout
                | Self::AgentFailed
                | Self::Internal
        )
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An error that is safe to show to users. Internal details belong in logs, never in `message`.
#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize, PartialEq)]
#[error("{code}: {message}")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    /// Seconds a client should wait before retrying (rate limits).
    #[serde(skip)]
    pub retry_after: Option<u64>,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), retryable: code.default_retryable(), details: None, retry_after: None }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub fn with_retry_after(mut self, secs: u64) -> Self {
        self.retry_after = Some(secs);
        self
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }

    pub fn not_found(what: &str) -> Self {
        Self::new(ErrorCode::NotFound, format!("{what} not found"))
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Conflict, message)
    }

    /// A validation error with per-field messages, rendered as `details.fields`.
    pub fn validation<I, K, V>(fields: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let map: serde_json::Map<String, Value> =
            fields.into_iter().map(|(k, v)| (k.into(), Value::String(v.into()))).collect();
        let summary = if map.len() == 1 {
            let (k, v) = map.iter().next().expect("one entry");
            format!("{k}: {}", v.as_str().unwrap_or_default())
        } else {
            "Some fields are invalid".to_owned()
        };
        Self::new(ErrorCode::ValidationFailed, summary).with_details(serde_json::json!({ "fields": map }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_as_snake_case() {
        for code in [ErrorCode::DuplicateJob, ErrorCode::ModelLockedByAgent, ErrorCode::OutputEmpty] {
            let json = serde_json::to_value(code).unwrap();
            assert_eq!(json, Value::String(code.as_str().to_owned()));
            assert_eq!(ErrorCode::parse(code.as_str()), Some(code));
        }
        assert_eq!(ErrorCode::parse("nope"), None);
    }

    #[test]
    fn validation_error_carries_fields() {
        let err = AppError::validation([("repo", "Invalid repository name")]);
        assert_eq!(err.code, ErrorCode::ValidationFailed);
        assert_eq!(err.message, "repo: Invalid repository name");
        assert_eq!(err.details.unwrap()["fields"]["repo"], "Invalid repository name");
    }
}
