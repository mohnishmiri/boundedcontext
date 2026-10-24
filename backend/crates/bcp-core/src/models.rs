//! Operator-maintained model catalog (config/models.yaml). The catalog is the server-side source of truth
//! for which model identifiers may be requested; GitHub Copilot still enforces per-user entitlement at run
//! time.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, ErrorCode};
use crate::validate;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    pub vendor: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub multiplier: Option<f64>,
    #[serde(default)]
    pub verified: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogFile {
    default: String,
    models: Vec<ModelEntry>,
}

#[derive(Debug, Clone)]
pub struct ModelCatalog {
    default: String,
    models: Vec<ModelEntry>,
    show_unverified: bool,
}

impl ModelCatalog {
    pub fn load(path: &Path, show_unverified: bool) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading model catalog {}", path.display()))?;
        Self::parse(&text, show_unverified)
    }

    pub fn parse(yaml: &str, show_unverified: bool) -> Result<Self> {
        let file: CatalogFile = serde_yaml::from_str(yaml).context("parsing model catalog")?;
        let mut seen = std::collections::HashSet::new();
        for m in &file.models {
            if let Err(e) = validate::model_id(&m.id) {
                bail!("model catalog: invalid id {:?}: {e}", m.id);
            }
            if !seen.insert(m.id.clone()) {
                bail!("model catalog: duplicate id {}", m.id);
            }
        }
        let catalog = Self { default: file.default, models: file.models, show_unverified };
        if catalog.get(&catalog.default).is_none() {
            bail!("model catalog: default {:?} is not a visible model", catalog.default);
        }
        Ok(catalog)
    }

    /// Models users may choose from.
    pub fn visible(&self) -> impl Iterator<Item = &ModelEntry> {
        self.models.iter().filter(|m| m.verified || self.show_unverified)
    }

    pub fn default_model(&self) -> &str {
        &self.default
    }

    pub fn get(&self, id: &str) -> Option<&ModelEntry> {
        self.visible().find(|m| m.id == id)
    }

    /// Server-side validation of a requested model.
    pub fn validate(&self, id: &str) -> Result<&ModelEntry, AppError> {
        if validate::model_id(id).is_err() {
            return Err(AppError::validation([("model", "Not a valid model identifier")]));
        }
        self.get(id).ok_or_else(|| {
            AppError::new(
                ErrorCode::ModelUnavailable,
                format!("Model '{id}' is not available in this portal. Choose one of the listed models."),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"
default: auto
models:
  - { id: auto, name: Auto, vendor: GitHub, verified: true }
  - { id: claude-haiku-4.5, name: Claude Haiku 4.5, vendor: Anthropic, verified: true }
  - { id: secret-model, name: Hidden, vendor: X, verified: false }
"#;

    #[test]
    fn hides_unverified_by_default() {
        let c = ModelCatalog::parse(YAML, false).unwrap();
        assert_eq!(c.visible().count(), 2);
        assert!(c.validate("claude-haiku-4.5").is_ok());
        assert_eq!(c.validate("secret-model").unwrap_err().code, ErrorCode::ModelUnavailable);
        let c = ModelCatalog::parse(YAML, true).unwrap();
        assert!(c.validate("secret-model").is_ok());
    }

    #[test]
    fn rejects_malformed_ids_as_validation_errors() {
        let c = ModelCatalog::parse(YAML, false).unwrap();
        assert_eq!(c.validate("gpt; rm -rf /").unwrap_err().code, ErrorCode::ValidationFailed);
        assert_eq!(c.validate("unknown-model").unwrap_err().code, ErrorCode::ModelUnavailable);
    }

    #[test]
    fn default_must_be_visible_and_ids_unique() {
        assert!(
            ModelCatalog::parse("default: x\nmodels: [{id: y, name: Y, vendor: V, verified: true}]", false).is_err()
        );
        assert!(ModelCatalog::parse(
            "default: y\nmodels: [{id: y, name: Y, vendor: V, verified: true}, {id: y, name: Y, vendor: V, verified: true}]",
            false
        )
        .is_err());
    }

    #[test]
    fn repository_catalog_parses() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../config/models.yaml");
        if path.exists() {
            let c = ModelCatalog::load(&path, false).unwrap();
            assert_eq!(c.default_model(), "auto");
        }
    }
}
