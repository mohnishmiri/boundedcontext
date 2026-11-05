//! `/api/bounded-context/jobs` — create, list, inspect, cancel, retry, review and publish generations.

use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use bcp_core::domain::{JobStatus, PublishStatus};
use bcp_core::generators::BUILTIN_ID;
use bcp_core::{validate, AppError, ErrorCode};
use bcp_github::GitHubError;
use bcp_store::{actions, CancelOutcome, JobFilter, JobRow, NewAuditEvent, NewJob, PublishOptions, PublishOutcome};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::error::{current_request_id, ApiJson, ApiQuery, ApiResult};
use crate::generators::{self, SelectedGenerator};
use crate::github_routes::repo_installation;
use crate::models;
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
    /// `builtin` (default), a curated generator id, or a saved generator's id.
    #[serde(default)]
    pub generator_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatchRepository {
    pub owner: String,
    pub repo: String,
    pub base_branch: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBatch {
    pub generator_id: String,
    pub model: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, String>,
    pub repositories: Vec<BatchRepository>,
}

/// How many repositories of one batch are checked against GitHub at the same time.
const BATCH_CONCURRENCY: usize = 5;

fn parse_job_id(raw: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(raw).map_err(|_| AppError::not_found("Job"))
}

/// A job the user may read: their own or, for administrators, anyone's. Changing a job always requires
/// ownership (`Store::get_user_job`).
pub(crate) async fn readable_job(state: &AppState, user: &CurrentUser, id: Uuid) -> Result<JobRow, AppError> {
    if user.is_admin {
        return state.store.get_job(id).await?.ok_or_else(|| AppError::not_found("Job"));
    }
    Ok(state.store.get_user_job(user.user.id, id).await?)
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
    job_json_as(state, job, true).await
}

/// The job as seen by its owner, or by an administrator viewing another user's job (`owner_view == false`): they
/// may only publish it on the requester's behalf (their own push access is checked when they do).
pub(crate) async fn job_json_as(state: &AppState, job: &JobRow, owner_view: bool) -> Result<Value, AppError> {
    let artifacts =
        if job.status() == JobStatus::Succeeded { state.store.list_artifacts(job.id).await? } else { Vec::new() };
    let central_repo = job
        .central_publish_repo
        .clone()
        .or_else(|| central_repo_for(state, job).map(|(owner, repo)| format!("{owner}/{repo}")));
    let mut v = view::job(job, &artifacts, state.github.web_url().as_str(), central_repo.as_deref());
    v["viewer_is_owner"] = json!(owner_view);
    if !owner_view {
        v["can_cancel"] = json!(false);
        v["can_retry"] = json!(false);
        v["can_publish"] = json!(view::can_publish_as_admin(job, &artifacts));
    }
    Ok(v)
}

/// The central repository for this job's generator: the registry's `central_repo`, else `BCP_CENTRAL_REPO`.
fn central_repo_for(state: &AppState, job: &JobRow) -> Option<(String, String)> {
    match state.registry.central_repo_for(job.generator_id.as_deref()).and_then(|v| v.split_once('/')) {
        Some((owner, repo)) => Some((owner.to_owned(), repo.to_owned())),
        None => state.cfg.common.central_repository.clone(),
    }
}

/// Everything about a request that is the same for every repository in it: the chosen generator, the validated
/// model and inputs, and the user's token.
struct RequestContext {
    token: String,
    generator: SelectedGenerator,
    model: String,
    inputs: BTreeMap<String, String>,
    batch_id: Option<Uuid>,
}

/// Resolves the generator and validates model and inputs against it (and, for the model, against the list the
/// user's own Copilot entitlement allows).
async fn prepare(
    state: &AppState,
    user: &CurrentUser,
    generator_id: &str,
    model: &str,
    raw_inputs: &BTreeMap<String, String>,
    batch_id: Option<Uuid>,
) -> Result<RequestContext, AppError> {
    let token = state.tokens.access_token(user.user.id).await?;
    let generator = generators::select(state, user, &token, generator_id).await?;
    prepare_with(state, user, token, generator, model, raw_inputs, batch_id).await
}

async fn prepare_with(
    state: &AppState,
    user: &CurrentUser,
    token: String,
    generator: SelectedGenerator,
    model: &str,
    raw_inputs: &BTreeMap<String, String>,
    batch_id: Option<Uuid>,
) -> Result<RequestContext, AppError> {
    if !state.cfg.copilot_shared_token
        && token.starts_with("ghp_")
        && state.tokens.copilot_token(user.user.id).await?.is_none()
    {
        return Err(AppError::new(
            ErrorCode::CopilotUnauthorized,
            "Generation runs on your own Copilot credits, and Copilot does not accept classic tokens (ghp_). \
             Add a Copilot token (banner at the top of the page) - a fine-grained token with only the 'Copilot \
             Requests' permission, which needs no organisation approval.",
        ));
    }
    let copilot = state.tokens.copilot_run_token(user.user.id).await?;
    if !state.cfg.copilot_shared_token {
        crate::copilot_token::ensure_copilot_accepts(state, &copilot).await?;
    }
    let model = match generator.bundle.locked_model() {
        Some(locked) if model != locked => {
            return Err(AppError::new(
                ErrorCode::ModelLockedByAgent,
                format!("The generator's agent pins the model '{locked}'; it cannot be changed in the portal."),
            )
            .with_details(json!({ "locked_model": locked })))
        }
        Some(locked) => locked.to_owned(),
        None => models::model_list(state, user.user.id, &copilot).await.validate(model)?.id.clone(),
    };
    let inputs = generator.bundle.validate_inputs(raw_inputs)?;
    Ok(RequestContext { token, generator, model, inputs, batch_id })
}

/// Validates and authorises one repository, then enqueues its generation.
async fn create_job_for(
    state: &AppState,
    user: &CurrentUser,
    ctx: &RequestContext,
    (owner, repo_name, base_branch): (&str, &str, &str),
    retry_of: Option<Uuid>,
) -> Result<JobRow, AppError> {
    let mut errors: Vec<(&str, String)> = Vec::new();
    if let Err(e) = validate::owner(owner) {
        errors.push(("owner", e.into()));
    }
    if let Err(e) = validate::repo(repo_name) {
        errors.push(("repo", e.into()));
    }
    if let Err(e) = validate::branch(base_branch) {
        errors.push(("base_branch", e.into()));
    }
    if !errors.is_empty() {
        return Err(AppError::validation(errors));
    }

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

    let token = &ctx.token;
    let repo = state.github.get_repository(token, owner, repo_name).await.map_err(|e| e.to_user_error())?;
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
    match state.github.get_branch(token, &repo.owner.login, &repo.name, base_branch).await {
        Ok(_) => {}
        Err(GitHubError::NotFound) => {
            return Err(AppError::validation([("base_branch", "Branch not found in this repository")]))
        }
        Err(e) => return Err(e.to_user_error()),
    }
    let installation_id = if state.github.has_app() {
        Some(repo_installation(state, &repo.owner.login, &repo.name).await?.ok_or_else(|| {
            AppError::new(ErrorCode::AppNotInstalled, "The portal's GitHub App is not installed on this repository.")
                .with_details(json!({ "install_url": state.github.install_url(&state.cfg.common.github.app_slug) }))
        })?)
    } else {
        None
    };

    let bundle = &ctx.generator.bundle;
    let job = state
        .store
        .create_job(NewJob {
            user_id: user.user.id,
            owner: repo.owner.login.clone(),
            repo: repo.name.clone(),
            repo_id: Some(repo.id),
            installation_id,
            base_branch: base_branch.to_owned(),
            model: ctx.model.clone(),
            inputs: serde_json::to_value(&ctx.inputs).expect("serialisable"),
            can_push: perms.push,
            retry_of,
            agent_id: bundle.id().to_owned(),
            agent_version: bundle.version.clone(),
            agent_placeholder: bundle.manifest.placeholder,
            max_attempts: DEFAULT_GENERATE_ATTEMPTS,
            batch_id: ctx.batch_id,
            generator: ctx.generator.job.clone(),
            output_paths: bundle.output_paths().to_vec(),
        })
        .await?;
    metrics::counter!("bcp_jobs_created_total").increment(1);
    audit(
        state,
        user,
        actions::GENERATION_REQUESTED,
        &job,
        json!({
            "model": job.model, "base_branch": job.base_branch, "retry_of": retry_of, "batch_id": ctx.batch_id,
            "generator": { "source": job.generator_source, "key": job.generator_key, "sha": job.generator_sha },
        }),
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
    let generator_id = req.generator_id.as_deref().unwrap_or(BUILTIN_ID);
    let ctx = prepare(&state, &user, generator_id, &req.model, &req.inputs, None).await?;
    let job = create_job_for(&state, &user, &ctx, (&req.owner, &req.repo, &req.base_branch), None).await?;
    Ok((StatusCode::ACCEPTED, Json(job_json(&state, &job).await?)).into_response())
}

fn failure_json(owner: &str, repo: &str, base_branch: &str, err: &AppError) -> Value {
    let mut error = json!({ "code": err.code, "message": err.message });
    if let Some(d) = &err.details {
        error["details"] = d.clone();
    }
    json!({ "owner": owner, "repo": repo, "base_branch": base_branch, "error": error })
}

/// `POST /api/bounded-context/batches` — one generation per selected repository. Repositories are checked
/// independently: the ones that pass are queued and the rest are reported in `failures`.
pub async fn create_batch(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiJson(req): ApiJson<CreateBatch>,
) -> ApiResult<Response> {
    let max = state.cfg.batch_max_repos;
    if req.repositories.is_empty() {
        return Err(AppError::validation([("repositories", "Select at least one repository")]).into());
    }
    if req.repositories.len() > max {
        return Err(
            AppError::validation([("repositories", format!("Select at most {max} repositories at once"))]).into()
        );
    }
    let mut seen = std::collections::HashSet::new();
    for r in &req.repositories {
        let key = format!("{}/{}@{}", r.owner.to_ascii_lowercase(), r.repo.to_ascii_lowercase(), r.base_branch);
        if !seen.insert(key) {
            return Err(AppError::validation([(
                "repositories",
                format!("{}/{}@{} is listed more than once", r.owner, r.repo, r.base_branch),
            )])
            .into());
        }
    }
    let batch_id = Uuid::new_v4();
    let ctx = prepare(&state, &user, &req.generator_id, &req.model, &req.inputs, Some(batch_id)).await?;

    // Checked a few at a time so one large batch does not hit GitHub with dozens of simultaneous requests.
    let mut results: Vec<Result<JobRow, AppError>> = Vec::with_capacity(req.repositories.len());
    for chunk in req.repositories.chunks(BATCH_CONCURRENCY) {
        let chunk_results = futures::future::join_all(
            chunk.iter().map(|r| create_job_for(&state, &user, &ctx, (&r.owner, &r.repo, &r.base_branch), None)),
        )
        .await;
        results.extend(chunk_results);
    }

    let mut jobs = Vec::new();
    let mut failures = Vec::new();
    for (r, result) in req.repositories.iter().zip(results) {
        match result {
            Ok(job) => jobs.push(view::summary(&job)),
            Err(e) => failures.push(failure_json(&r.owner, &r.repo, &r.base_branch, &e)),
        }
    }
    Ok((StatusCode::ACCEPTED, Json(json!({ "batch_id": batch_id, "jobs": jobs, "failures": failures })))
        .into_response())
}

#[derive(Deserialize)]
pub struct ListQuery {
    status: Option<String>,
    /// `mine` (default) or `all` (administrators).
    scope: Option<String>,
    /// Requester login; administrators with `scope=all` only.
    requested_by: Option<String>,
    owner: Option<String>,
    repo: Option<String>,
    batch_id: Option<String>,
    /// Free-text search over repository, branch, requester and job id.
    q: Option<String>,
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
    let batch_id = match q.batch_id.as_deref().filter(|s| !s.is_empty()) {
        Some(b) => Some(Uuid::parse_str(b).map_err(|_| AppError::validation([("batch_id", "Not a valid batch id")]))?),
        None => None,
    };
    let page = q.page.unwrap_or(1).max(1);
    let per_page = q.per_page.unwrap_or(20);
    if per_page == 0 || per_page > 100 {
        return Err(AppError::validation([("per_page", "Must be between 1 and 100")]).into());
    }
    let all_users = match q.scope.as_deref().unwrap_or("mine") {
        "mine" => false,
        "all" if user.is_admin => true,
        "all" => {
            return Err(AppError::new(ErrorCode::Forbidden, "Only administrators can see every user's jobs.").into())
        }
        _ => return Err(AppError::validation([("scope", "Must be 'mine' or 'all'")]).into()),
    };
    let requested_by = q.requested_by.map(|s| s.trim().trim_start_matches('@').to_owned()).filter(|s| !s.is_empty());
    if let Some(r) = &requested_by {
        if !all_users {
            return Err(AppError::validation([("requested_by", "Only available with scope=all")]).into());
        }
        validate::user_login(r).map_err(|e| AppError::validation([("requested_by", e)]))?;
    }
    let scope = (!all_users).then_some(user.user.id);
    let search = q.q.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    if search.as_ref().is_some_and(|s| s.chars().count() > 100 || s.chars().any(char::is_control)) {
        return Err(AppError::validation([("q", "At most 100 characters, no control characters")]).into());
    }
    let filter = JobFilter { status, owner, repo, batch_id, requested_by, search };
    let (rows, total) = state.store.list_jobs(scope, &filter, page, per_page).await?;
    let mut counts = json!({ "all": 0, "queued": 0, "running": 0, "succeeded": 0, "failed": 0, "cancelled": 0 });
    let mut all = 0;
    for (status, n) in state.store.count_jobs_by_status(scope, &filter).await? {
        all += n;
        counts[status] = json!(n);
    }
    counts["all"] = json!(all);
    let has_more = i64::from(page) * i64::from(per_page) < total;
    Ok(Json(json!({
        "items": rows.iter().map(view::summary).collect::<Vec<_>>(),
        "page": page, "per_page": per_page, "has_more": has_more, "total": total, "counts": counts,
        "scope": if all_users { "all" } else { "mine" },
    })))
}

/// `GET /api/bounded-context/jobs/{id}`
pub async fn get(State(state): State<AppState>, user: CurrentUser, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let job = readable_job(&state, &user, parse_job_id(&id)?).await?;
    let owner_view = job.user_id == user.user.id;
    if !owner_view {
        audit(&state, &user, "admin_job_viewed", &job, json!({ "requested_by": job.requested_by })).await;
    }
    Ok(Json(job_json_as(&state, &job, owner_view).await?))
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
    let job = readable_job(&state, &user, parse_job_id(&id)?).await?;
    let after = q.after.unwrap_or(0).max(0);
    let limit = q.limit.unwrap_or(200).clamp(1, 500);
    let items = state.store.list_events(job.id, after, limit).await?;
    let next_after = items.last().map(|e| e.seq).unwrap_or(after);
    Ok(Json(json!({ "items": items, "next_after": next_after })))
}

/// `GET /api/bounded-context/jobs/{id}/diff`
pub async fn diff(State(state): State<AppState>, user: CurrentUser, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let job = readable_job(&state, &user, parse_job_id(&id)?).await?;
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
    let job = readable_job(&state, &user, parse_job_id(&id)?).await?;
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

/// Largest archive the API builds in memory (generated documentation is a few MiB at most).
const MAX_ARCHIVE_BYTES: i64 = 64 * 1024 * 1024;

/// `GET /api/bounded-context/jobs/{id}/archive` — every generated file as a ZIP, under `<repo>/<path>`.
pub async fn archive(State(state): State<AppState>, user: CurrentUser, Path(id): Path<String>) -> ApiResult<Response> {
    let job = readable_job(&state, &user, parse_job_id(&id)?).await?;
    if job.status() != JobStatus::Succeeded {
        return Err(AppError::conflict("The output is available once generation has succeeded.").into());
    }
    let artifacts: Vec<_> = state.store.list_artifacts(job.id).await?.into_iter().filter(|a| a.allowed).collect();
    if artifacts.is_empty() {
        return Err(AppError::not_found("Output").into());
    }
    if artifacts.iter().map(|a| a.size_bytes).sum::<i64>() > MAX_ARCHIVE_BYTES {
        return Err(AppError::conflict("The output is too large to download as one archive.").into());
    }
    let root = safe_file_name(&job.repo, "output");
    let bytes = tokio::task::spawn_blocking(move || {
        let mut zip = crate::zip::ZipWriter::default();
        for a in &artifacts {
            // Stored paths passed `validate::relative_path` when the worker collected them.
            zip.add(&format!("{root}/{}", a.path), a.new_content.as_deref().unwrap_or_default().as_bytes())?;
        }
        zip.finish()
    })
    .await
    .map_err(|e| AppError::internal(format!("building archive: {e}")))?
    .map_err(|e| AppError::internal(format!("building archive: {e}")))?;
    let filename = format!("{}-{}-docs.zip", safe_file_name(&job.repo, "output"), job.short_id());
    let mut resp = bytes.into_response();
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/zip"));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(resp)
}

/// `raw` reduced to `[A-Za-z0-9._-]` for use in file names; `fallback` when nothing remains.
fn safe_file_name(raw: &str, fallback: &str) -> String {
    let s: String = raw.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')).collect();
    let s = s.trim_start_matches('.').to_owned();
    if s.is_empty() {
        fallback.to_owned()
    } else {
        s
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishRequest {
    pub title: String,
    #[serde(default)]
    pub publish_central: bool,
    /// GitHub logins asked to review the pull request(s).
    #[serde(default)]
    pub reviewers: Vec<String>,
    /// `owner/repo` for the central pull request instead of the generator's default.
    #[serde(default)]
    pub central_repo: Option<String>,
}

/// `owner/repo` typed by the user, validated like every other repository identifier.
fn parse_repo_slug(raw: &str) -> Result<(String, String), AppError> {
    let invalid = || AppError::validation([("central_repo", "Enter the repository as owner/repository.")]);
    let (owner, repo) = raw.trim().split_once('/').ok_or_else(invalid)?;
    if validate::owner(owner).is_err() || validate::repo(repo).is_err() {
        return Err(invalid());
    }
    Ok((owner.to_owned(), repo.to_owned()))
}

const MAX_REVIEWERS: usize = 10;

/// Trimmed, de-duplicated reviewer logins (a leading `@` is accepted).
fn clean_reviewers(state: &AppState, user: &CurrentUser, raw: &[String]) -> Result<Vec<String>, AppError> {
    let mut out: Vec<String> = Vec::new();
    for r in raw {
        let login = r.trim().trim_start_matches('@');
        if login.is_empty() || out.iter().any(|o| o.eq_ignore_ascii_case(login)) {
            continue;
        }
        if validate::user_login(login).is_err() {
            return Err(AppError::validation([("reviewers", format!("'{login}' is not a valid GitHub username."))]));
        }
        // Without a GitHub App the requester authors the pull request, and GitHub refuses self-review.
        if !state.github.has_app() && login.eq_ignore_ascii_case(&user.user.login) {
            return Err(AppError::validation([(
                "reviewers",
                "You open the pull request, so GitHub does not let you review it. Remove your own username.",
            )]));
        }
        out.push(login.to_owned());
    }
    if out.len() > MAX_REVIEWERS {
        return Err(AppError::validation([("reviewers", format!("Add at most {MAX_REVIEWERS} reviewers."))]));
    }
    Ok(out)
}

/// `POST /api/bounded-context/jobs/{id}/pull-request` — by the requester, or by an administrator on their behalf
/// (with the administrator's own push access).
pub async fn publish(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    ApiJson(req): ApiJson<PublishRequest>,
) -> ApiResult<Response> {
    let job = readable_job(&state, &user, parse_job_id(&id)?).await?;
    let owner_view = job.user_id == user.user.id;
    if job.status() != JobStatus::Succeeded {
        return Err(AppError::conflict("Only a successfully generated job can be published.").into());
    }
    if matches!(job.publish_status(), PublishStatus::Queued | PublishStatus::Running | PublishStatus::Succeeded) {
        return Ok((StatusCode::OK, Json(job_json_as(&state, &job, owner_view).await?)).into_response());
    }
    let title = req.title.trim();
    if title.is_empty() || title.len() > 256 || title.chars().any(char::is_control) {
        return Err(AppError::validation([(
            "title",
            "Enter a pull request title of 1 to 256 characters without control characters.",
        )])
        .into());
    }
    let reviewers = clean_reviewers(&state, &user, &req.reviewers)?;
    let token = state.tokens.access_token(user.user.id).await?;
    let repo = state.github.get_repository(&token, &job.owner, &job.repo).await.map_err(|e| e.to_user_error())?;
    if !repo.permissions().push {
        return Err(AppError::new(
            ErrorCode::RepositoryAccessDenied,
            "You need write access to this repository to open a pull request.",
        )
        .into());
    }
    let central = if req.publish_central {
        let chosen = match req.central_repo.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(raw) => Some(parse_repo_slug(raw)?),
            None => central_repo_for(&state, &job),
        };
        let Some((central_owner, central_repo)) = chosen else {
            return Err(
                AppError::validation([("central_repo", "Enter the central repository as owner/repository.")]).into()
            );
        };
        if central_owner.eq_ignore_ascii_case(&job.owner) && central_repo.eq_ignore_ascii_case(&job.repo) {
            return Err(AppError::conflict("The central repository cannot also be the source repository.").into());
        }
        let central =
            state.github.get_repository(&token, &central_owner, &central_repo).await.map_err(|e| e.to_user_error())?;
        if !central.permissions().push {
            return Err(AppError::new(
                ErrorCode::RepositoryAccessDenied,
                format!("You need write access to {central_owner}/{central_repo} to open its pull request."),
            )
            .into());
        }
        if state.github.has_app() {
            state
                .github
                .get_repository_installation(&central_owner, &central_repo)
                .await
                .map_err(|e| e.to_user_error())?;
        }
        Some(format!("{central_owner}/{central_repo}"))
    } else {
        None
    };
    if !state.store.list_artifacts(job.id).await?.iter().any(|a| a.allowed) {
        return Err(AppError::conflict("This job produced nothing that can be published.").into());
    }
    match state
        .store
        .request_publish(
            user.user.id,
            job.id,
            DEFAULT_PUBLISH_ATTEMPTS,
            &json!({}),
            &PublishOptions { title, central_repo: central.as_deref(), reviewers: &reviewers, as_admin: !owner_view },
        )
        .await?
    {
        PublishOutcome::Queued(job) => {
            let mut details = json!({ "title": title, "publish_central": req.publish_central, "reviewers": reviewers });
            if !owner_view {
                details["on_behalf_of"] = json!(job.requested_by);
            }
            audit(&state, &user, actions::PUBLISH_REQUESTED, &job, details).await;
            Ok((StatusCode::ACCEPTED, Json(job_json_as(&state, &job, owner_view).await?)).into_response())
        }
        PublishOutcome::Unchanged(job) => {
            Ok((StatusCode::OK, Json(job_json_as(&state, &job, owner_view).await?)).into_response())
        }
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

/// `POST /api/bounded-context/jobs/{id}/retry` — a new job with the same inputs and generator (at the
/// generator repository's current head).
pub async fn retry(State(state): State<AppState>, user: CurrentUser, Path(id): Path<String>) -> ApiResult<Response> {
    let old = state.store.get_user_job(user.user.id, parse_job_id(&id)?).await?;
    if !matches!(old.status(), JobStatus::Failed | JobStatus::Cancelled) {
        return Err(AppError::conflict("Only failed or cancelled jobs can be retried.").into());
    }
    let generator_id = if old.generator_source == "builtin" {
        old.generator_id.clone().unwrap_or_else(|| BUILTIN_ID.to_owned())
    } else {
        old.generator_id
            .clone()
            .ok_or_else(|| AppError::conflict("The generator of this job is no longer available."))?
    };
    let ctx = {
        let raw: BTreeMap<String, String> = serde_json::from_value(old.inputs.clone()).unwrap_or_default();
        // Resolve first so inputs the current agent no longer declares can be dropped rather than failing the retry.
        let token = state.tokens.access_token(user.user.id).await?;
        let generator = generators::select(&state, &user, &token, &generator_id).await.map_err(|e| {
            if e.code == ErrorCode::NotFound {
                AppError::conflict("The generator of this job is no longer available.")
            } else {
                e
            }
        })?;
        let inputs: BTreeMap<String, String> =
            raw.into_iter().filter(|(k, _)| generator.bundle.inputs().iter().any(|i| &i.name == k)).collect();
        prepare_with(&state, &user, token, generator, &old.model, &inputs, old.batch_id).await?
    };
    let job = create_job_for(&state, &user, &ctx, (&old.owner, &old.repo, &old.base_branch), Some(old.id)).await?;
    Ok((StatusCode::ACCEPTED, Json(job_json(&state, &job).await?)).into_response())
}
