//! API behaviour without a worker: authentication, CSRF, browsing, validation and authorisation.

mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use common::{Harness, PUBLIC_URL};
use serde_json::json;

#[tokio::test]
async fn health_and_auth_basics() {
    let Some(h) = Harness::new().await else { return };
    assert_eq!(h.call("GET", "/healthz", None, None).await.status, StatusCode::OK);
    assert_eq!(h.call("GET", "/readyz", None, None).await.status, StatusCode::OK);

    let r = h.call("GET", "/api/auth/me", None, None).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.body["error"]["code"], "unauthenticated");
    assert!(!r.body["error"]["request_id"].as_str().unwrap().is_empty());
    assert_eq!(r.headers["x-request-id"], r.body["error"]["request_id"].as_str().unwrap());

    let cookie = h.login("octocat").await;
    let me = h.get("/api/auth/me", &cookie).await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.body["user"]["login"], "octocat");

    let out = h.post("/api/auth/logout", &cookie, None).await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert_eq!(h.get("/api/auth/me", &cookie).await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn oauth_rejects_tampered_state_and_foreign_redirects() {
    let Some(h) = Harness::new().await else { return };
    // Absolute return_to URLs are ignored (open redirect protection).
    let start = h.call("GET", "/api/auth/github?return_to=https://evil.example.com", None, None).await;
    let oauth = common::set_cookie(&start.headers, "bcp_oauth").unwrap();
    let r =
        h.call("GET", "/api/auth/github/callback?code=x&state=wrong", Some(&format!("bcp_oauth={oauth}")), None).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(r.headers[header::LOCATION], "/?auth_error=state_mismatch");
    let r = h.call("GET", "/api/auth/github/callback?code=x&state=y", None, None).await;
    assert_eq!(r.headers[header::LOCATION], "/?auth_error=state_missing");
    let r = h.call("GET", "/api/auth/github/callback?error=access_denied", None, None).await;
    assert_eq!(r.headers[header::LOCATION], "/?auth_error=access_denied");
}

#[tokio::test]
async fn csrf_protection_on_mutations() {
    let Some(h) = Harness::new().await else { return };
    let cookie = h.login("octocat").await;
    let body = json!({ "owner": "acme", "repo": "shop", "base_branch": "main", "model": "auto" }).to_string();
    let no_header = Request::post("/api/bounded-context/jobs")
        .header(header::COOKIE, &cookie)
        .header(header::ORIGIN, PUBLIC_URL)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.clone()))
        .unwrap();
    let r = h.send(no_header).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.body["error"]["code"], "csrf_rejected");
    let foreign = Request::post("/api/bounded-context/jobs")
        .header(header::COOKIE, &cookie)
        .header(header::ORIGIN, "https://evil.example.com")
        .header("x-requested-with", "bcp")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap();
    assert_eq!(h.send(foreign).await.body["error"]["code"], "csrf_rejected");
}

