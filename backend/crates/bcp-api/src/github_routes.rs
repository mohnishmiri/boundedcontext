//! Repository browsing on behalf of the signed-in user. Everything uses the user's token, so results are the
//! intersection of what the user may access and where the portal's GitHub App is installed.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use bcp_core::{validate, AppError, ErrorCode};
use bcp_github::{GhBranch, GhInstallation, GhRepo, GitHubError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::error::{ApiQuery, ApiResult};
use crate::state::AppState;

const LIST_TTL: Duration = Duration::from_secs(60);
const MAX_LIST_PAGES: usize = 10;

#[derive(Debug, Clone, Serialize)]
pub struct RepositoryView {
    pub id: i64,
    pub owner: String,
    pub name: String,
    pub full_name: String,
    pub private: bool,
    pub default_branch: String,
    pub html_url: String,
    pub description: Option<String>,
    pub permissions: Value,
    pub updated_at: Option<String>,
}

impl From<&GhRepo> for RepositoryView {
    fn from(r: &GhRepo) -> Self {
        let p = r.permissions();
        Self {
            id: r.id,
            owner: r.owner.login.clone(),
            name: r.name.clone(),
            full_name: r.full_name.clone(),
            private: r.private,
            default_branch: r.default_branch.clone(),
            html_url: r.html_url.clone(),
            description: r.description.clone(),
            permissions: json!({ "pull": p.pull, "push": p.push, "admin": p.admin }),
            updated_at: r.updated_at.clone(),
        }
    }
}

fn page_params(page: Option<u32>, per_page: Option<u32>, default: u32) -> Result<(u32, u32), AppError> {
    let page = page.unwrap_or(1);
    let per_page = per_page.unwrap_or(default);
    if page == 0 || page > 10_000 {
        return Err(AppError::validation([("page", "Must be between 1 and 10000")]));
    }
    if per_page == 0 || per_page > 100 {
        return Err(AppError::validation([("per_page", "Must be between 1 and 100")]));
    }
    Ok((page, per_page))
}

fn paginate<T: Clone>(items: &[T], page: u32, per_page: u32) -> (Vec<T>, bool) {
    let start = ((page - 1) * per_page) as usize;
    let slice: Vec<T> = items.iter().skip(start).take(per_page as usize).cloned().collect();
    (slice, start + (per_page as usize) < items.len())
}

fn normalise_query(q: Option<String>) -> Result<Option<String>, AppError> {
    match q.map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()) {
        Some(s) if s.chars().count() > 100 || s.chars().any(char::is_control) => {
            Err(AppError::validation([("q", "Must be at most 100 printable characters")]))
        }
        other => Ok(other),
    }
}

pub(crate) async fn installations(
    state: &AppState,
    user_id: Uuid,
    token: &str,
) -> Result<Arc<Vec<GhInstallation>>, AppError> {
    let key = format!("inst:{user_id}");
    if let Some(v) = state.cache.get::<Vec<GhInstallation>>(&key, LIST_TTL) {
        return Ok(v);
    }
    let list = Arc::new(state.github.list_user_installations(token).await.map_err(|e| e.to_user_error())?);
    state.cache.put(key, list.clone());
    Ok(list)
}

async fn user_repos(state: &AppState, user_id: Uuid, token: &str) -> Result<Arc<Vec<GhRepo>>, AppError> {
    let key = format!("userrepos:{user_id}");
    if let Some(v) = state.cache.get::<Vec<GhRepo>>(&key, LIST_TTL) {
        return Ok(v);
    }
    let (mut list, truncated) =
        state.github.list_user_repos(token, MAX_LIST_PAGES).await.map_err(|e| e.to_user_error())?;
    if truncated {
        tracing::warn!(%user_id, "user repository listing truncated at {} entries", list.len());
    }
    list.retain(|r| !r.archived);
    list.sort_by_key(|r| r.name.to_ascii_lowercase());
    let list = Arc::new(list);
    state.cache.put(key, list.clone());
    Ok(list)
}

