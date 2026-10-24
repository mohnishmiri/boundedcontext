use bcp_core::{AppError, ErrorCode};

#[derive(Debug, Clone, thiserror::Error, PartialEq)]
pub enum GitHubError {
    #[error("GitHub rejected the credentials (401)")]
    Unauthorized,
    #[error("GitHub denied access (403): {message}")]
    Forbidden { message: String },
    #[error("GitHub rate limit exceeded")]
    RateLimited { retry_after: Option<u64> },
    #[error("not found (404)")]
    NotFound,
    #[error("GitHub rejected the request (422): {message}")]
    Unprocessable { message: String },
    #[error("conflict (409): {message}")]
    Conflict { message: String },
    #[error("GitHub server error ({status})")]
    Server { status: u16 },
    #[error("unexpected GitHub response ({status})")]
    Unexpected { status: u16 },
    #[error("network error talking to GitHub: {0}")]
    Network(String),
    #[error("could not decode GitHub response: {0}")]
    Decode(String),
    #[error("download exceeds the size limit")]
    TooLarge,
    #[error("OAuth error: {error}")]
    OAuth { error: String, description: Option<String> },
    #[error("local I/O error: {0}")]
    Io(String),
}

impl GitHubError {
    /// Worth retrying the same request later.
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::RateLimited { .. } | Self::Server { .. } | Self::Network(_))
    }

    /// Mapping for calls made with the signed-in user's token.
    pub fn to_user_error(&self) -> AppError {
        match self {
            Self::Unauthorized => AppError::new(
                ErrorCode::GithubAuthExpired,
                "Your GitHub authorisation has expired or was revoked. Sign in again.",
            ),
            Self::OAuth { error, .. } if error == "bad_refresh_token" || error == "bad_verification_code" => {
                AppError::new(ErrorCode::GithubAuthExpired, "Your GitHub authorisation has expired. Sign in again.")
            }
            Self::Forbidden { .. } => AppError::new(
                ErrorCode::RepositoryAccessDenied,
                "GitHub denied access to this repository for your account.",
            ),
            Self::NotFound => AppError::new(
                ErrorCode::RepositoryAccessDenied,
                "The repository or branch does not exist, or your account cannot access it.",
            ),
            other => other.to_service_error(),
        }
    }

    /// Mapping for calls made with the App JWT or an installation token.
    pub fn to_service_error(&self) -> AppError {
        match self {
            Self::RateLimited { retry_after } => {
                let err = AppError::new(ErrorCode::GithubRateLimited, "GitHub rate limit reached. Try again shortly.");
                match retry_after {
                    Some(s) => err.with_retry_after(*s),
                    None => err,
                }
            }
            Self::Server { .. } | Self::Network(_) | Self::Unexpected { .. } | Self::Decode(_) => {
                AppError::new(ErrorCode::GithubUnavailable, "GitHub is not responding as expected. Try again later.")
            }
            Self::NotFound => AppError::not_found("GitHub resource"),
            Self::TooLarge => AppError::new(
                ErrorCode::RepositoryTooLarge,
                "The repository snapshot is larger than this portal allows.",
            ),
            Self::Unauthorized | Self::Forbidden { .. } => AppError::new(
                ErrorCode::Internal,
                "The portal's GitHub App credentials were rejected. Contact the portal administrators.",
            )
            .retryable(false),
            Self::Unprocessable { .. } | Self::Conflict { .. } | Self::OAuth { .. } | Self::Io(_) => {
                AppError::internal("Unexpected error while talking to GitHub.")
            }
        }
    }
}
