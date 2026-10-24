//! `/api/bounded-context/jobs` — create, list, inspect, cancel, retry, review and publish generations.

use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use bcp_core::domain::{JobStatus, PublishStatus};
use bcp_core::{validate, AppError, ErrorCode};
use bcp_github::GitHubError;
use bcp_store::{actions, CancelOutcome, JobFilter, JobRow, NewAuditEvent, NewJob, PublishOutcome};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::error::{current_request_id, ApiJson, ApiQuery, ApiResult};
use crate::github_routes::repo_installation;
use crate::state::AppState;
use crate::view;

/// Defaults for new tasks; the worker applies its own configured limits when it claims them.
const DEFAULT_GENERATE_ATTEMPTS: i32 = 2;
const DEFAULT_PUBLISH_ATTEMPTS: i32 = 5;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateJob {
    pub owner: String,
    pub repo: String,
    pub base_branch: String,
    pub model: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, String>,
}

fn parse_job_id(raw: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(raw).map_err(|_| AppError::not_found("Job"))
}

async fn audit(state: &AppState, user: &CurrentUser, action: &str, job: &JobRow, details: Value) {
    state
        .store
        .audit_best_effort(NewAuditEvent {
            actor_user_id: Some(user.user.id),
            actor_login: Some(&user.user.login),
            action,
            job_id: Some(job.id),
            owner: Some(&job.owner),
            repo: Some(&job.repo),
            details,
            request_id: Some(&current_request_id()),
        })
        .await;
}

pub(crate) async fn job_json(state: &AppState, job: &JobRow) -> Result<Value, AppError> {
    let artifacts =
        if job.status() == JobStatus::Succeeded { state.store.list_artifacts(job.id).await? } else { Vec::new() };
    Ok(view::job(job, &artifacts, state.github.web_url().as_str()))
}

/// Validates and authorises a generation request, then enqueues it.
async fn create_job_for(
    state: &AppState,
    user: &CurrentUser,
    req: CreateJob,
    retry_of: Option<Uuid>,
) -> Result<JobRow, AppError> {
    let mut errors: Vec<(&str, String)> = Vec::new();
    if let Err(e) = validate::owner(&req.owner) {
        errors.push(("owner", e.into()));
    }
    if let Err(e) = validate::repo(&req.repo) {
        errors.push(("repo", e.into()));
    }
    if let Err(e) = validate::branch(&req.base_branch) {
        errors.push(("base_branch", e.into()));
    }
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }
    let model = match state.bundle.locked_model() {
        Some(locked) if req.model != locked => {
            return Err(AppError::new(
                ErrorCode::ModelLockedByAgent,
                format!("The bounded-context agent pins the model '{locked}'; it cannot be changed in the portal."),
            )
            .with_details(json!({ "locked_model": locked })))
        }
        Some(locked) => locked.to_owned(),
        None => state.catalog.validate(&req.model)?.id.clone(),
    };
    let inputs = state.bundle.validate_inputs(&req.inputs)?;

    let limit = state.cfg.job_create_limit_per_hour;
    if limit > 0 {
        let recent = state.store.count_jobs_since(user.user.id, Utc::now() - chrono::Duration::hours(1)).await?;
        if recent >= i64::from(limit) {
            return Err(AppError::new(
                ErrorCode::RateLimited,
                format!("You can start at most {limit} generations per hour. Try again later."),
            )
            .with_retry_after(600));
        }
    }

    let token = state.tokens.access_token(user.user.id).await?;
    let repo = state.github.get_repository(&token, &req.owner, &req.repo).await.map_err(|e| e.to_user_error())?;
    let perms = repo.permissions();
    if !perms.pull {
        return Err(AppError::new(
            ErrorCode::RepositoryAccessDenied,
            "You do not have read access to this repository.",
        ));
    }
    if repo.archived {
        return Err(AppError::conflict("This repository is archived."));
    }
    match state.github.get_branch(&token, &repo.owner.login, &repo.name, &req.base_branch).await {
        Ok(_) => {}
        Err(GitHubError::NotFound) => {
            return Err(AppError::validation([("base_branch", "Branch not found in this repository")]))
        }
        Err(e) => return Err(e.to_user_error()),
    }
    let installation_id = repo_installation(state, &repo.owner.login, &repo.name).await?.ok_or_else(|| {
        AppError::new(ErrorCode::AppNotInstalled, "The portal's GitHub App is not installed on this repository.")
            .with_details(json!({ "install_url": state.github.install_url(&state.cfg.common.github.app_slug) }))
    })?;

    let job = state
        .store
        .create_job(NewJob {
            user_id: user.user.id,
            owner: repo.owner.login.clone(),
            repo: repo.name.clone(),
            repo_id: Some(repo.id),
            installation_id: Some(installation_id),
            base_branch: req.base_branch.clone(),
            model,
            inputs: serde_json::to_value(&inputs).expect("serialisable"),
            can_push: perms.push,
            retry_of,
            agent_id: state.agent.id.clone(),
            agent_version: state.agent.version.clone(),
            agent_placeholder: state.agent.placeholder,
            max_attempts: DEFAULT_GENERATE_ATTEMPTS,
        })
        .await?;
    metrics::counter!("bcp_jobs_created_total").increment(1);
    audit(
        state,
        user,
        actions::GENERATION_REQUESTED,
        &job,
        json!({ "model": job.model, "base_branch": job.base_branch, "retry_of": retry_of }),
    )
    .await;
    Ok(job)
}