#[tokio::test]
async fn browsing_is_scoped_to_user_and_installation() {
    let Some(h) = Harness::new().await else { return };
    let cookie = h.login("octocat").await;
    let orgs = h.get("/api/github/organizations", &cookie).await;
    assert_eq!(orgs.status, StatusCode::OK, "{}", orgs.text);
    let logins: Vec<_> = orgs.body["items"].as_array().unwrap().iter().map(|o| o["login"].as_str().unwrap()).collect();
    assert_eq!(logins, ["acme", "octocat"]);
    assert!(orgs.body["install_url"].as_str().unwrap().ends_with("/apps/bcp-test/installations/new"));

    let page1 = h.get("/api/github/repositories?owner=acme&q=svc&per_page=20", &cookie).await;
    assert_eq!(page1.body["items"].as_array().unwrap().len(), 20);
    assert_eq!(page1.body["has_more"], true);
    assert_eq!(page1.body["total"], 35);
    let page2 = h.get("/api/github/repositories?owner=acme&q=svc&per_page=20&page=2", &cookie).await;
    assert_eq!(page2.body["items"].as_array().unwrap().len(), 15);
    assert_eq!(page2.body["has_more"], false);
    let all = h.get("/api/github/repositories?owner=acme&per_page=100", &cookie).await;
    let names: Vec<_> =
        all.body["items"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap().to_owned()).collect();
    assert!(names.contains(&"shop".to_owned()));
    assert!(!names.contains(&"not-installed".to_owned()), "repos without the App are not listed");

    let viewer = h.login("viewer").await;
    let v = h.get("/api/github/repositories?owner=acme&per_page=100", &viewer).await;
    let vnames: Vec<_> = v.body["items"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap()).collect();
    assert_eq!(vnames, ["readonly-docs", "shop"], "only repositories the user can read");
    let detail = h.get("/api/github/repositories/acme/shop", &viewer).await;
    assert_eq!(detail.body["can_generate"], true);
    assert_eq!(detail.body["can_publish"], false);
    assert!(detail.body["reason"].as_str().unwrap().contains("read-only"));
    let hidden = h.get("/api/github/repositories/acme/payments", &viewer).await;
    assert_eq!(hidden.status, StatusCode::FORBIDDEN);
    assert_eq!(hidden.body["error"]["code"], "repository_access_denied");

    let outsider = h.login("outsider").await;
    let none = h.get("/api/github/repositories?owner=acme", &outsider).await;
    assert_eq!(none.status, StatusCode::NOT_FOUND);
    assert_eq!(none.body["error"]["code"], "app_not_installed");

    let branches = h.get("/api/github/repositories/acme/shop/branches", &cookie).await;
    let names: Vec<_> =
        branches.body["items"].as_array().unwrap().iter().map(|b| b["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["main", "develop", "feature/checkout-v2"], "default branch first");
    assert_eq!(branches.body["default_branch"], "main");
    assert_eq!(branches.body["items"][0]["protected"], true);
    let filtered = h.get("/api/github/repositories/acme/shop/branches?q=checkout", &cookie).await;
    assert_eq!(filtered.body["items"].as_array().unwrap().len(), 1);

    let bad = h.get("/api/github/repositories/acme/..%2Fetc/branches", &cookie).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn agent_and_models() {
    let Some(h) = Harness::new().await else { return };
    let cookie = h.login("octocat").await;
    let agent = h.get("/api/agent", &cookie).await;
    assert_eq!(agent.body["id"], "bounded-context");
    assert_eq!(agent.body["placeholder"], true);
    assert_eq!(agent.body["output_paths"][0], "docs/bounded-context.md");
    assert_eq!(agent.body["inputs"][0]["name"], "scope_path");
    // Models come from the user's own Copilot entitlement (mock: chat models that are enabled for octocat).
    let models = h.get("/api/models", &cookie).await;
    assert_eq!(models.body["source"], "copilot", "{}", models.text);
    assert!(models.body["warning"].is_null());
    assert_eq!(models.body["default_model"], "auto");
    let ids: Vec<_> = models.body["items"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["auto", "gpt-5", "claude-sonnet-4.5"], "chat + enabled only, plus auto");
    // A different account sees a different list.
    let viewer = h.login("viewer").await;
    let models = h.get("/api/models", &viewer).await;
    let ids: Vec<_> = models.body["items"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["auto", "gpt-5-mini"]);
}

#[tokio::test]
async fn models_fall_back_to_the_catalog_when_copilot_is_unavailable() {
    let Some(h) = Harness::new().await else { return };
    h.mock.with(|s| s.fail_next("copilot_models", 500, 5));
    let cookie = h.login("octocat").await;
    let models = h.get("/api/models", &cookie).await;
    assert_eq!(models.status, StatusCode::OK);
    assert_eq!(models.body["source"], "catalog");
    assert!(models.body["warning"].as_str().unwrap().contains("standard list"));
    let ids: Vec<_> = models.body["items"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"auto") && !ids.contains(&"claude-opus-5.5"), "unverified models hidden: {ids:?}");
}

#[tokio::test]
async fn jobs_validate_the_model_against_the_users_entitlement() {
    let Some(h) = Harness::new().await else { return };
    let viewer = h.login("viewer").await;
    // `gpt-5` is listed for octocat but not for viewer.
    let r = h
        .post(
            "/api/bounded-context/jobs",
            &viewer,
            Some(json!({ "owner": "acme", "repo": "shop", "base_branch": "main", "model": "gpt-5" })),
        )
        .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", r.text);
    assert_eq!(r.body["error"]["code"], "model_unavailable");
    assert!(r.body["error"]["message"].as_str().unwrap().contains("your GitHub Copilot account"));
}

#[tokio::test]
async fn job_creation_validation_and_authorisation() {
    let Some(h) = Harness::new().await else { return };
    let cookie = h.login("octocat").await;
    let post = |body: serde_json::Value| h.post("/api/bounded-context/jobs", &cookie, Some(body));

    let r = post(json!({ "owner": "acme", "repo": "shop", "base_branch": "bad..name", "model": "auto" })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.body["error"]["details"]["fields"]["base_branch"].is_string());

    let r = post(json!({ "owner": "acme", "repo": "shop", "base_branch": "main", "model": "gpt-unknown" })).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(r.body["error"]["code"], "model_unavailable");

    let r = post(json!({ "owner": "acme", "repo": "shop", "base_branch": "main", "model": "auto", "inputs": { "scope_path": "../etc" } })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.body["error"]["details"]["fields"]["inputs.scope_path"].is_string());

    let r = post(json!({ "owner": "acme", "repo": "shop", "base_branch": "main", "model": "auto", "extra": 1 })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "unknown fields are rejected");

    let r = post(json!({ "owner": "acme", "repo": "shop", "base_branch": "no-such-branch", "model": "auto" })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.body["error"]["details"]["fields"]["base_branch"], "Branch not found in this repository");

    let r = post(json!({ "owner": "acme", "repo": "not-installed", "base_branch": "main", "model": "auto" })).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(r.body["error"]["code"], "app_not_installed");

    let r = post(json!({ "owner": "acme", "repo": "no-such-repo", "base_branch": "main", "model": "auto" })).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.body["error"]["code"], "repository_access_denied");

    let big = "x".repeat(70 * 1024);
    let r = post(json!({ "owner": "acme", "repo": "shop", "base_branch": "main", "model": "auto", "inputs": { "scope_path": big } })).await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);

    let ok = post(json!({ "owner": "acme", "repo": "shop", "base_branch": "develop", "model": "auto", "inputs": { "scope_path": "services/orders" } })).await;
    assert_eq!(ok.status, StatusCode::ACCEPTED, "{}", ok.text);
    assert_eq!(ok.body["status"], "queued");
    assert_eq!(ok.body["inputs"]["scope_path"], "services/orders");
    assert_eq!(ok.body["agent"]["placeholder"], true);
    assert_eq!(ok.body["can_cancel"], true);
    assert_eq!(ok.body["steps"].as_array().unwrap().len(), 5);
    let dup = post(json!({ "owner": "acme", "repo": "shop", "base_branch": "develop", "model": "auto" })).await;
    assert_eq!(dup.status, StatusCode::CONFLICT);
    assert_eq!(dup.body["error"]["code"], "duplicate_job");
    assert_eq!(dup.body["error"]["details"]["job_id"], ok.body["id"]);

    // Jobs are private to their requester.
    let viewer = h.login("viewer").await;
    let id = ok.body["id"].as_str().unwrap();
    assert_eq!(h.get(&format!("/api/bounded-context/jobs/{id}"), &viewer).await.status, StatusCode::NOT_FOUND);
    assert_eq!(h.get("/api/bounded-context/jobs", &viewer).await.body["total"], 0);
    let mine = h.get("/api/bounded-context/jobs?status=queued", &cookie).await;
    assert_eq!(mine.body["total"], 1);

    // Cancel while queued.
    let c = h.post(&format!("/api/bounded-context/jobs/{id}/cancel"), &cookie, None).await;
    assert_eq!(c.status, StatusCode::ACCEPTED);
    assert_eq!(c.body["status"], "cancelled");
    assert_eq!(c.body["can_retry"], true);
    let again = h.post(&format!("/api/bounded-context/jobs/{id}/cancel"), &cookie, None).await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    let diff = h.get(&format!("/api/bounded-context/jobs/{id}/diff"), &cookie).await;
    assert_eq!(diff.status, StatusCode::CONFLICT);
    let retry = h.post(&format!("/api/bounded-context/jobs/{id}/retry"), &cookie, None).await;
    assert_eq!(retry.status, StatusCode::ACCEPTED, "{}", retry.text);
    assert_ne!(retry.body["id"], ok.body["id"]);
    assert_eq!(retry.body["inputs"]["scope_path"], "services/orders");
}