/// `GET /api/github/organizations`
pub async fn organizations(State(state): State<AppState>, user: CurrentUser) -> ApiResult<Json<Value>> {
    let token = state.tokens.access_token(user.user.id).await?;
    if !state.github.has_app() {
        let mut seen = std::collections::BTreeMap::<String, Value>::new();
        seen.insert(
            user.user.login.to_ascii_lowercase(),
            json!({
                "installation_id": user.user.github_id.max(1),
                "login": user.user.login,
                "type": "User",
                "avatar_url": user.user.avatar_url,
            }),
        );
        if let Ok(orgs) = state.github.list_user_orgs(&token).await {
            for org in orgs {
                seen.entry(org.login.to_ascii_lowercase()).or_insert_with(|| {
                    json!({
                        "installation_id": org.id,
                        "login": org.login,
                        "type": "Organization",
                        "avatar_url": org.avatar_url,
                    })
                });
            }
        }
        if let Ok(repos) = user_repos(&state, user.user.id, &token).await {
            for (idx, r) in repos.iter().enumerate() {
                let key = r.owner.login.to_ascii_lowercase();
                seen.entry(key).or_insert_with(|| {
                    let kind = if !r.owner.kind.is_empty() {
                        r.owner.kind.as_str()
                    } else if r.owner.login.eq_ignore_ascii_case(&user.user.login) {
                        "User"
                    } else {
                        "Organization"
                    };
                    let id = if r.owner.id > 0 { r.owner.id } else { 1_000_000 + idx as i64 };
                    json!({
                        "installation_id": id,
                        "login": r.owner.login,
                        "type": kind,
                        "avatar_url": r.owner.avatar_url,
                    })
                });
            }
        }
        let mut items: Vec<Value> = seen.into_values().collect();
        items.sort_by_key(|v| v["login"].as_str().unwrap_or_default().to_ascii_lowercase());
        return Ok(Json(json!({ "items": items, "install_url": Value::Null })));
    }

    let list = installations(&state, user.user.id, &token).await?;
    let mut items: Vec<Value> = list
        .iter()
        .map(|i| json!({ "installation_id": i.id, "login": i.account.login, "type": i.account.kind, "avatar_url": i.account.avatar_url }))
        .collect();
    items.sort_by_key(|v| v["login"].as_str().unwrap_or_default().to_ascii_lowercase());
    Ok(Json(json!({ "items": items, "install_url": state.github.install_url(&state.cfg.common.github.app_slug) })))
}

#[derive(Deserialize)]
pub struct RepoListQuery {
    owner: String,
    q: Option<String>,
    page: Option<u32>,
    per_page: Option<u32>,
}

/// `GET /api/github/repositories?owner=&q=&page=&per_page=`
pub async fn repositories(
    State(state): State<AppState>,
    user: CurrentUser,
    ApiQuery(q): ApiQuery<RepoListQuery>,
) -> ApiResult<Json<Value>> {
    validate::owner(&q.owner).map_err(|e| AppError::validation([("owner", e)]))?;
    let (page, per_page) = page_params(q.page, q.per_page, 30)?;
    let filter = normalise_query(q.q)?;
    let token = state.tokens.access_token(user.user.id).await?;
    if !state.github.has_app() {
        let all_repos = user_repos(&state, user.user.id, &token).await?;
        let filtered: Vec<&GhRepo> = all_repos
            .iter()
            .filter(|r| r.owner.login.eq_ignore_ascii_case(&q.owner))
            .filter(|r| filter.as_ref().is_none_or(|f| r.name.to_ascii_lowercase().contains(f)))
            .collect();
        let (items, has_more) = paginate(&filtered, page, per_page);
        let items: Vec<RepositoryView> = items.into_iter().map(RepositoryView::from).collect();
        return Ok(Json(
            json!({ "items": items, "page": page, "per_page": per_page, "has_more": has_more, "total": filtered.len() }),
        ));
    }
    let inst = installations(&state, user.user.id, &token)
        .await?
        .iter()
        .find(|i| i.account.login.eq_ignore_ascii_case(&q.owner))
        .cloned()
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::AppNotInstalled,
                format!("The portal's GitHub App is not installed on '{}', or you have no access there.", q.owner),
            )
            .with_details(json!({ "install_url": state.github.install_url(&state.cfg.common.github.app_slug) }))
        })?;
    let key = format!("repos:{}:{}", user.user.id, inst.id);
    let repos = match state.cache.get::<Vec<GhRepo>>(&key, LIST_TTL) {
        Some(r) => r,
        None => {
            let (mut list, truncated) = state
                .github
                .list_installation_repositories(&token, inst.id, MAX_LIST_PAGES)
                .await
                .map_err(|e| e.to_user_error())?;
            if truncated {
                tracing::warn!(installation = inst.id, "repository listing truncated at {} entries", list.len());
            }
            list.retain(|r| !r.archived);
            list.sort_by_key(|r| r.name.to_ascii_lowercase());
            let list = Arc::new(list);
            state.cache.put(key, list.clone());
            list
        }
    };
    let filtered: Vec<&GhRepo> =
        repos.iter().filter(|r| filter.as_ref().is_none_or(|f| r.name.to_ascii_lowercase().contains(f))).collect();
    let (items, has_more) = paginate(&filtered, page, per_page);
    let items: Vec<RepositoryView> = items.into_iter().map(RepositoryView::from).collect();
    Ok(Json(
        json!({ "items": items, "page": page, "per_page": per_page, "has_more": has_more, "total": filtered.len() }),
    ))
}