/// `POST /api/bounded-context/jobs`
pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiJson(req): ApiJson<CreateJob>,
) -> ApiResult<Response> {
    let job = create_job_for(&state, &user, req, None).await?;
    Ok((StatusCode::ACCEPTED, Json(job_json(&state, &job).await?)).into_response())
}

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
    owner: Option<String>,
    repo: Option<String>,
    page: Option<u32>,
    per_page: Option<u32>,
}

/// `GET /api/bounded-context/jobs`
pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiQuery(q): ApiQuery<ListQuery>,
) -> ApiResult<Json<Value>> {
    let status = match q.status.as_deref().filter(|s| !s.is_empty()) {
        Some(s) => Some(JobStatus::parse(s).ok_or_else(|| AppError::validation([("status", "Unknown status")]))?),
        None => None,
    };
    let owner = q.owner.filter(|s| !s.is_empty());
    let repo = q.repo.filter(|s| !s.is_empty());
    if let Some(o) = &owner {
        validate::owner(o).map_err(|e| AppError::validation([("owner", e)]))?;
    }
    if let Some(r) = &repo {
        validate::repo(r).map_err(|e| AppError::validation([("repo", e)]))?;
    }
    let page = q.page.unwrap_or(1).max(1);
    let per_page = q.per_page.unwrap_or(20);
    if per_page == 0 || per_page > 100 {
        return Err(AppError::validation([("per_page", "Must be between 1 and 100")]).into());
    }
    let (rows, total) = state.store.list_jobs(user.user.id, &JobFilter { status, owner, repo }, page, per_page).await?;
    let has_more = i64::from(page) * i64::from(per_page) < total;
    Ok(Json(json!({
        "items": rows.iter().map(view::summary).collect::<Vec<_>>(),
        "page": page, "per_page": per_page, "has_more": has_more, "total": total,
    })))
}

/// `GET /api/bounded-context/jobs/{id}`
pub async fn get(State(state): State<AppState>, user: CurrentUser, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let job = state.store.get_user_job(user.user.id, parse_job_id(&id)?).await?;
    Ok(Json(job_json(&state, &job).await?))
}

#[derive(Deserialize)]
pub struct LogsQuery {
    after: Option<i64>,
    limit: Option<i64>,
}

/// `GET /api/bounded-context/jobs/{id}/logs`
pub async fn logs(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiQuery(q): ApiQuery<LogsQuery>,
) -> ApiResult<Json<Value>> {
    let job = state.store.get_user_job(user.user.id, parse_job_id(&id)?).await?;
    let after = q.after.unwrap_or(0).max(0);
    let limit = q.limit.unwrap_or(200).clamp(1, 500);
    let items = state.store.list_events(job.id, after, limit).await?;
    let next_after = items.last().map(|e| e.seq).unwrap_or(after);
    Ok(Json(json!({ "items": items, "next_after": next_after })))
}

/// `GET /api/bounded-context/jobs/{id}/diff`
pub async fn diff(State(state): State<AppState>, user: CurrentUser, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let job = state.store.get_user_job(user.user.id, parse_job_id(&id)?).await?;
    if job.status() != JobStatus::Succeeded {
        return Err(AppError::conflict("The diff is available once generation has succeeded.").into());
    }
    let artifacts = state.store.list_artifacts(job.id).await?;
    let files: Vec<Value> = artifacts
        .iter()
        .filter(|a| a.allowed)
        .map(|a| {
            json!({
                "path": a.path, "change_type": a.change_type, "base_content": a.base_content,
                "new_content": a.new_content, "unified_diff": a.unified_diff, "size_bytes": a.size_bytes, "sha256": a.sha256,
            })
        })
        .collect();
    let unexpected: Vec<Value> = artifacts
        .iter()
        .filter(|a| !a.allowed)
        .map(|a| json!({ "path": a.path, "change_type": a.change_type }))
        .collect();
    Ok(Json(json!({ "base_sha": job.base_sha, "files": files, "unexpected_changes": unexpected })))
}

