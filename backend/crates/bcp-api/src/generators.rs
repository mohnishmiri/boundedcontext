//! `/api/generators` — the agent bundles ("generators") users can run. The built-in one ships with the worker
//! image; curated ones come from `config/generators.yaml`; users can add repositories the operator allows.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use bcp_agent::{AgentAdapter, AgentBundle, CopilotCliAdapter};
use bcp_core::generators::{GeneratorRepo, BUILTIN_ID};
use bcp_core::AppError;
use bcp_services::generators::ResolvedGenerator;
use bcp_store::{JobGenerator, NewUserGenerator, UserGeneratorRow};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::error::{ApiJson, ApiResult};
use crate::state::AppState;

/// A generator chosen for a request: what to record on the job, plus its loaded bundle.
pub struct SelectedGenerator {
    pub job: JobGenerator,
    pub bundle: Arc<AgentBundle>,
    /// Keeps a repository generator's files on disk while the bundle is used.
    pub resolved: Option<Arc<ResolvedGenerator>>,
}

fn view(
    id: &str,
    name: &str,
    description: Option<&str>,
    source: &str,
    repo: Option<&GeneratorRepo>,
    removable: bool,
) -> Value {
    json!({
        "id": id, "name": name, "description": description, "source": source, "removable": removable,
        "repository": repo.map(|r| json!({ "owner": r.owner, "repo": r.repo, "branch": r.branch, "path": r.path })),
    })
}

fn builtin_view(state: &AppState) -> Value {
    let (name, description) = builtin_labels(state);
    view(BUILTIN_ID, &name, Some(&description), "builtin", None, false)
}

fn builtin_labels(state: &AppState) -> (String, String) {
    (
        state.registry.builtin_name.clone().unwrap_or_else(|| state.bundle.display_name().to_owned()),
        state.registry.builtin_description.clone().unwrap_or_else(|| state.bundle.description().to_owned()),
    )
}

fn user_repo(row: &UserGeneratorRow) -> GeneratorRepo {
    GeneratorRepo {
        owner: row.owner.clone(),
        repo: row.repo.clone(),
        branch: row.branch.clone(),
        path: row.path.clone(),
    }
}

fn user_view(row: &UserGeneratorRow) -> Value {
    view(&row.id.to_string(), &row.name, None, "user", Some(&user_repo(row)), true)
}

