use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct GhUser {
    pub id: i64,
    pub login: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct GhAccount {
    pub login: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct GhInstallation {
    pub id: i64,
    pub account: GhAccount,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq)]
pub struct GhPermissions {
    #[serde(default)]
    pub pull: bool,
    #[serde(default)]
    pub push: bool,
    #[serde(default)]
    pub admin: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct GhOwner {
    pub login: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct GhRepo {
    pub id: i64,
    pub name: String,
    pub full_name: String,
    pub owner: GhOwner,
    #[serde(default)]
    pub private: bool,
    #[serde(default = "default_branch")]
    pub default_branch: String,
    pub html_url: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub permissions: Option<GhPermissions>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub archived: bool,
    /// Repository size in KiB as reported by GitHub (approximate).
    #[serde(default)]
    pub size: Option<u64>,
}

fn default_branch() -> String {
    "main".to_owned()
}

impl GhRepo {
    pub fn permissions(&self) -> GhPermissions {
        self.permissions.unwrap_or_default()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct GhSha {
    pub sha: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct GhBranch {
    pub name: String,
    pub commit: GhSha,
    #[serde(default)]
    pub protected: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct GhRefObject {
    pub sha: String,
    #[serde(rename = "type", default)]
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct GhRef {
    #[serde(rename = "ref")]
    pub name: String,
    pub object: GhRefObject,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct GhCommit {
    pub sha: String,
    pub tree: GhSha,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub parents: Vec<GhSha>,
    #[serde(default)]
    pub html_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct GhContent {
    pub sha: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct GhPullRef {
    #[serde(rename = "ref")]
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct GhPull {
    pub number: i64,
    pub html_url: String,
    #[serde(default)]
    pub state: String,
    pub head: GhPullRef,
    pub base: GhPullRef,
}

/// OAuth token response (user-to-server). Expiry fields are absent when token expiration is disabled for the App.
#[derive(Clone, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub refresh_token_expires_in: Option<i64>,
}

impl std::fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenResponse")
            .field("expires_in", &self.expires_in)
            .field("has_refresh_token", &self.refresh_token.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Deserialize)]
pub struct InstallationToken {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for InstallationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstallationToken").field("expires_at", &self.expires_at).finish_non_exhaustive()
    }
}

/// A file to place in a new tree.
#[derive(Debug, Clone)]
pub struct TreeFile {
    pub path: String,
    pub content: String,
}
