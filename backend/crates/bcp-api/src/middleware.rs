//! Cross-cutting request handling: request ids, CSRF checks, per-client rate limiting and HTTP metrics.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{MatchedPath, Request, State};
use axum::http::{header, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use bcp_core::crypto::sha256_hex;
use bcp_core::{AppError, ErrorCode};

use crate::auth::SESSION_COOKIE;
use crate::error::{ApiError, REQUEST_ID};
use crate::state::AppState;

pub const REQUEST_ID_HEADER: &str = "x-request-id";
pub const CSRF_HEADER: &str = "x-requested-with";
pub const CSRF_VALUE: &str = "bcp";

/// Accepts a sane inbound `X-Request-Id` (from the ingress) or generates one; exposes it to handlers (error
/// bodies), the tracing span and the response.
pub async fn request_id(mut req: Request, next: Next) -> Response {
    let id = req
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .filter(|v| {
            !v.is_empty() && v.len() <= 128 && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if let Ok(v) = HeaderValue::from_str(&id) {
        req.headers_mut().insert(REQUEST_ID_HEADER, v);
    }
    let span = tracing::info_span!("request", request_id = %id, method = %req.method(), path = %req.uri().path());
    let mut resp = REQUEST_ID.scope(id.clone(), tracing::Instrument::instrument(next.run(req), span)).await;
    if let Ok(v) = HeaderValue::from_str(&id) {
        resp.headers_mut().insert(REQUEST_ID_HEADER, v);
    }
    resp
}

/// Mutating requests must carry `X-Requested-With: bcp` and come from the portal's own origin. Together with
/// the `SameSite=Lax` session cookie this blocks cross-site request forgery.
pub async fn csrf(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let mutating = !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if mutating {
        let headers = req.headers();
        let header_ok = headers.get(CSRF_HEADER).and_then(|v| v.to_str().ok()) == Some(CSRF_VALUE);
        let expected = state.cfg.common.public_origin();
        let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()).map(str::to_owned).or_else(|| {
            headers
                .get(header::REFERER)
                .and_then(|v| v.to_str().ok())
                .and_then(|r| url::Url::parse(r).ok())
                .map(|u| u.origin().ascii_serialization())
        });
        if !header_ok || origin.as_deref() != Some(expected.as_str()) {
            return ApiError(AppError::new(
                ErrorCode::CsrfRejected,
                "The request was rejected by cross-site request protection.",
            ))
            .into_response();
        }
    }
    next.run(req).await
}

/// Fixed-window counter per client (session, else forwarded client address). Per pod; the per-hour job
/// creation limit is enforced in the database across pods.
#[derive(Debug)]
pub struct RateLimiter {
    per_minute: u32,
    windows: Mutex<HashMap<String, (Instant, u32)>>,
}

impl RateLimiter {
    pub fn new(per_minute: u32) -> Self {
        Self { per_minute, windows: Mutex::new(HashMap::new()) }
    }

    /// `Err(seconds until reset)` when the client is over the limit.
    pub fn check(&self, key: &str) -> Result<(), u64> {
        if self.per_minute == 0 {
            return Ok(());
        }
        let now = Instant::now();
        let window = Duration::from_secs(60);
        let mut map = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > 50_000 {
            map.retain(|_, (start, _)| now.duration_since(*start) < window);
        }
        let entry = map.entry(key.to_owned()).or_insert((now, 0));
        if now.duration_since(entry.0) >= window {
            *entry = (now, 0);
        }
        entry.1 += 1;
        if entry.1 > self.per_minute {
            Err(window.saturating_sub(now.duration_since(entry.0)).as_secs().max(1))
        } else {
            Ok(())
        }
    }
}

fn client_key(req: &Request) -> String {
    let cookie = req
        .headers()
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|c| c.trim().split_once('='))
        .find(|(k, _)| *k == SESSION_COOKIE)
        .map(|(_, v)| v.to_owned());
    if let Some(c) = cookie {
        return format!("s:{}", &sha256_hex(c.as_bytes())[..16]);
    }
    let ip = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|v| v.trim().to_owned())
        .unwrap_or_else(|| "unknown".into());
    format!("ip:{ip}")
}

pub async fn rate_limit(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if let Err(retry_after) = state.limiter.check(&client_key(&req)) {
        metrics::counter!("bcp_api_rate_limited_total").increment(1);
        return ApiError(
            AppError::new(ErrorCode::RateLimited, "Too many requests. Slow down and try again shortly.")
                .with_retry_after(retry_after),
        )
        .into_response();
    }
    next.run(req).await
}

pub async fn http_metrics(req: Request, next: Next) -> Response {
    let route =
        req.extensions().get::<MatchedPath>().map(|p| p.as_str().to_owned()).unwrap_or_else(|| "unmatched".into());
    let method = req.method().to_string();
    let started = Instant::now();
    let resp = next.run(req).await;
    let status = resp.status().as_u16().to_string();
    metrics::counter!("bcp_http_requests_total", "route" => route.clone(), "method" => method.clone(), "status" => status).increment(1);
    metrics::histogram!("bcp_http_request_seconds", "route" => route, "method" => method)
        .record(started.elapsed().as_secs_f64());
    resp
}

/// Baseline security headers on every API response.
pub async fn security_headers(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.entry(header::CACHE_CONTROL).or_insert(HeaderValue::from_static("no-store"));
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_blocks_after_quota() {
        let l = RateLimiter::new(2);
        assert!(l.check("a").is_ok());
        assert!(l.check("a").is_ok());
        assert!(l.check("a").unwrap_err() >= 1);
        assert!(l.check("b").is_ok());
        assert!(RateLimiter::new(0).check("a").is_ok());
    }
}
