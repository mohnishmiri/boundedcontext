//! HTTP API of the Bounded Context Portal (Axum). It authenticates users, browses GitHub on their behalf,
//! validates and enqueues generation/publish work, and streams progress. It never runs the agent.

pub mod auth;
pub mod config;
pub mod copilot_models;
pub mod error;
pub mod generators;
pub mod github_routes;
pub mod jobs;
pub mod middleware;
pub mod models;
pub mod sse;
pub mod state;
pub mod view;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{middleware as mw, Json, Router};
use bcp_agent::AgentBundle;
use bcp_core::generators::GeneratorRegistry;
use bcp_core::models::ModelCatalog;
use bcp_core::{AppError, ErrorCode};
use bcp_github::GitHubClient;
use bcp_store::Store;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use tower_http::timeout::TimeoutLayer;

pub use config::ApiConfig;
pub use state::AppState;

use crate::error::ApiError;

pub const MAX_BODY_BYTES: usize = 64 * 1024;

async fn healthz() -> &'static str {
    "ok"
}

async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    match state.store.ready().await {
        Ok(true) => (StatusCode::OK, Json(json!({ "status": "ready" }))),
        Ok(false) => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "status": "migrations_pending" }))),
        Err(e) => {
            tracing::warn!(error = %e, "readiness check failed");
            (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "status": "database_unavailable" })))
        }
    }
}

async fn not_found() -> ApiError {
    ApiError(AppError::new(ErrorCode::NotFound, "No such API endpoint."))
}

pub fn router(state: AppState) -> Router {
    let jobs = Router::new()
        .route("/jobs", get(jobs::list).post(jobs::create))
        .route("/jobs/{id}", get(jobs::get))
        .route("/jobs/{id}/logs", get(jobs::logs))
        .route("/jobs/{id}/diff", get(jobs::diff))
        .route("/jobs/{id}/output", get(jobs::output))
        .route("/jobs/{id}/pull-request", post(jobs::publish))
        .route("/jobs/{id}/cancel", post(jobs::cancel))
        .route("/jobs/{id}/retry", post(jobs::retry))
        .route("/batches", post(jobs::create_batch));

    // SSE streams must not be cut by the request timeout, so they live outside that layer.
    let streaming = Router::new().route("/bounded-context/jobs/{id}/events", get(sse::events));

    let api = Router::new()
        .route("/auth/github", get(auth::login))
        .route("/auth/github/callback", get(auth::callback))
        .route("/auth/me", get(auth::me))
        .route("/auth/logout", post(auth::logout))
        .route("/github/organizations", get(github_routes::organizations))
        .route("/github/repositories", get(github_routes::repositories))
        .route("/github/repositories/{owner}/{repo}", get(github_routes::repository))
        .route("/github/repositories/{owner}/{repo}/branches", get(github_routes::branches))
        .route("/agent", get(github_routes::agent))
        .route("/models", get(models::list))
        .route("/generators", get(generators::list).post(generators::add))
        .route("/generators/{id}", axum::routing::delete(generators::remove))
        .route("/generators/{id}/agent", get(generators::agent_info))
        .nest("/bounded-context", jobs)
        .layer(TimeoutLayer::with_status_code(StatusCode::GATEWAY_TIMEOUT, Duration::from_secs(60)))
        .merge(streaming)
        .fallback(not_found)
        .layer(mw::from_fn_with_state(state.clone(), middleware::csrf))
        .layer(mw::from_fn_with_state(state.clone(), middleware::rate_limit))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(mw::from_fn(middleware::security_headers));

    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .nest("/api", api)
        .layer(mw::from_fn(middleware::http_metrics))
        .layer(mw::from_fn(middleware::request_id))
        .with_state(state)
}

/// Loads everything the API needs from its configuration.
pub async fn build_state(cfg: ApiConfig) -> anyhow::Result<AppState> {
    let store = Store::connect(&cfg.common.database_url, cfg.common.database_max_connections)
        .await
        .context("connecting to PostgreSQL")?;
    if cfg.run_migrations {
        store.migrate().await.context("running migrations")?;
    }
    let github = GitHubClient::new(&cfg.common.github)?;
    let bundle = Arc::new(AgentBundle::load(&cfg.common.agent_bundle_dir).context("loading agent bundle")?);
    let catalog = ModelCatalog::load(&cfg.common.models_file, cfg.common.models_show_unverified)?;
    let registry = GeneratorRegistry::load(&cfg.generators_file)?;
    let copilot_models = match &cfg.copilot_api_url {
        Some(url) => Some(copilot_models::CopilotModelsClient::new(url, &cfg.copilot_integration_id)?),
        None => None,
    };
    if bundle.manifest.placeholder {
        tracing::warn!("the agent bundle is a PLACEHOLDER — install the real agent (agent-bundle/README.md)");
    }
    tracing::info!(agent = bundle.id(), version = %bundle.version, locked_model = ?bundle.locked_model(), "agent bundle loaded");
    Ok(AppState::new(cfg, store, github, bundle, catalog, registry, copilot_models))
}

pub async fn serve(cfg: ApiConfig) -> anyhow::Result<()> {
    bcp_core::telemetry::init_metrics(cfg.metrics_listen)?;
    let listen = cfg.listen;
    let state = build_state(cfg).await?;
    let shutdown = CancellationToken::new();

    tokio::spawn(sse::run_notifier(state.clone(), shutdown.clone()));
    let purge_state = state.clone();
    let purge_shutdown = shutdown.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tokio::select! {
                _ = purge_shutdown.cancelled() => return,
                _ = tick.tick() => {
                    match purge_state.store.purge_expired_sessions().await {
                        Ok(n) if n > 0 => tracing::info!(removed = n, "purged expired sessions"),
                        Ok(_) => {}
                        Err(e) => tracing::warn!(error = %e, "session purge failed"),
                    }
                    let days = purge_state.cfg.job_retention_days;
                    if days > 0 {
                        let cutoff = chrono::Utc::now() - chrono::Duration::days(i64::from(days));
                        match purge_state.store.purge_jobs_before(cutoff).await {
                            Ok(n) if n > 0 => tracing::info!(removed = n, days, "purged jobs past retention"),
                            Ok(_) => {}
                            Err(e) => tracing::warn!(error = %e, "job retention purge failed"),
                        }
                    }
                }
            }
        }
    });

    let listener = tokio::net::TcpListener::bind(listen).await.with_context(|| format!("binding {listen}"))?;
    tracing::info!(%listen, "bcp-api listening");
    let s2 = shutdown.clone();
    axum::serve(listener, router(state))
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            s2.cancel();
        })
        .await?;
    shutdown.cancel();
    Ok(())
}

pub async fn shutdown_signal() {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
    tracing::info!("shutdown signal received");
}
