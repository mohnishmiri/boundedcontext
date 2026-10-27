//! Services shared by the API and the worker.

pub mod generators;

use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

use bcp_core::crypto::TokenCipher;
use bcp_core::{AppError, ErrorCode};
use bcp_github::{GitHubClient, GitHubError, TokenResponse};
use bcp_store::{store_tokens_on, tokens_for_update, Store, TokenRow};

/// Refresh access tokens this long before they expire, so a token handed to a long agent run or a
/// multi-step publish does not lapse midway.
const REFRESH_MARGIN_MINUTES: i64 = 15;

/// Stores, decrypts and refreshes GitHub App user-to-server tokens.
#[derive(Clone, Debug)]
pub struct TokenService {
    store: Store,
    github: GitHubClient,
    cipher: TokenCipher,
}

fn access_aad(user_id: Uuid) -> Vec<u8> {
    format!("github-access:{user_id}").into_bytes()
}

fn refresh_aad(user_id: Uuid) -> Vec<u8> {
    format!("github-refresh:{user_id}").into_bytes()
}

fn copilot_aad(user_id: Uuid) -> Vec<u8> {
    format!("copilot-access:{user_id}").into_bytes()
}

fn expiry(now: DateTime<Utc>, secs: Option<i64>) -> Option<DateTime<Utc>> {
    secs.filter(|s| *s > 0).map(|s| now + Duration::seconds(s))
}

fn expired_auth() -> AppError {
    AppError::new(ErrorCode::GithubAuthExpired, "Your GitHub authorisation has expired. Sign in again.")
}

impl TokenService {
    pub fn new(store: Store, github: GitHubClient, cipher: TokenCipher) -> Self {
        Self { store, github, cipher }
    }

    pub fn github(&self) -> &GitHubClient {
        &self.github
    }

    pub async fn save_copilot_token(&self, user_id: Uuid, token: &str) -> Result<(), AppError> {
        let enc = self.cipher.encrypt_str(token, &copilot_aad(user_id));
        self.store.set_copilot_token(user_id, &enc).await?;
        Ok(())
    }

    pub async fn delete_copilot_token(&self, user_id: Uuid) -> Result<(), AppError> {
        self.store.delete_copilot_token(user_id).await?;
        Ok(())
    }

    /// The user's dedicated Copilot token, if they saved one.
    pub async fn copilot_token(&self, user_id: Uuid) -> Result<Option<String>, AppError> {
        let Some(enc) = self.store.get_copilot_token(user_id).await? else { return Ok(None) };
        match self.cipher.decrypt_string(&enc, &copilot_aad(user_id)) {
            Ok(t) => Ok(Some(t)),
            Err(_) => {
                tracing::error!(%user_id, "stored Copilot token could not be decrypted (key rotated?)");
                Ok(None)
            }
        }
    }

    /// The token Copilot runs with for this user: their dedicated Copilot token, else their sign-in token.
    pub async fn copilot_run_token(&self, user_id: Uuid) -> Result<String, AppError> {
        match self.copilot_token(user_id).await? {
            Some(t) => Ok(t),
            None => self.access_token(user_id).await,
        }
    }

    /// Persists tokens from an OAuth code exchange or refresh.
    pub async fn save(&self, user_id: Uuid, tokens: &TokenResponse) -> Result<(), AppError> {
        let now = Utc::now();
        let access = self.cipher.encrypt_str(&tokens.access_token, &access_aad(user_id));
        let refresh = tokens.refresh_token.as_deref().map(|r| self.cipher.encrypt_str(r, &refresh_aad(user_id)));
        self.store
            .store_tokens(
                user_id,
                &access,
                expiry(now, tokens.expires_in),
                refresh.as_deref(),
                expiry(now, tokens.refresh_token_expires_in),
            )
            .await?;
        Ok(())
    }

    fn decrypt_access(&self, row: &TokenRow) -> Result<String, AppError> {
        self.cipher.decrypt_string(&row.access_token, &access_aad(row.user_id)).map_err(|_| {
            tracing::error!(user_id = %row.user_id, "stored GitHub token could not be decrypted (key rotated?)");
            expired_auth()
        })
    }

    fn is_fresh(row: &TokenRow, now: DateTime<Utc>) -> bool {
        row.access_expires_at.is_none_or(|exp| exp > now + Duration::minutes(REFRESH_MARGIN_MINUTES))
    }

    /// A usable access token for the user, refreshed first when it is close to expiry.
    pub async fn access_token(&self, user_id: Uuid) -> Result<String, AppError> {
        let row = self.store.get_tokens(user_id).await?.ok_or_else(expired_auth)?;
        if Self::is_fresh(&row, Utc::now()) {
            return self.decrypt_access(&row);
        }
        self.refresh(user_id).await
    }

    /// Refreshes under a row lock: GitHub refresh tokens are single-use, so concurrent refreshes from several
    /// pods must not race.
    async fn refresh(&self, user_id: Uuid) -> Result<String, AppError> {
        let mut tx = self.store.pool().begin().await.map_err(bcp_store::StoreError::from)?;
        let row = tokens_for_update(&mut tx, user_id).await?.ok_or_else(expired_auth)?;
        let now = Utc::now();
        if Self::is_fresh(&row, now) {
            // Someone else refreshed while we waited for the lock.
            return self.decrypt_access(&row);
        }
        let refresh_usable = row.refresh_token.is_some() && row.refresh_expires_at.is_none_or(|exp| exp > now);
        if !refresh_usable {
            // The access token may still be valid for a few minutes; use it rather than failing early.
            if row.access_expires_at.is_some_and(|exp| exp > now + Duration::minutes(1)) {
                return self.decrypt_access(&row);
            }
            return Err(expired_auth());
        }
        let refresh_token = self
            .cipher
            .decrypt_string(row.refresh_token.as_deref().unwrap_or_default(), &refresh_aad(user_id))
            .map_err(|_| expired_auth())?;
        let fresh = match self.github.refresh_user_token(&refresh_token).await {
            Ok(t) => t,
            Err(e @ GitHubError::OAuth { .. }) | Err(e @ GitHubError::Unauthorized) => {
                tracing::warn!(%user_id, error = %e, "GitHub token refresh rejected; user must sign in again");
                drop(tx);
                self.store.delete_tokens(user_id).await?;
                return Err(expired_auth());
            }
            Err(e) => return Err(e.to_service_error()),
        };
        let access = self.cipher.encrypt_str(&fresh.access_token, &access_aad(user_id));
        let refresh = fresh.refresh_token.as_deref().map(|r| self.cipher.encrypt_str(r, &refresh_aad(user_id)));
        store_tokens_on(
            &mut tx,
            user_id,
            &access,
            expiry(now, fresh.expires_in),
            refresh.as_deref(),
            expiry(now, fresh.refresh_token_expires_in),
        )
        .await?;
        tx.commit().await.map_err(bcp_store::StoreError::from)?;
        metrics::counter!("bcp_github_token_refresh_total").increment(1);
        Ok(fresh.access_token)
    }
}
