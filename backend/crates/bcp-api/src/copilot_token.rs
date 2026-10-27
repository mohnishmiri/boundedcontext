//! `/api/copilot/token` — an optional per-user token used only to run Copilot, so users who sign in with a
//! classic token (which Copilot rejects) can still spend their own credits. A fine-grained token owned by the
//! user's own account with only the "Copilot Requests" permission needs no organisation approval.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use bcp_core::{AppError, ErrorCode};
use bcp_github::GitHubError;
use bcp_store::NewAuditEvent;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::CurrentUser;
use crate::error::{current_request_id, ApiJson, ApiResult};
use crate::state::AppState;

/// `GET /api/copilot/token`
pub async fn status(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Value>> {
    Ok(Json(status_json(&state, &user).await?))
}

async fn status_json(state: &AppState, user: &CurrentUser) -> Result<Value, AppError> {
    let configured = state.tokens.copilot_token(user.user.id).await?.is_some();
    let sign_in_classic = state.tokens.access_token(user.user.id).await?.starts_with("ghp_");
    let shared = state.cfg.copilot_shared_token;
    Ok(json!({
        "configured": configured,
        "sign_in_token_classic": sign_in_classic,
        "shared": shared,
        "needed": !shared && !configured && sign_in_classic,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveToken {
    token: String,
}

/// `POST /api/copilot/token` — validates the token belongs to the signed-in user, then stores it encrypted.
pub async fn save(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiJson(req): ApiJson<SaveToken>,
) -> ApiResult<Json<Value>> {
    let token = req.token.trim();
    if token.len() < 20 || token.len() > 255 || !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(AppError::validation([("token", "This does not look like a GitHub token.")]).into());
    }
    if token.starts_with("ghp_") {
        return Err(AppError::validation([(
            "token",
            "Copilot does not accept classic tokens (ghp_). Create a fine-grained token with the 'Copilot Requests' permission.",
        )])
        .into());
    }
    let owner = match state.github.get_authenticated_user(token).await {
        Ok(u) => u,
        Err(GitHubError::Unauthorized) => {
            return Err(AppError::validation([("token", "GitHub rejected this token. Check it has not expired.")]).into())
        }
        Err(e) => return Err(e.to_service_error().into()),
    };
    if owner.id != user.user.github_id {
        return Err(AppError::new(
            ErrorCode::Forbidden,
            format!("This token belongs to {}; use a token from your own account ({}).", owner.login, user.user.login),
        )
        .into());
    }
    state.tokens.save_copilot_token(user.user.id, token).await?;
    forget_cached(&state, &user);
    audit(&state, &user, "copilot_token_saved").await;
    Ok(Json(status_json(&state, &user).await?))
}

/// `DELETE /api/copilot/token`
pub async fn remove(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Response> {
    state.tokens.delete_copilot_token(user.user.id).await?;
    forget_cached(&state, &user);
    audit(&state, &user, "copilot_token_removed").await;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// The budget and model list both depend on which Copilot token is used.
fn forget_cached(state: &AppState, user: &CurrentUser) {
    state.cache.remove(&format!("quota:{}", user.user.id));
    state.cache.remove(&format!("models:{}", user.user.id));
}

async fn audit(state: &AppState, user: &CurrentUser, action: &str) {
    state
        .store
        .audit_best_effort(NewAuditEvent {
            actor_user_id: Some(user.user.id),
            actor_login: Some(&user.user.login),
            action,
            request_id: Some(&current_request_id()),
            ..Default::default()
        })
        .await;
}
