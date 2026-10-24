//! The agent bundle: the real agent files (verbatim) plus `bundle.yaml`, which records the facts the
//! platform cannot infer — prompt, output location, inputs and tool policy. See agent-bundle/README.md.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{bail, ensure, Context, Result};
use bcp_core::{validate, AppError};
use globset::{Glob, GlobSet, GlobSetBuilder};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::template::Template;

pub const MANIFEST_FILE: &str = "bundle.yaml";
pub const AGENTS_DIR: &str = "agents";

/// Variables the platform supplies to every prompt (inputs are added on top).
pub const BUILTIN_VARS: &[&str] = &["owner", "repo", "branch", "sha", "output_path"];

static TOOL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_.:*()/@ -]{1,128}$").unwrap());
static MCP_NAME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_.-]{1,64}$").unwrap());
static AGENT_ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9_.-]{0,99}$").unwrap());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    Text,
    Select,
    Path,
}

/// One user-supplied value the agent genuinely needs. Serialises as the API's `AgentInput`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputSpec {
    pub name: String,
    pub label: String,
    #[serde(default)]
    pub help: Option<String>,
    pub kind: InputKind,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub choices: Option<Vec<String>>,
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default = "default_max_length")]
    pub max_length: usize,
}

fn default_max_length() -> usize {
    200
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSection {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptSection {
    pub template: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputSection {
    pub paths: Vec<String>,
    #[serde(default)]
    pub primary: Option<String>,
    #[serde(default = "default_min_bytes")]
    pub min_bytes: u64,
    #[serde(default = "default_max_bytes")]
    pub max_bytes: u64,
}

fn default_min_bytes() -> u64 {
    64
}
fn default_max_bytes() -> u64 {
    1024 * 1024
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSection {
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub allow_tools: Vec<String>,
    #[serde(default)]
    pub deny_tools: Vec<String>,
    #[serde(default)]
    pub disable_mcp_servers: Vec<String>,
    #[serde(default)]
    pub repository_instructions: bool,
    #[serde(default)]
    pub mcp_config: Option<String>,
}

fn default_timeout() -> u64 {
    1200
}

impl Default for RuntimeSection {
    fn default() -> Self {
        Self {
            timeout_secs: default_timeout(),
            allow_tools: vec!["read".into(), "write".into()],
            deny_tools: vec!["shell".into(), "url".into()],
            disable_mcp_servers: Vec::new(),
            repository_instructions: false,
            mcp_config: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    #[serde(default)]
    pub placeholder: bool,
    pub agent: AgentSection,
    pub prompt: PromptSection,
    #[serde(default)]
    pub inputs: Vec<InputSpec>,
    pub output: OutputSection,
    #[serde(default)]
    pub runtime: RuntimeSection,
}

/// Facts read from the agent file's YAML frontmatter.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentFrontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    /// `model:` pins the model; Copilot CLI then ignores `--model`.
    pub model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AgentBundle {
    pub dir: PathBuf,
    pub manifest: Manifest,
    /// Content hash of the bundle (first 12 hex chars of SHA-256): identifies exactly what ran.
    pub version: String,
    pub agent_file: PathBuf,
    pub frontmatter: AgentFrontmatter,
    template: Template,
    output_globs: GlobSet,
}

impl AgentBundle {
    pub fn load(dir: &Path) -> Result<Self> {
        let manifest_path = dir.join(MANIFEST_FILE);
        let text = std::fs::read_to_string(&manifest_path)
            .with_context(|| format!("reading agent bundle manifest {}", manifest_path.display()))?;
        let manifest: Manifest = serde_yaml::from_str(&text).context("parsing bundle.yaml")?;
        Self::from_manifest(dir, manifest)
    }

    fn from_manifest(dir: &Path, manifest: Manifest) -> Result<Self> {
        ensure!(manifest.schema_version == 1, "bundle.yaml: unsupported schema_version {}", manifest.schema_version);
        ensure!(AGENT_ID_RE.is_match(&manifest.agent.id), "bundle.yaml: invalid agent.id {:?}", manifest.agent.id);

        let agent_file = dir.join(AGENTS_DIR).join(format!("{}.agent.md", manifest.agent.id));
        let agent_text = std::fs::read_to_string(&agent_file).with_context(|| {
            format!(
                "agent.id is {:?} but {} does not exist (the id must equal the agent file name without .agent.md)",
                manifest.agent.id,
                agent_file.display()
            )
        })?;
        let frontmatter = parse_frontmatter(&agent_text)?;

        // Inputs
        let mut names = BTreeSet::new();
        for input in &manifest.inputs {
            ensure!(
                input.name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                    && input.name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                    && input.name.len() <= 64,
                "bundle.yaml: invalid input name {:?}",
                input.name
            );
            ensure!(
                !BUILTIN_VARS.contains(&input.name.as_str()),
                "bundle.yaml: input {:?} shadows a built-in variable",
                input.name
            );
            ensure!(names.insert(input.name.clone()), "bundle.yaml: duplicate input {:?}", input.name);
            ensure!(
                (1..=2000).contains(&input.max_length),
                "bundle.yaml: input {:?} max_length must be 1..=2000",
                input.name
            );
            match input.kind {
                InputKind::Select => {
                    let choices = input.choices.as_deref().unwrap_or_default();
                    ensure!(!choices.is_empty(), "bundle.yaml: select input {:?} needs choices", input.name);
                    if let Some(d) = input.default.as_deref().filter(|d| !d.is_empty()) {
                        ensure!(
                            choices.iter().any(|c| c == d),
                            "bundle.yaml: default of {:?} is not a choice",
                            input.name
                        );
                    }
                }
                _ => {
                    ensure!(input.choices.is_none(), "bundle.yaml: only select inputs take choices ({:?})", input.name)
                }
            }
        }

        // Output
        let out = &manifest.output;
        ensure!(!out.paths.is_empty(), "bundle.yaml: output.paths must not be empty");
        let mut gs = GlobSetBuilder::new();
        for p in &out.paths {
            ensure!(
                !p.starts_with('/')
                    && !p.split('/').any(|c| c == ".." || c == "." || c.is_empty())
                    && !p.contains('\\'),
                "bundle.yaml: output path {p:?} must be repository-relative"
            );
            ensure!(!p.to_ascii_lowercase().starts_with(".git/"), "bundle.yaml: output path {p:?} is inside .git");
            gs.add(Glob::new(p).with_context(|| format!("bundle.yaml: invalid output glob {p:?}"))?);
        }
        let output_globs = gs.build()?;
        if let Some(primary) = &out.primary {
            validate::relative_path(primary).map_err(|e| anyhow::anyhow!("bundle.yaml: output.primary: {e}"))?;
            ensure!(
                output_globs.is_match(primary),
                "bundle.yaml: output.primary {primary:?} does not match output.paths"
            );
        }
        ensure!(
            out.min_bytes <= out.max_bytes && out.max_bytes <= 10 * 1024 * 1024,
            "bundle.yaml: output size limits are inconsistent"
        );

        // Prompt
        let template = Template::parse(&manifest.prompt.template).context("bundle.yaml: prompt.template")?;
        for var in template.variables() {
            let known = BUILTIN_VARS.contains(&var.as_str()) || names.contains(&var);
            ensure!(known, "bundle.yaml: prompt.template uses unknown variable {{{{{var}}}}}");
            if var == "output_path" {
                ensure!(out.primary.is_some(), "bundle.yaml: {{{{output_path}}}} requires output.primary");
            }
        }

        // Runtime
        let rt = &manifest.runtime;
        ensure!((30..=86_400).contains(&rt.timeout_secs), "bundle.yaml: runtime.timeout_secs must be 30..=86400");
        for t in rt.allow_tools.iter().chain(&rt.deny_tools) {
            ensure!(TOOL_RE.is_match(t), "bundle.yaml: invalid tool pattern {t:?}");
        }
        for s in &rt.disable_mcp_servers {
            ensure!(MCP_NAME_RE.is_match(s), "bundle.yaml: invalid MCP server name {s:?}");
        }
        if let Some(cfg) = &rt.mcp_config {
            validate::relative_path(cfg).map_err(|e| anyhow::anyhow!("bundle.yaml: runtime.mcp_config: {e}"))?;
            ensure!(dir.join(cfg).is_file(), "bundle.yaml: runtime.mcp_config {cfg:?} not found in the bundle");
        }

        let version = hash_dir(dir)?;
        Ok(Self { dir: dir.to_owned(), manifest, version, agent_file, frontmatter, template, output_globs })
    }

    pub fn id(&self) -> &str {
        &self.manifest.agent.id
    }

    pub fn display_name(&self) -> &str {
        self.manifest
            .agent
            .display_name
            .as_deref()
            .or(self.frontmatter.name.as_deref())
            .unwrap_or(&self.manifest.agent.id)
    }

    pub fn description(&self) -> &str {
        self.manifest.agent.description.as_deref().or(self.frontmatter.description.as_deref()).unwrap_or_default()
    }

    /// The model pinned by the agent file, which overrides the portal's selection in Copilot CLI.
    pub fn locked_model(&self) -> Option<&str> {
        self.frontmatter.model.as_deref()
    }

    pub fn inputs(&self) -> &[InputSpec] {
        &self.manifest.inputs
    }

    pub fn output_paths(&self) -> &[String] {
        &self.manifest.output.paths
    }

    pub fn primary_output(&self) -> Option<&str> {
        self.manifest.output.primary.as_deref()
    }

    pub fn matches_output(&self, path: &str) -> bool {
        self.output_globs.is_match(path)
    }

    pub fn agents_dir(&self) -> PathBuf {
        self.dir.join(AGENTS_DIR)
    }

    /// Validates user-supplied inputs against the manifest and fills defaults. Unknown names are rejected.
    pub fn validate_inputs(&self, raw: &BTreeMap<String, String>) -> Result<BTreeMap<String, String>, AppError> {
        let mut errors: Vec<(String, String)> = Vec::new();
        for name in raw.keys() {
            if !self.manifest.inputs.iter().any(|i| &i.name == name) {
                errors.push((format!("inputs.{name}"), "Unknown input".to_owned()));
            }
        }
        let mut out = BTreeMap::new();
        for spec in &self.manifest.inputs {
            let value = raw
                .get(&spec.name)
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
                .or_else(|| spec.default.clone().filter(|d| !d.is_empty()))
                .unwrap_or_default();
            let field = format!("inputs.{}", spec.name);
            if value.is_empty() {
                if spec.required {
                    errors.push((field, "Required".to_owned()));
                }
                out.insert(spec.name.clone(), value);
                continue;
            }
            if let Err(e) = validate::input_text(&value, spec.max_length) {
                errors.push((field, e));
                continue;
            }
            match spec.kind {
                InputKind::Text => {}
                InputKind::Path => {
                    let normalised = value.trim_end_matches('/').to_owned();
                    if let Err(e) = validate::relative_path(&normalised) {
                        errors.push((field, e.to_owned()));
                        continue;
                    }
                    out.insert(spec.name.clone(), normalised);
                    continue;
                }
                InputKind::Select => {
                    if !spec.choices.as_deref().unwrap_or_default().contains(&value) {
                        errors.push((field, "Not one of the allowed choices".to_owned()));
                        continue;
                    }
                }
            }
            out.insert(spec.name.clone(), value);
        }
        if errors.is_empty() {
            Ok(out)
        } else {
            Err(AppError::validation(errors))
        }
    }

    /// Renders the prompt for one run. All values must already be validated.
    pub fn render_prompt(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
        sha: &str,
        inputs: &BTreeMap<String, String>,
    ) -> String {
        let mut vars = inputs.clone();
        vars.insert("owner".into(), owner.into());
        vars.insert("repo".into(), repo.into());
        vars.insert("branch".into(), branch.into());
        vars.insert("sha".into(), sha.into());
        if let Some(p) = self.primary_output() {
            vars.insert("output_path".into(), p.into());
        }
        self.template.render(&vars).trim().to_owned()
    }
}

/// Extracts `name`, `description` and `model` from the leading `---` YAML block, if any.
pub fn parse_frontmatter(text: &str) -> Result<AgentFrontmatter> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(rest) = text.strip_prefix("---").filter(|r| r.starts_with('\n') || r.starts_with("\r\n")) else {
        return Ok(AgentFrontmatter::default());
    };
    let Some(end) = rest.find("\n---") else { bail!("agent file: frontmatter is not closed with '---'") };
    let yaml: serde_yaml::Value = serde_yaml::from_str(&rest[..end]).context("agent file: invalid YAML frontmatter")?;
    let str_field =
        |key: &str| yaml.get(key).and_then(|v| v.as_str()).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    let model = match yaml.get("model") {
        Some(serde_yaml::Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        // VS Code accepts a prioritised list; the first entry is what runs when available.
        Some(serde_yaml::Value::Sequence(items)) => items.iter().find_map(|v| v.as_str()).map(|s| s.trim().to_owned()),
        _ => None,
    };
    Ok(AgentFrontmatter { name: str_field("name"), description: str_field("description"), model })
}

/// Lists bundle files (relative path, absolute path), skipping dot entries such as Kubernetes ConfigMap
/// `..data` links, and following symlinks to regular files.
pub fn bundle_files(dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(dir)
        .follow_links(true)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'))
    {
        let entry = entry.with_context(|| format!("walking {}", dir.display()))?;
        if entry.file_type().is_file() {
            let rel = entry.path().strip_prefix(dir)?.to_string_lossy().replace('\\', "/");
            out.push((rel, entry.path().to_owned()));
        }
    }
    Ok(out)
}

fn hash_dir(dir: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    for (rel, path) in bundle_files(dir)? {
        let data = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        hasher.update(rel.as_bytes());
        hasher.update([0]);
        hasher.update((data.len() as u64).to_le_bytes());
        hasher.update(&data);
    }
    Ok(hex::encode(hasher.finalize())[..12].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn write_bundle(dir: &Path, manifest: &str, agent: &str) {
        std::fs::create_dir_all(dir.join(AGENTS_DIR)).unwrap();
        std::fs::write(dir.join(MANIFEST_FILE), manifest).unwrap();
        std::fs::write(dir.join(AGENTS_DIR).join("bc.agent.md"), agent).unwrap();
    }

    const MANIFEST: &str = r#"
schema_version: 1
placeholder: true
agent: { id: bc }
prompt:
  template: "Document {{owner}}/{{repo}}@{{branch}}.{{#scope}} Only {{scope}}.{{/scope}} Write {{output_path}}."
inputs:
  - { name: scope, label: Scope, kind: path, required: false, max_length: 50 }
  - { name: depth, label: Depth, kind: select, choices: [short, full], default: short }
output:
  paths: ["docs/bounded-context.md", "docs/contexts/*.md"]
  primary: docs/bounded-context.md
runtime:
  allow_tools: [read, write]
  deny_tools: [shell, url]
"#;

    #[test]
    fn loads_and_renders() {
        let dir = tempfile::tempdir().unwrap();
        write_bundle(dir.path(), MANIFEST, "---\nname: BC\ndescription: d\n---\n# Agent\n");
        let b = AgentBundle::load(dir.path()).unwrap();
        assert_eq!(b.id(), "bc");
        assert_eq!(b.display_name(), "BC");
        assert_eq!(b.version.len(), 12);
        assert!(b.locked_model().is_none());
        assert!(b.matches_output("docs/contexts/orders.md"));
        assert!(!b.matches_output("README.md"));
        let inputs = b.validate_inputs(&BTreeMap::from([("scope".to_owned(), "services/orders/".to_owned())])).unwrap();
        assert_eq!(inputs["scope"], "services/orders");
        assert_eq!(inputs["depth"], "short");
        assert_eq!(
            b.render_prompt("acme", "shop", "main", "abc", &inputs),
            "Document acme/shop@main. Only services/orders. Write docs/bounded-context.md."
        );
    }

    #[test]
    fn version_changes_with_content() {
        let dir = tempfile::tempdir().unwrap();
        write_bundle(dir.path(), MANIFEST, "# v1\n");
        let v1 = AgentBundle::load(dir.path()).unwrap().version;
        std::fs::write(dir.path().join(AGENTS_DIR).join("bc.agent.md"), "# v2\n").unwrap();
        assert_ne!(v1, AgentBundle::load(dir.path()).unwrap().version);
    }

    #[test]
    fn input_validation_errors() {
        let dir = tempfile::tempdir().unwrap();
        write_bundle(dir.path(), MANIFEST, "# a\n");
        let b = AgentBundle::load(dir.path()).unwrap();
        for (k, v) in [("scope", "../etc"), ("scope", "a\nb"), ("depth", "huge"), ("bogus", "x")] {
            let err = b.validate_inputs(&BTreeMap::from([(k.to_owned(), v.to_owned())])).unwrap_err();
            assert_eq!(err.code, bcp_core::ErrorCode::ValidationFailed, "{k}={v:?}");
        }
    }

    #[test]
    fn model_pin_from_frontmatter() {
        assert_eq!(
            parse_frontmatter("---\nmodel: gpt-5.3-codex\n---\nx").unwrap().model.as_deref(),
            Some("gpt-5.3-codex")
        );
        assert_eq!(
            parse_frontmatter("---\nmodel: ['Claude Sonnet 4.5 (copilot)', 'GPT-5']\n---\n").unwrap().model.as_deref(),
            Some("Claude Sonnet 4.5 (copilot)")
        );
        assert_eq!(parse_frontmatter("# no frontmatter").unwrap(), AgentFrontmatter::default());
        assert!(parse_frontmatter("---\nname: x\n").is_err());
    }

    #[test]
    fn rejects_bad_manifests() {
        let cases = [
            MANIFEST.replace("{{output_path}}", "{{nope}}"),
            MANIFEST.replace("agent: { id: bc }", "agent: { id: other }"),
            MANIFEST.replace("\"docs/contexts/*.md\"", "\"../escape.md\""),
            MANIFEST.replace("primary: docs/bounded-context.md", "primary: README.md"),
            MANIFEST.replace("deny_tools: [shell, url]", "deny_tools: [\"shell; rm -rf\"]"),
            MANIFEST.replace("schema_version: 1", "schema_version: 2"),
            MANIFEST.replace("placeholder: true", "placeholder: true\nsurprise: 1"),
        ];
        for (i, m) in cases.iter().enumerate() {
            let dir = tempfile::tempdir().unwrap();
            write_bundle(dir.path(), m, "# a\n");
            assert!(AgentBundle::load(dir.path()).is_err(), "case {i} should fail");
        }
    }

    #[test]
    fn repository_bundle_is_valid() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../agent-bundle");
        if dir.exists() {
            let b = AgentBundle::load(&dir).unwrap();
            assert!(b.manifest.placeholder, "update this test when the real agent is installed");
        }
    }
}