/// `GET /api/bounded-context/jobs/{id}/output` — the primary Markdown file as a download.
pub async fn output(State(state): State<AppState>, user: CurrentUser, Path(id): Path<String>) -> ApiResult<Response> {
    let job = state.store.get_user_job(user.user.id, parse_job_id(&id)?).await?;
    if job.status() != JobStatus::Succeeded {
        return Err(AppError::conflict("The output is available once generation has succeeded.").into());
    }
    let primary = job.output_primary.clone().unwrap_or_default();
    let artifact = state
        .store
        .list_artifacts(job.id)
        .await?
        .into_iter()
        .find(|a| a.allowed && a.path == primary)
        .ok_or_else(|| AppError::not_found("Output"))?;
    let base = primary.rsplit('/').next().unwrap_or("bounded-context.md");
    let filename: String = base.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')).collect();
    let filename = if filename.is_empty() { "bounded-context.md".to_owned() } else { filename };
    let mut resp = artifact.new_content.unwrap_or_default().into_response();
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/markdown; charset=utf-8"));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(resp)
}

/// `POST /api/bounded-context/jobs/{id}/pull-request`
pub async fn publish(State(state): State<AppState>, user: CurrentUser, Path(id): Path<String>) -> ApiResult<Response> {
    let job = state.store.get_user_job(user.user.id, parse_job_id(&id)?).await?;
    if job.status() != JobStatus::Succeeded {
        return Err(AppError::conflict("Only a successfully generated job can be published.").into());
    }
    if matches!(job.publish_status(), PublishStatus::Queued | PublishStatus::Running | PublishStatus::Succeeded) {
        return Ok((StatusCode::OK, Json(job_json(&state, &job).await?)).into_response());
    }
    let token = state.tokens.access_token(user.user.id).await?;
    let repo = state.github.get_repository(&token, &job.owner, &job.repo).await.map_err(|e| e.to_user_error())?;
    if !repo.permissions().push {
        return Err(AppError::new(
            ErrorCode::RepositoryAccessDenied,
            "You need write access to this repository to open a pull request.",
        )
        .into());
    }
    if !state.store.list_artifacts(job.id).await?.iter().any(|a| a.allowed) {
        return Err(AppError::conflict("This job produced nothing that can be published.").into());
    }
    match state.store.request_publish(user.user.id, job.id, DEFAULT_PUBLISH_ATTEMPTS, &json!({})).await? {
        PublishOutcome::Queued(job) => {
            audit(&state, &user, actions::PUBLISH_REQUESTED, &job, json!({})).await;
            Ok((StatusCode::ACCEPTED, Json(job_json(&state, &job).await?)).into_response())
        }
        PublishOutcome::Unchanged(job) => Ok((StatusCode::OK, Json(job_json(&state, &job).await?)).into_response()),
    }
}

/// `POST /api/bounded-context/jobs/{id}/cancel`
pub async fn cancel(State(state): State<AppState>, user: CurrentUser, Path(id): Path<String>) -> ApiResult<Response> {
    let job = match state.store.request_cancel(user.user.id, parse_job_id(&id)?).await? {
        CancelOutcome::Cancelled(j) | CancelOutcome::Requested(j) => j,
    };
    audit(&state, &user, actions::GENERATION_CANCELLED, &job, json!({ "status_at_cancel": job.status })).await;
    Ok((StatusCode::ACCEPTED, Json(job_json(&state, &job).await?)).into_response())
}

/// `POST /api/bounded-context/jobs/{id}/retry` — a new job with the same inputs.
pub async fn retry(State(state): State<AppState>, user: CurrentUser, Path(id): Path<String>) -> ApiResult<Response> {
    let old = state.store.get_user_job(user.user.id, parse_job_id(&id)?).await?;
    if !matches!(old.status(), JobStatus::Failed | JobStatus::Cancelled) {
        return Err(AppError::conflict("Only failed or cancelled jobs can be retried.").into());
    }
    let inputs: BTreeMap<String, String> = serde_json::from_value(old.inputs.clone()).unwrap_or_default();
    // Inputs the current agent no longer declares are dropped rather than failing the retry.
    let inputs = inputs.into_iter().filter(|(k, _)| state.bundle.inputs().iter().any(|i| &i.name == k)).collect();
    let req = CreateJob {
        owner: old.owner.clone(),
        repo: old.repo.clone(),
        base_branch: old.base_branch.clone(),
        model: old.model.clone(),
        inputs,
    };
    let job = create_job_for(&state, &user, req, Some(old.id)).await?;
    Ok((StatusCode::ACCEPTED, Json(job_json(&state, &job).await?)).into_response())
}