#[tokio::test]
async fn expired_access_tokens_are_refreshed_once() {
    let Some(h) = Harness::new().await else { return };
    let cookie = h.login("octocat").await;
    h.mock.with(|s| s.expire_user_tokens("octocat"));
    sqlx::query("UPDATE github_tokens SET access_expires_at = now() - interval '1 minute'")
        .execute(h.store.pool())
        .await
        .unwrap();
    let r = h.get("/api/github/organizations", &cookie).await;
    assert_eq!(r.status, StatusCode::OK, "refresh should succeed: {}", r.text);
    // The rotated refresh token is persisted; revoking everything forces a new sign-in.
    h.mock.with(|s| s.revoke_user_tokens("octocat"));
    sqlx::query("UPDATE github_tokens SET access_expires_at = now() - interval '1 minute'")
        .execute(h.store.pool())
        .await
        .unwrap();
    let r = h.get("/api/github/repositories/acme/shop", &cookie).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.body["error"]["code"], "github_auth_expired");
}

#[tokio::test]
async fn unknown_routes_and_bad_ids() {
    let Some(h) = Harness::new().await else { return };
    let cookie = h.login("octocat").await;
    let r = h.get("/api/nope", &cookie).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(r.body["error"]["code"], "not_found");
    assert_eq!(h.get("/api/bounded-context/jobs/not-a-uuid", &cookie).await.status, StatusCode::NOT_FOUND);
}
