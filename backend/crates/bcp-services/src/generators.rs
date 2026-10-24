//! Fetches a generator (agent bundle) from a GitHub repository, pinned to a commit, and loads it under the
//! platform's hard limits. Shared by the API (validation, input forms) and the worker (the actual run).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bcp_agent::workspace::{extract_tarball, ExtractLimits};
use bcp_agent::AgentBundle;
use bcp_core::generators::GeneratorRepo;
use bcp_core::{AppError, ErrorCode};
use bcp_github::{GitHubClient, GitHubError};

/// Generators are small; anything bigger is almost certainly the wrong repository.
const MAX_GENERATOR_BYTES: u64 = 50 * 1024 * 1024;
const MAX_GENERATOR_FILES: u64 = 5_000;
const CACHE_TTL: Duration = Duration::from_secs(600);
const CACHE_MAX_ENTRIES: usize = 16;

/// A generator bundle extracted on disk. Keep the `Arc` alive for as long as the bundle's files are needed
/// (the directory is deleted when the last reference is dropped).
pub struct ResolvedGenerator {
    pub repo: GeneratorRepo,
    pub branch: String,
    pub sha: String,
    pub bundle: Arc<AgentBundle>,
    /// What the platform tightened (tools, instructions) — worth logging.
    pub notes: Vec<String>,
    _dir: tempfile::TempDir,
}

impl std::fmt::Debug for ResolvedGenerator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedGenerator")
            .field("repo", &self.repo.display())
            .field("branch", &self.branch)
            .field("sha", &self.sha)
            .finish_non_exhaustive()
    }
}

type CacheEntry = (Instant, Arc<ResolvedGenerator>);

#[derive(Clone)]
pub struct GeneratorResolver {
    github: GitHubClient,
    work_dir: PathBuf,
    cache: Arc<Mutex<HashMap<String, CacheEntry>>>,
}

impl GeneratorResolver {
    pub fn new(github: GitHubClient, work_dir: PathBuf) -> Self {
        Self { github, work_dir, cache: Arc::default() }
    }

    /// The branch (default branch when unspecified) and its head commit. The user needs read access.
    pub async fn resolve_revision(&self, token: &str, repo: &GeneratorRepo) -> Result<(String, String), AppError> {
        let gh = &self.github;
        let branch = match repo.branch.as_deref().filter(|b| !b.is_empty()) {
            Some(b) => b.to_owned(),
            None => {
                let r = gh.get_repository(token, &repo.owner, &repo.repo).await.map_err(|e| repo_error(repo, &e))?;
                r.default_branch
            }
        };
        let head = gh.get_branch(token, &repo.owner, &repo.repo, &branch).await.map_err(|e| match e {
            GitHubError::NotFound => AppError::new(
                ErrorCode::GeneratorInvalid,
                format!("Branch '{branch}' was not found in {}.", repo.display()),
            ),
            other => repo_error(repo, &other),
        })?;
        Ok((branch, head.commit.sha))
    }

