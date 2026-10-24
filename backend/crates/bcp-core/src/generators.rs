//! Generator registry: which agent bundles ("generators") users may run. The operator maintains
//! `config/generators.yaml`:
//!
//! * the **built-in** generator is the bundle baked into the worker image (always available);
//! * **curated** generators are repositories the operator lists explicitly;
//! * users may add their own generator repositories, but only ones matching the operator's `allow` patterns.
//!
//! A generator repository holds a bundle (`bundle.yaml` + `agents/`) at `path` (default: the repository root).

use std::path::Path;
use std::sync::LazyLock;

use anyhow::{bail, ensure, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, ErrorCode};
use crate::validate;

pub const BUILTIN_ID: &str = "builtin";
const DEFAULT_MAX_USER_GENERATORS: usize = 20;

static CURATED_ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9-]{0,39}$").unwrap());

/// Where a generator's bundle lives in a GitHub repository.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct GeneratorRepo {
    pub owner: String,
    pub repo: String,
    /// `None` = the repository's default branch.
    #[serde(default)]
    pub branch: Option<String>,
    /// Directory holding `bundle.yaml`; empty = repository root.
    #[serde(default)]
    pub path: String,
}

impl GeneratorRepo {
    /// Validates every field the same way for operator and user supplied values.
    pub fn validate(&self) -> Result<(), AppError> {
        let mut errors: Vec<(&str, String)> = Vec::new();
        if let Err(e) = validate::owner(&self.owner) {
            errors.push(("owner", e.into()));
        }
        if let Err(e) = validate::repo(&self.repo) {
            errors.push(("repo", e.into()));
        }
        if let Some(b) = &self.branch {
            if let Err(e) = validate::branch(b) {
                errors.push(("branch", e.into()));
            }
        }
        if !self.path.is_empty() {
            if let Err(e) = validate::relative_path(&self.path) {
                errors.push(("path", e.into()));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(AppError::validation(errors))
        }
    }

    /// Normalises `path` (no leading/trailing slash).
    pub fn normalised(mut self) -> Self {
        self.path = self.path.trim_matches('/').to_owned();
        if self.branch.as_deref() == Some("") {
            self.branch = None;
        }
        self
    }

    /// Duplicate-prevention identity (branch is excluded: the job pins the revision).
    pub fn key(&self) -> String {
        format!("{}/{}#{}", self.owner.to_ascii_lowercase(), self.repo.to_ascii_lowercase(), self.path)
    }

    pub fn display(&self) -> String {
        if self.path.is_empty() {
            format!("{}/{}", self.owner, self.repo)
        } else {
            format!("{}/{}/{}", self.owner, self.repo, self.path)
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CuratedGenerator {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(flatten)]
    pub repository: GeneratorRepo,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct BuiltinOverride {
    name: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryFile {
    #[serde(default)]
    builtin: BuiltinOverride,
    #[serde(default)]
    curated: Vec<CuratedGenerator>,
    /// `owner/repo` or `owner/*` (case-insensitive) patterns users may add as generator repositories.
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default)]
    max_user_generators: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct GeneratorRegistry {
    pub builtin_name: Option<String>,
    pub builtin_description: Option<String>,
    curated: Vec<CuratedGenerator>,
    allow: Vec<(String, Option<String>)>,
    pub max_user_generators: usize,
}

impl Default for GeneratorRegistry {
    /// Built-in generator only; users cannot add repositories.
    fn default() -> Self {
        Self {
            builtin_name: None,
            builtin_description: None,
            curated: Vec::new(),
            allow: Vec::new(),
            max_user_generators: DEFAULT_MAX_USER_GENERATORS,
        }
    }
}

impl GeneratorRegistry {
    /// Loads the registry; a missing file means "built-in only".
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::warn!(path = %path.display(), "generator registry not found; only the built-in generator is available");
                Ok(Self::default())
            }
            Err(e) => Err(e).with_context(|| format!("reading generator registry {}", path.display())),
        }
    }

    pub fn parse(yaml: &str) -> Result<Self> {
        let file: RegistryFile = if yaml.trim().is_empty() {
            RegistryFile::default()
        } else {
            serde_yaml::from_str(yaml).context("parsing generator registry")?
        };
        let mut seen = std::collections::HashSet::new();
        let mut curated = Vec::new();
        for mut c in file.curated {
            ensure!(CURATED_ID_RE.is_match(&c.id), "generator registry: invalid id {:?}", c.id);
            ensure!(c.id != BUILTIN_ID, "generator registry: id {BUILTIN_ID:?} is reserved");
            ensure!(seen.insert(c.id.clone()), "generator registry: duplicate id {}", c.id);
            c.repository = c.repository.normalised();
            if let Err(e) = c.repository.validate() {
                bail!("generator registry: {}: {}", c.id, e.message);
            }
            curated.push(c);
        }
        let mut allow = Vec::new();
        for pattern in file.allow {
            let (owner, repo) = pattern.split_once('/').with_context(|| {
                format!("generator registry: allow pattern {pattern:?} must be owner/repo or owner/*")
            })?;
            ensure!(
                validate::owner(owner).is_ok() && (repo == "*" || validate::repo(repo).is_ok()),
                "generator registry: invalid allow pattern {pattern:?}"
            );
            allow.push((owner.to_ascii_lowercase(), (repo != "*").then(|| repo.to_ascii_lowercase())));
        }
        Ok(Self {
            builtin_name: file.builtin.name,
            builtin_description: file.builtin.description,
            curated,
            allow,
            max_user_generators: file.max_user_generators.unwrap_or(DEFAULT_MAX_USER_GENERATORS),
        })
    }

    pub fn curated(&self) -> &[CuratedGenerator] {
        &self.curated
    }

    pub fn curated_by_id(&self, id: &str) -> Option<&CuratedGenerator> {
        self.curated.iter().find(|c| c.id == id)
    }

    /// Whether a repository may be used as a generator: curated entries and `allow` patterns.
    pub fn is_allowed(&self, repo: &GeneratorRepo) -> bool {
        let (owner, name) = (repo.owner.to_ascii_lowercase(), repo.repo.to_ascii_lowercase());
        self.curated
            .iter()
            .any(|c| c.repository.owner.eq_ignore_ascii_case(&owner) && c.repository.repo.eq_ignore_ascii_case(&name))
            || self.allow.iter().any(|(o, r)| *o == owner && r.as_ref().is_none_or(|r| *r == name))
    }

    pub fn user_additions_enabled(&self) -> bool {
        !self.allow.is_empty()
    }

    pub fn ensure_allowed(&self, repo: &GeneratorRepo) -> Result<(), AppError> {
        if self.is_allowed(repo) {
            return Ok(());
        }
        Err(AppError::new(
            ErrorCode::GeneratorNotAllowed,
            format!(
                "{} is not on this portal's list of allowed generator repositories. Ask an administrator to allow it.",
                repo.display()
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"
builtin: { name: Standard generator }
curated:
  - { id: aifc, name: AIFC, owner: mohnishmiri, repo: bounded-context-generator, path: aifc-archetype-generator/ }
allow:
  - acme/*
  - Other/Generator
max_user_generators: 3
"#;

    fn repo(owner: &str, name: &str) -> GeneratorRepo {
        GeneratorRepo { owner: owner.into(), repo: name.into(), branch: None, path: String::new() }
    }

    #[test]
    fn allowlist_matches_owner_wildcards_and_exact_repos() {
        let r = GeneratorRegistry::parse(YAML).unwrap();
        assert!(r.is_allowed(&repo("ACME", "anything")));
        assert!(r.is_allowed(&repo("other", "generator")));
        assert!(!r.is_allowed(&repo("other", "different")));
        assert!(!r.is_allowed(&repo("evil", "gen")));
        // Curated repositories are implicitly allowed.
        assert!(r.is_allowed(&repo("mohnishmiri", "bounded-context-generator")));
        assert_eq!(r.ensure_allowed(&repo("evil", "gen")).unwrap_err().code, ErrorCode::GeneratorNotAllowed);
        assert_eq!(r.max_user_generators, 3);
        assert_eq!(r.curated()[0].repository.path, "aifc-archetype-generator");
        assert_eq!(r.builtin_name.as_deref(), Some("Standard generator"));
    }

    #[test]
    fn default_registry_allows_nothing_extra() {
        let r = GeneratorRegistry::parse("").unwrap();
        assert!(!r.user_additions_enabled());
        assert!(!r.is_allowed(&repo("acme", "x")));
    }

    #[test]
    fn rejects_bad_registries() {
        for bad in [
            "curated: [{ id: builtin, name: X, owner: a, repo: b }]",
            "curated: [{ id: Bad_Id, name: X, owner: a, repo: b }]",
            "curated: [{ id: a, name: X, owner: a, repo: b }, { id: a, name: Y, owner: a, repo: c }]",
            "curated: [{ id: a, name: X, owner: a, repo: b, path: ../x }]",
            "allow: [not-a-pattern]",
            "allow: ['bad owner/*']",
            "surprise: true",
        ] {
            assert!(GeneratorRegistry::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn repo_validation_and_keys() {
        let ok =
            GeneratorRepo { owner: "Acme".into(), repo: "Gen".into(), branch: Some("main".into()), path: "a/b".into() };
        assert!(ok.validate().is_ok());
        assert_eq!(ok.key(), "acme/gen#a/b");
        assert_eq!(ok.display(), "Acme/Gen/a/b");
        let bad =
            GeneratorRepo { owner: "-x".into(), repo: "..".into(), branch: Some("a..b".into()), path: "../x".into() };
        let err = bad.validate().unwrap_err();
        assert_eq!(err.code, ErrorCode::ValidationFailed);
    }

    #[test]
    fn repository_registry_parses() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../config/generators.yaml");
        if path.exists() {
            GeneratorRegistry::load(&path).unwrap();
        }
    }
}