pub(crate) fn validate_repo_path(owner: &str, repo: &str) -> Result<(), AppError> {
    let mut errors = Vec::new();
    if let Err(e) = validate::owner(owner) {
        errors.push(("owner", e));
    }
    if let Err(e) = validate::repo(repo) {
        errors.push(("repo", e));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::validation(errors))
    }
}

/// The App installation covering a repository, or `None` when the App is not installed there.
pub(crate) async fn repo_installation(state: &AppState, owner: &str, repo: &str) -> Result<Option<i64>, AppError> {
    let key = format!("repoinst:{}/{}", owner.to_ascii_lowercase(), repo.to_ascii_lowercase());
    if let Some(v) = state.cache.get::<Option<i64>>(&key, LIST_TTL) {
        return Ok(*v);
    }
    let found = match state.github.get_repository_installation(owner, repo).await {
        Ok(i) => Some(i.id),
        Err(GitHubError::NotFound) => None,
        Err(e) => return Err(e.to_service_error()),
    };
    state.cache.put(key, Arc::new(found));
    Ok(found)
}

/// `GET /api/github/repositories/{owner}/{repo}`
pub async fn repository(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((owner, repo)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    validate_repo_path(&owner, &repo)?;
    let token = state.tokens.access_token(user.user.id).await?;
    let r = state.github.get_repository(&token, &owner, &repo).await.map_err(|e| e.to_user_error())?;
    let installed =
        if state.github.has_app() { repo_installation(&state, &owner, &repo).await?.is_some() } else { true };
    let p = r.permissions();
    let can_generate = installed && p.pull && !r.archived;
    let can_publish = can_generate && p.push;
    let reason = if !installed {
        Some("The portal's GitHub App is not installed on this repository. Ask an organisation owner to install it.")
    } else if r.archived {
        Some("This repository is archived.")
    } else if !p.pull {
        Some("You do not have read access to this repository.")
    } else if !p.push {
        Some("You have read-only access: you can generate documentation but cannot open a pull request.")
    } else {
        None
    };
    let mut v = serde_json::to_value(RepositoryView::from(&r)).expect("serialisable");
    v["can_generate"] = json!(can_generate);
    v["can_publish"] = json!(can_publish);
    v["reason"] = json!(reason);
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct BranchQuery {
    q: Option<String>,
    page: Option<u32>,
    per_page: Option<u32>,
}

/// `GET /api/github/repositories/{owner}/{repo}/branches`
pub async fn branches(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((owner, repo)): Path<(String, String)>,
    ApiQuery(q): ApiQuery<BranchQuery>,
) -> ApiResult<Json<Value>> {
    validate_repo_path(&owner, &repo)?;
    let (page, per_page) = page_params(q.page, q.per_page, 50)?;
    let filter = normalise_query(q.q)?;
    let token = state.tokens.access_token(user.user.id).await?;
    let key = format!("branches:{}:{}/{}", user.user.id, owner.to_ascii_lowercase(), repo.to_ascii_lowercase());
    let cached = state.cache.get::<(String, Vec<GhBranch>)>(&key, Duration::from_secs(30));
    let data = match cached {
        Some(d) => d,
        None => {
            let r = state.github.get_repository(&token, &owner, &repo).await.map_err(|e| e.to_user_error())?;
            let (mut list, _) = state
                .github
                .list_branches(&token, &owner, &repo, MAX_LIST_PAGES)
                .await
                .map_err(|e| e.to_user_error())?;
            // Default branch first, then alphabetical.
            list.sort_by(|a, b| {
                (a.name != r.default_branch).cmp(&(b.name != r.default_branch)).then_with(|| a.name.cmp(&b.name))
            });
            let d = Arc::new((r.default_branch, list));
            state.cache.put(key, d.clone());
            d
        }
    };
    let (default_branch, list) = (&data.0, &data.1);
    let filtered: Vec<&GhBranch> =
        list.iter().filter(|b| filter.as_ref().is_none_or(|f| b.name.to_ascii_lowercase().contains(f))).collect();
    let (items, has_more) = paginate(&filtered, page, per_page);
    let items: Vec<Value> = items
        .into_iter()
        .map(|b| json!({ "name": b.name, "commit_sha": b.commit.sha, "protected": b.protected }))
        .collect();
    Ok(Json(json!({
        "items": items, "page": page, "per_page": per_page, "has_more": has_more, "total": filtered.len(),
        "default_branch": default_branch,
    })))
}

/// `GET /api/agent`
pub async fn agent(State(state): State<AppState>, _user: CurrentUser) -> Json<Value> {
    let a = &state.agent;
    Json(json!({
        "id": a.id, "name": a.name, "description": a.description, "version": a.version,
        "placeholder": a.placeholder, "output_paths": a.output_paths, "inputs": a.inputs,
    }))
}
