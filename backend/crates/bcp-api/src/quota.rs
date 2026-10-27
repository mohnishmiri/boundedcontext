//! `GET /api/copilot/quota` — the signed-in user's own Copilot budget (AI credits, or premium requests on
//! legacy billing), read with their token from the endpoint Copilot clients use for the same display.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::auth::CurrentUser;
use crate::error::ApiResult;
use crate::state::AppState;

const QUOTA_TTL: Duration = Duration::from_secs(60);

pub async fn quota(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Value>> {
    let key = format!("quota:{}", user.user.id);
    if let Some(hit) = state.cache.get::<Value>(&key, QUOTA_TTL) {
        return Ok(Json((*hit).clone()));
    }
    let token = state.tokens.copilot_run_token(user.user.id).await?;
    let body = match state.github.get_copilot_user(&token).await {
        Ok(v) => summarize(&v),
        Err(e) => {
            tracing::info!(user = %user.user.login, error = %e, "Copilot quota unavailable");
            unavailable()
        }
    };
    state.cache.put(key, Arc::new(body.clone()));
    Ok(Json(body))
}

fn unavailable() -> Value {
    json!({
        "available": false,
        "message": "GitHub did not return a Copilot budget for your sign-in token.",
    })
}

fn summarize(v: &Value) -> Value {
    let p = &v["quota_snapshots"]["premium_interactions"];
    if !p.is_object() {
        return unavailable();
    }
    let token_based = p["token_based_billing"].as_bool().or_else(|| v["token_based_billing"].as_bool()).unwrap_or(false);
    let entitlement = p["entitlement"].as_f64();
    let remaining = p["quota_remaining"].as_f64().or_else(|| p["remaining"].as_f64());
    let used = p["credits_used"].as_f64().or(match (entitlement, remaining) {
        (Some(e), Some(r)) => Some((e - r).max(0.0)),
        _ => None,
    });
    json!({
        "available": true,
        "login": v["login"],
        "plan": v["copilot_plan"],
        "unit": if token_based { "ai_credits" } else { "premium_requests" },
        "unlimited": p["unlimited"].as_bool().unwrap_or(false),
        "used": used,
        "entitlement": entitlement,
        "remaining": remaining,
        "percent_remaining": p["percent_remaining"].as_f64(),
        "overage_permitted": p["overage_permitted"].as_bool().unwrap_or(false),
        "overage_used": p["overage_count"].as_f64().unwrap_or(0.0),
        "reset_date": v["quota_reset_date_utc"].as_str().or_else(|| v["quota_reset_date"].as_str()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_token_based_budget() {
        let v = json!({
            "login": "mona", "copilot_plan": "business", "quota_reset_date_utc": "2026-11-01T00:00:00.000Z",
            "quota_snapshots": { "premium_interactions": {
                "token_based_billing": true, "unlimited": false, "entitlement": 30000, "quota_remaining": 5566.9,
                "credits_used": 24414, "percent_remaining": 18.5, "overage_permitted": true, "overage_count": 0
            }}
        });
        let s = summarize(&v);
        assert_eq!(s["available"], true);
        assert_eq!(s["unit"], "ai_credits");
        assert_eq!(s["used"], 24414.0);
        assert_eq!(s["entitlement"], 30000.0);
        assert_eq!(s["percent_remaining"], 18.5);
        assert_eq!(s["reset_date"], "2026-11-01T00:00:00.000Z");
    }

    #[test]
    fn derives_used_for_legacy_premium_requests() {
        let v = json!({ "quota_snapshots": { "premium_interactions": { "entitlement": 300, "remaining": 120 }}});
        let s = summarize(&v);
        assert_eq!(s["unit"], "premium_requests");
        assert_eq!(s["used"], 180.0);
    }

    #[test]
    fn missing_snapshot_is_unavailable() {
        assert_eq!(summarize(&json!({ "login": "mona" }))["available"], false);
    }
}