    /// Downloads, validates and loads the bundle at `sha`. Results are cached briefly per commit.
    pub async fn load(
        &self,
        token: &str,
        repo: &GeneratorRepo,
        branch: &str,
        sha: &str,
    ) -> Result<Arc<ResolvedGenerator>, AppError> {
        let key = format!("{}@{sha}", repo.key());
        if let Some(hit) = self.cached(&key) {
            return Ok(hit);
        }
        std::fs::create_dir_all(&self.work_dir).map_err(|e| AppError::internal(format!("generator work dir: {e}")))?;
        let dir = tempfile::Builder::new()
            .prefix("generator-")
            .tempdir_in(&self.work_dir)
            .map_err(|e| AppError::internal(format!("generator temp dir: {e}")))?;
        let tgz = dir.path().join("generator.tar.gz");
        let root = dir.path().join("repo");
        self.github.download_tarball(token, &repo.owner, &repo.repo, sha, &tgz, MAX_GENERATOR_BYTES).await.map_err(
            |e| match e {
                GitHubError::TooLarge => AppError::new(
                    ErrorCode::GeneratorInvalid,
                    format!("{} is larger than a generator repository can be (50 MiB).", repo.display()),
                ),
                other => repo_error(repo, &other),
            },
        )?;
        let (tgz2, root2) = (tgz.clone(), root.clone());
        let limits = ExtractLimits { max_bytes: MAX_GENERATOR_BYTES, max_files: MAX_GENERATOR_FILES };
        tokio::task::spawn_blocking(move || extract_tarball(&tgz2, &root2, limits))
            .await
            .map_err(|e| AppError::internal(format!("extract task: {e}")))?
            .map_err(|e| {
                if e.code == ErrorCode::RepositoryTooLarge {
                    AppError::new(
                        ErrorCode::GeneratorInvalid,
                        format!("{} has too many or too large files.", repo.display()),
                    )
                } else {
                    e
                }
            })?;
        let _ = std::fs::remove_file(&tgz);

        let bundle_dir = if repo.path.is_empty() { root.clone() } else { root.join(&repo.path) };
        let manifest_hint =
            if repo.path.is_empty() { "bundle.yaml".to_owned() } else { format!("{}/bundle.yaml", repo.path) };
        if !bundle_dir.join(bcp_agent::bundle::MANIFEST_FILE).is_file() {
            return Err(AppError::new(
                ErrorCode::GeneratorInvalid,
                format!(
                    "{} has no {manifest_hint} at {branch}. A generator repository needs a bundle.yaml next to its agent files.",
                    repo.display()
                ),
            ));
        }
        let shown = dir.path().display().to_string();
        let loaded = tokio::task::spawn_blocking(move || {
            let mut bundle = AgentBundle::load(&bundle_dir)?;
            let notes = bundle.restrict_to_platform_policy().map_err(|e| anyhow::anyhow!(e.message))?;
            Ok::<_, anyhow::Error>((bundle, notes))
        })
        .await
        .map_err(|e| AppError::internal(format!("bundle load task: {e}")))?;
        let (bundle, notes) = loaded.map_err(|e| {
            let msg = format!("{e:#}").replace(&shown, "<generator>");
            AppError::new(ErrorCode::GeneratorInvalid, format!("The generator bundle is not valid: {msg}"))
        })?;
        let resolved = Arc::new(ResolvedGenerator {
            repo: repo.clone(),
            branch: branch.to_owned(),
            sha: sha.to_owned(),
            bundle: Arc::new(bundle),
            notes,
            _dir: dir,
        });
        self.remember(key, resolved.clone());
        Ok(resolved)
    }

    fn cached(&self, key: &str) -> Option<Arc<ResolvedGenerator>> {
        let map = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        map.get(key).filter(|(at, _)| at.elapsed() < CACHE_TTL).map(|(_, v)| v.clone())
    }

    fn remember(&self, key: String, value: Arc<ResolvedGenerator>) {
        let mut map = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
        while map.len() >= CACHE_MAX_ENTRIES {
            let Some(oldest) = map.iter().min_by_key(|(_, (at, _))| *at).map(|(k, _)| k.clone()) else { break };
            map.remove(&oldest);
        }
        map.insert(key, (Instant::now(), value));
    }
}

/// GitHub errors about the *generator* repository, phrased for the user.
fn repo_error(repo: &GeneratorRepo, e: &GitHubError) -> AppError {
    match e {
        GitHubError::NotFound | GitHubError::Forbidden { .. } => AppError::new(
            ErrorCode::RepositoryAccessDenied,
            format!(
                "The generator repository {} was not found or you cannot read it. In GitHub App mode the portal's \
                 App must also be installed on it.",
                repo.display()
            ),
        ),
        other => other.to_user_error(),
    }
}