/// `GET /api/generators`
pub async fn list(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Value>> {
    let mut items = vec![builtin_view(&state)];
    for c in state.registry.curated() {
        items.push(view(&c.id, &c.name, c.description.as_deref(), "curated", Some(&c.repository), false));
    }
    for row in state.store.list_user_generators(user.user.id).await? {
        items.push(user_view(&row));
    }
    Ok(Json(json!({
        "items": items,
        "default_id": BUILTIN_ID,
        "can_add": state.registry.user_additions_enabled(),
    })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddGenerator {
    pub owner: String,
    pub repo: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// `POST /api/generators` — validates the repository against the allowlist and by actually loading its bundle,
/// so a saved generator is known to work.
pub async fn add(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiJson(req): ApiJson<AddGenerator>,
) -> ApiResult<Response> {
    let repo = GeneratorRepo {
        owner: req.owner.trim().to_owned(),
        repo: req.repo.trim().to_owned(),
        branch: req.branch.map(|b| b.trim().to_owned()).filter(|b| !b.is_empty()),
        path: req.path.unwrap_or_default().trim().to_owned(),
    }
    .normalised();
    repo.validate()?;
    let name = req.name.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());
    if let Some(n) = &name {
        if n.chars().count() > 80 || n.chars().any(char::is_control) {
            return Err(AppError::validation([("name", "Must be at most 80 printable characters")]).into());
        }
    }
    state.registry.ensure_allowed(&repo)?;
    let count = state.store.count_user_generators(user.user.id).await?;
    if count >= state.registry.max_user_generators as i64 {
        return Err(AppError::conflict(format!(
            "You can save at most {} generators. Remove one first.",
            state.registry.max_user_generators
        ))
        .into());
    }
    let token = state.tokens.access_token(user.user.id).await?;
    let (branch, sha) = state.resolver.resolve_revision(&token, &repo).await?;
    let resolved = state.resolver.load(&token, &repo, &branch, &sha).await?;
    let name = name.unwrap_or_else(|| resolved.bundle.display_name().to_owned());
    let row = state
        .store
        .add_user_generator(NewUserGenerator {
            user_id: user.user.id,
            name: &name,
            owner: &repo.owner,
            repo: &repo.repo,
            branch: repo.branch.as_deref(),
            path: &repo.path,
        })
        .await?;
    Ok((StatusCode::CREATED, Json(user_view(&row))).into_response())
}

/// `DELETE /api/generators/{id}`
pub async fn remove(State(state): State<AppState>, user: CurrentUser, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let id = Uuid::parse_str(&id).map_err(|_| AppError::not_found("Generator"))?;
    state.store.delete_user_generator(user.user.id, id).await.map_err(|e| match e {
        bcp_store::StoreError::NotFound => AppError::not_found("Generator"),
        other => other.into(),
    })?;
    Ok(StatusCode::NO_CONTENT)
}

/// Resolves a generator id (`builtin`, a curated id or a saved generator's UUID) to a loaded bundle pinned to
/// the repository's current head. Allowlist and access are re-checked every time.
pub async fn select(
    state: &AppState,
    user: &CurrentUser,
    token: &str,
    id: &str,
) -> Result<SelectedGenerator, AppError> {
    if id == BUILTIN_ID {
        let (name, _) = builtin_labels(state);
        return Ok(SelectedGenerator {
            job: JobGenerator::builtin(name),
            bundle: state.bundle.clone(),
            resolved: None,
        });
    }
    let (source, name, repo) = if let Some(c) = state.registry.curated_by_id(id) {
        ("curated", c.name.clone(), c.repository.clone())
    } else {
        let uuid = Uuid::parse_str(id).map_err(|_| AppError::not_found("Generator"))?;
        let row = state.store.get_user_generator(user.user.id, uuid).await.map_err(|e| match e {
            bcp_store::StoreError::NotFound => AppError::not_found("Generator"),
            other => other.into(),
        })?;
        ("user", row.name.clone(), user_repo(&row))
    };
    state.registry.ensure_allowed(&repo)?;
    let (branch, sha) = state.resolver.resolve_revision(token, &repo).await?;
    let resolved = state.resolver.load(token, &repo, &branch, &sha).await?;
    for note in &resolved.notes {
        tracing::info!(generator = %repo.display(), "{note}");
    }
    Ok(SelectedGenerator {
        job: JobGenerator {
            source: source.to_owned(),
            id: Some(id.to_owned()),
            name,
            owner: Some(repo.owner.clone()),
            repo: Some(repo.repo.clone()),
            branch: Some(branch),
            path: Some(repo.path.clone()),
            sha: Some(sha),
            key: repo.key(),
        },
        bundle: resolved.bundle.clone(),
        resolved: Some(resolved),
    })
}

/// What the New Job form needs to know about a generator: its inputs, output paths and any pinned model.
pub fn agent_json(selected: &SelectedGenerator) -> Value {
    let d = CopilotCliAdapter::new(selected.bundle.clone(), "copilot").describe();
    let locked = d.locked_model.clone();
    json!({
        "id": d.id, "name": selected.job.name, "description": d.description, "version": d.version,
        "placeholder": d.placeholder, "output_paths": d.output_paths, "inputs": d.inputs,
        "locked_model": locked,
        "lock_reason": locked.as_ref().map(|m| format!(
            "This generator's agent pins the model '{m}' in its definition, so GitHub Copilot always uses it."
        )),
        "resolved": { "branch": selected.job.branch, "sha": selected.job.sha },
    })
}

/// `GET /api/generators/{id}/agent`
pub async fn agent_info(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    if id == BUILTIN_ID {
        let selected = select(&state, &user, "", &id).await?;
        return Ok(Json(agent_json(&selected)));
    }
    let token = state.tokens.access_token(user.user.id).await?;
    let selected = select(&state, &user, &token, &id).await?;
    Ok(Json(agent_json(&selected)))
}
