//! GitHub App OAuth web flow (state + PKCE), opaque server-side sessions, and the `CurrentUser` extractor.

use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use bcp_core::crypto::{constant_time_eq, pkce_challenge, random_token, sha256};
use bcp_core::{validate, AppError, ErrorCode};
use bcp_store::{actions, NewAuditEvent, UserRow};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{current_request_id, ApiError, ApiQuery, ApiResult};
use crate::state::AppState;

pub const SESSION_COOKIE: &str = "bcp_session";
pub const OAUTH_COOKIE: &str = "bcp_oauth";
const OAUTH_AAD: &[u8] = b"bcp-oauth-state";
const OAUTH_TTL_SECS: i64 = 600;

#[derive(Debug, Clone, Serialize)]
pub struct UserView {
    pub id: String,
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

impl From<&UserRow> for UserView {
    fn from(u: &UserRow) -> Self {
        Self { id: u.id.to_string(), login: u.login.clone(), name: u.name.clone(), avatar_url: u.avatar_url.clone() }
    }
}

/// The signed-in user, resolved from the session cookie.
#[derive(Debug, Clone)]
pub struct CurrentUser {
    pub user: UserRow,
    pub session_hash: [u8; 32],
}

pub fn read_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|c| c.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_owned())
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let unauth = || ApiError(AppError::new(ErrorCode::Unauthenticated, "Sign in with GitHub to continue."));
        let raw = read_cookie(&parts.headers, SESSION_COOKIE)
            .filter(|v| v.len() >= 32 && v.len() <= 128)
            .ok_or_else(unauth)?;
        let hash = sha256(raw.as_bytes());
        let session = state.store.session_user(&hash).await?.ok_or_else(unauth)?;
        Ok(CurrentUser { user: session.user, session_hash: hash })
    }
}

fn cookie(state: &AppState, name: &str, value: &str, path: &str, max_age: i64) -> HeaderValue {
    let secure = if state.cfg.cookie_secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!("{name}={value}; Path={path}; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure}"))
        .expect("valid cookie")
}

#[derive(Serialize, Deserialize)]
struct OAuthState {
    state: String,
    verifier: String,
    return_to: String,
    exp: i64,
}

#[derive(Deserialize)]
pub struct LoginQuery {
    return_to: Option<String>,
}

/// `GET /api/auth/github` — starts the OAuth flow.
pub async fn login(State(state): State<AppState>, ApiQuery(q): ApiQuery<LoginQuery>) -> Response {
    let return_to = q.return_to.as_deref().and_then(validate::return_to_path).unwrap_or("/").to_owned();
    let st = OAuthState {
        state: random_token(24),
        verifier: random_token(48),
        return_to,
        exp: Utc::now().timestamp() + OAUTH_TTL_SECS,
    };
    let url = state.github.authorize_url(&state.cfg.redirect_uri(), &st.state, &pkce_challenge(&st.verifier));
    let sealed = state.cfg.common.cipher.seal_url_safe(&serde_json::to_vec(&st).expect("serialisable"), OAUTH_AAD);
    let mut resp = Redirect::to(url.as_str()).into_response();
    resp.headers_mut().append(header::SET_COOKIE, cookie(&state, OAUTH_COOKIE, &sealed, "/api/auth", OAUTH_TTL_SECS));
    resp
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

fn auth_failure(state: &AppState, code: &str) -> Response {
    tracing::warn!(code, "sign-in failed");
    metrics::counter!("bcp_auth_failures_total", "code" => code.to_owned()).increment(1);
    let mut resp = Redirect::to(&format!("/?auth_error={code}")).into_response();
    resp.headers_mut().append(header::SET_COOKIE, cookie(state, OAUTH_COOKIE, "", "/api/auth", 0));
    resp
}

/// `GET /api/auth/github/callback` — completes the OAuth flow and creates a session.
pub async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiQuery(q): ApiQuery<CallbackQuery>,
) -> Response {
    if let Some(e) = q.error.as_deref() {
        return auth_failure(&state, if e == "access_denied" { "access_denied" } else { "github_error" });
    }
    let (Some(code), Some(returned_state)) = (q.code.as_deref(), q.state.as_deref()) else {
        return auth_failure(&state, "invalid_request");
    };
    let Some(st) = read_cookie(&headers, OAUTH_COOKIE)
        .and_then(|c| state.cfg.common.cipher.open_url_safe(&c, OAUTH_AAD).ok())
        .and_then(|raw| serde_json::from_slice::<OAuthState>(&raw).ok())
    else {
        return auth_failure(&state, "state_missing");
    };
    if st.exp < Utc::now().timestamp() || !constant_time_eq(st.state.as_bytes(), returned_state.as_bytes()) {
        return auth_failure(&state, "state_mismatch");
    }
    let tokens = match state.github.exchange_code(code, &state.cfg.redirect_uri(), &st.verifier).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "OAuth code exchange failed");
            return auth_failure(&state, "exchange_failed");
        }
    };
    let gh_user = match state.github.get_authenticated_user(&tokens.access_token).await {
        Ok(u) => u,
        Err(e) => {
            tracing::warn!(error = %e, "fetching GitHub user failed");
            return auth_failure(&state, "github_unavailable");
        }
    };
    let result: Result<Response, AppError> = async {
        let user = state
            .store
            .upsert_user(gh_user.id, &gh_user.login, gh_user.name.as_deref(), gh_user.avatar_url.as_deref())
            .await?;
        state.tokens.save(user.id, &tokens).await?;
        let session_id = random_token(32);
        let expires =
            Utc::now() + chrono::Duration::from_std(state.cfg.session_ttl).unwrap_or(chrono::Duration::hours(12));
        state.store.create_session(&sha256(session_id.as_bytes()), user.id, expires).await?;
        state
            .store
            .audit_best_effort(NewAuditEvent {
                actor_user_id: Some(user.id),
                actor_login: Some(&user.login),
                action: actions::USER_SIGNED_IN,
                request_id: Some(&current_request_id()),
                ..Default::default()
            })
            .await;
        let mut resp = Redirect::to(&st.return_to).into_response();
        let h = resp.headers_mut();
        h.append(
            header::SET_COOKIE,
            cookie(&state, SESSION_COOKIE, &session_id, "/", state.cfg.session_ttl.as_secs() as i64),
        );
        h.append(header::SET_COOKIE, cookie(&state, OAUTH_COOKIE, "", "/api/auth", 0));
        Ok(resp)
    }
    .await;
    result.unwrap_or_else(|e| {
        tracing::error!(error = %e, "completing sign-in failed");
        auth_failure(&state, "internal")
    })
}

/// `GET /api/auth/me`
pub async fn me(user: CurrentUser) -> Json<serde_json::Value> {
    Json(json!({ "user": UserView::from(&user.user) }))
}

/// `POST /api/auth/logout`
pub async fn logout(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Response> {
    state.store.delete_session(&user.session_hash).await?;
    state
        .store
        .audit_best_effort(NewAuditEvent {
            actor_user_id: Some(user.user.id),
            actor_login: Some(&user.user.login),
            action: actions::USER_SIGNED_OUT,
            request_id: Some(&current_request_id()),
            ..Default::default()
        })
        .await;
    let mut resp = StatusCode::NO_CONTENT.into_response();
    resp.headers_mut().append(header::SET_COOKIE, cookie(&state, SESSION_COOKIE, "", "/", 0));
    Ok(resp)
}
