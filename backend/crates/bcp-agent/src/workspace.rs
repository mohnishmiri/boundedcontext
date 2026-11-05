//! Repository workspace handling: safe tarball extraction, before/after snapshots, and classification of what
//! the agent changed against the bundle's output allowlist. Everything here is synchronous; call it from
//! `spawn_blocking`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Component, Path};

use bcp_core::domain::ChangeType;
use bcp_core::{validate, AppError, ErrorCode};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};

use crate::bundle::AgentBundle;

#[derive(Debug, Clone, Copy)]
pub struct ExtractLimits {
    pub max_bytes: u64,
    pub max_files: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExtractStats {
    pub files: u64,
    pub bytes: u64,
    /// Symlinks, hard links, devices and FIFOs are never materialised.
    pub skipped_special: u64,
    /// Entries whose names are not valid UTF-8 or not safe relative paths.
    pub skipped_unsafe: u64,
}

fn too_large(what: &str, limit: String) -> AppError {
    AppError::new(
        ErrorCode::RepositoryTooLarge,
        format!("The repository exceeds the portal's {what} limit ({limit}). Ask the administrators to raise it."),
    )
}

fn mib(bytes: u64) -> String {
    format!("{} MiB", bytes / 1_048_576)
}

/// Extracts a GitHub tarball (`<owner>-<repo>-<sha>/...`) into `dest`, stripping the top-level directory.
pub fn extract_tarball(tgz: &Path, dest: &Path, limits: ExtractLimits) -> Result<ExtractStats, AppError> {
    let file = File::open(tgz).map_err(|e| AppError::internal(format!("opening snapshot: {e}")))?;
    extract_tar_reader(GzDecoder::new(file), dest, limits)
}

pub fn extract_tar_reader<R: Read>(reader: R, dest: &Path, limits: ExtractLimits) -> Result<ExtractStats, AppError> {
    let invalid = |e: io::Error| {
        AppError::new(ErrorCode::GithubUnavailable, "The repository snapshot from GitHub could not be read.")
            .with_details(serde_json::json!({ "reason": e.to_string() }))
    };
    let mut archive = tar::Archive::new(reader);
    let mut stats = ExtractStats::default();
    fs::create_dir_all(dest).map_err(|e| AppError::internal(format!("creating workspace: {e}")))?;

    for entry in archive.entries().map_err(invalid)? {
        let mut entry = entry.map_err(invalid)?;
        let kind = entry.header().entry_type();
        if kind.is_pax_global_extensions()
            || kind.is_pax_local_extensions()
            || kind.is_gnu_longname()
            || kind.is_gnu_longlink()
        {
            continue;
        }
        let raw_path = entry.path().map_err(invalid)?.into_owned();
        let Some(rel) = safe_relative(&raw_path) else {
            stats.skipped_unsafe += 1;
            continue;
        };
        if rel.is_empty() {
            continue; // the top-level directory itself
        }
        let target = dest.join(&rel);
        if kind.is_dir() {
            fs::create_dir_all(&target).map_err(|e| AppError::internal(format!("creating directory: {e}")))?;
            continue;
        }
        if !(kind.is_file() || kind == tar::EntryType::Continuous) {
            stats.skipped_special += 1;
            continue;
        }
        stats.files += 1;
        if stats.files > limits.max_files {
            return Err(too_large("file count", format!("{} files", limits.max_files)));
        }
        let declared = entry.header().size().map_err(invalid)?;
        if stats.bytes.saturating_add(declared) > limits.max_bytes {
            return Err(too_large("size", mib(limits.max_bytes)));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| AppError::internal(format!("creating directory: {e}")))?;
        }
        let executable = entry.header().mode().map(|m| m & 0o111 != 0).unwrap_or(false);
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(if executable { 0o755 } else { 0o644 })
            .open(&target)
            .map_err(|e| AppError::internal(format!("writing {rel}: {e}")))?;
        let remaining = limits.max_bytes - stats.bytes;
        let copied = io::copy(&mut (&mut entry).take(remaining + 1), &mut out).map_err(invalid)?;
        stats.bytes += copied;
        if stats.bytes > limits.max_bytes {
            return Err(too_large("size", mib(limits.max_bytes)));
        }
        out.flush().map_err(|e| AppError::internal(format!("writing {rel}: {e}")))?;
    }
    Ok(stats)
}

/// Strips the first component and validates the rest: only normal components, UTF-8, not inside `.git`.
fn safe_relative(path: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for (i, c) in path.components().enumerate() {
        match c {
            Component::Normal(os) => {
                let s = os.to_str()?;
                if i > 0 {
                    parts.push(s);
                }
            }
            Component::CurDir if i == 0 => {}
            _ => return None,
        }
    }
    let rel = parts.join("/");
    if rel.is_empty() {
        return Some(rel);
    }
    validate::relative_path(&rel).ok()?;
    Some(rel)
}

// ----------------------------------------------------------------------------------------------------------
// Snapshots
// ----------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct FileState {
    pub size: u64,
    pub sha256: [u8; 32],
    pub symlink: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub files: BTreeMap<String, FileState>,
}

fn hash_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut f = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().into())
}

/// Records every file under `root` with its content hash (symlinks are recorded, never followed). Every file
/// is hashed: mtime shortcuts miss same-size rewrites within one coarse timestamp tick.
pub fn snapshot(root: &Path) -> io::Result<Snapshot> {
    let mut files = BTreeMap::new();
    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        // A `.git` directory is never part of the repository snapshot.
        .filter_entry(|e| !(e.depth() == 1 && e.file_name() == ".git"));
    for entry in walker {
        let entry = entry.map_err(io::Error::other)?;
        if entry.depth() == 0 || entry.file_type().is_dir() {
            continue;
        }
        let rel = entry.path().strip_prefix(root).map_err(io::Error::other)?.to_string_lossy().replace('\\', "/");
        let meta = entry.path().symlink_metadata()?;
        // Anything that is not a regular file (symlink, FIFO, socket) is recorded as a link: never read,
        // never committed.
        let state = if meta.file_type().is_symlink() || !meta.is_file() {
            FileState { size: 0, sha256: [0; 32], symlink: true }
        } else {
            FileState { size: meta.len(), sha256: hash_file(entry.path())?, symlink: false }
        };
        files.insert(rel, state);
    }
    Ok(Snapshot { files })
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Change {
    pub path: String,
    pub change: ChangeType,
}

pub fn diff_snapshots(before: &Snapshot, after: &Snapshot) -> Vec<Change> {
    let mut out = Vec::new();
    for (path, new) in &after.files {
        match before.files.get(path) {
            None => out.push(Change { path: path.clone(), change: ChangeType::Added }),
            Some(old) if old.sha256 != new.sha256 || old.symlink != new.symlink || old.size != new.size => {
                out.push(Change { path: path.clone(), change: ChangeType::Modified })
            }
            Some(_) => {}
        }
    }
    for path in before.files.keys() {
        if !after.files.contains_key(path) {
            out.push(Change { path: path.clone(), change: ChangeType::Deleted });
        }
    }
    out.sort();
    out
}

/// Captures current contents of files matching the output allowlist, so diffs can be computed after the run.
pub fn capture_base_outputs(bundle: &AgentBundle, root: &Path, snap: &Snapshot) -> BTreeMap<String, String> {
    snap.files
        .iter()
        .filter(|(p, s)| !s.symlink && s.size <= bundle.manifest.output.max_bytes && bundle.matches_output(p))
        .filter_map(|(p, _)| {
            fs::read(root.join(p)).ok().and_then(|b| String::from_utf8(b).ok()).map(|c| (p.clone(), c))
        })
        .collect()
}

/// Creates the literal folders of the output globs (e.g. `docs/archetypes`) at the root and under each scope
/// directory, because the agent has no shell to create them and stages its files elsewhere when they are
/// missing. Empty folders never appear in snapshots. Returns the folders that were created.
pub fn prepare_output_dirs(bundle: &AgentBundle, root: &Path, scopes: &[String]) -> Vec<String> {
    let mut bases = vec![String::new()];
    for scope in scopes {
        let scope = scope.trim_matches('/');
        if scope.is_empty() || validate::relative_path(scope).is_err() {
            continue;
        }
        if root.join(scope).symlink_metadata().is_ok_and(|m| m.is_dir()) {
            bases.push(format!("{scope}/"));
        }
    }
    let mut created = Vec::new();
    for glob in bundle.output_paths() {
        let glob = glob.strip_prefix("**/").unwrap_or(glob);
        let parts: Vec<&str> = glob.split('/').collect();
        let literal: Vec<&str> =
            parts[..parts.len() - 1].iter().take_while(|c| !c.contains(['*', '?', '[', '{'])).copied().collect();
        if literal.is_empty() {
            continue;
        }
        for base in &bases {
            let rel = format!("{base}{}", literal.join("/"));
            let dir = root.join(&rel);
            if validate::relative_path(&rel).is_ok() && !dir.exists() && fs::create_dir_all(&dir).is_ok() {
                created.push(rel);
            }
        }
    }
    created
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputFile {
    pub path: String,
    pub change_type: ChangeType,
    pub base_content: Option<String>,
    pub new_content: String,
    pub unified_diff: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputReport {
    pub primary: String,
    pub files: Vec<OutputFile>,
    /// Changes outside the allowlist (or deletions / links). Reported, never committed.
    pub unexpected: Vec<Change>,
    /// Validation problems tolerated because `output.strict` is false; left for manual review.
    pub warnings: Vec<String>,
}

/// Classifies the agent's changes and validates the publishable output.
pub fn collect_outputs(
    bundle: &AgentBundle,
    root: &Path,
    before: &Snapshot,
    base_contents: &BTreeMap<String, String>,
    after: &Snapshot,
) -> Result<OutputReport, AppError> {
    let out_cfg = &bundle.manifest.output;
    let changes = diff_snapshots(before, after);
    let mut files = Vec::new();
    let mut unexpected = Vec::new();

    for change in changes {
        let is_link = after.files.get(&change.path).is_some_and(|s| s.symlink);
        // Without a shell the agent can only create a folder by writing a file into it; such placeholders are noise.
        if change.change == ChangeType::Added && !is_link && is_dir_placeholder(&change.path) {
            continue;
        }
        let allowed = change.change != ChangeType::Deleted
            && !is_link
            && validate::relative_path(&change.path).is_ok()
            && bundle.matches_output(&change.path);
        if !allowed {
            unexpected.push(change);
            continue;
        }
        let state = &after.files[&change.path];
        if state.size > out_cfg.max_bytes {
            return Err(AppError::new(
                ErrorCode::OutputTooLarge,
                format!("{} is {} bytes; the limit is {} bytes.", change.path, state.size, out_cfg.max_bytes),
            ));
        }
        let bytes =
            fs::read(root.join(&change.path)).map_err(|e| AppError::internal(format!("reading output: {e}")))?;
        if bytes.contains(&0) {
            return Err(AppError::new(ErrorCode::OutputInvalid, format!("{} is not a text file.", change.path)));
        }
        let content = String::from_utf8(bytes).map_err(|_| {
            AppError::new(ErrorCode::OutputInvalid, format!("{} is not valid UTF-8 text.", change.path))
        })?;
        let base = base_contents.get(&change.path).cloned();
        let diff = similar::TextDiff::from_lines(base.as_deref().unwrap_or(""), &content)
            .unified_diff()
            .context_radius(3)
            .header(
                &if base.is_some() { format!("a/{}", change.path) } else { "/dev/null".to_owned() },
                &format!("b/{}", change.path),
            )
            .to_string();
        files.push(OutputFile {
            path: change.path.clone(),
            change_type: change.change,
            base_content: base,
            size_bytes: content.len() as u64,
            sha256: hex::encode(state.sha256),
            unified_diff: diff,
            new_content: content,
        });
    }

    let expected = bundle.primary_output().map(str::to_owned).unwrap_or_else(|| bundle.output_paths().join(", "));
    if files.is_empty() {
        let unchanged =
            bundle.primary_output().is_some_and(|p| after.files.contains_key(p) && before.files.contains_key(p));
        let message = if unchanged {
            format!("The agent did not change {expected}: the generated content is identical to the existing file, so there is nothing to publish.")
        } else if unexpected.is_empty() {
            format!(
                "The agent finished without writing {expected}. It made no file changes at all in the repository; \
                 its final message in the job log explains why it stopped."
            )
        } else {
            const SHOWN: usize = 20;
            let mut listed: Vec<String> =
                unexpected.iter().take(SHOWN).map(|c| format!("{} {}", c.change, c.path)).collect();
            if unexpected.len() > SHOWN {
                listed.push(format!("… and {} more", unexpected.len() - SHOWN));
            }
            format!(
                "The agent finished without writing {expected}. It changed only files outside the allowed output paths, \
                 which are never kept: {}.",
                listed.join(", ")
            )
        };
        return Err(AppError::new(ErrorCode::OutputMissing, message).retryable(false));
    }
    let mut warnings = Vec::new();
    let fallback_primary = || {
        files
            .iter()
            .find(|f| {
                let lower = f.path.to_ascii_lowercase();
                lower.contains("bounded-context") || lower.contains("bounded_context")
            })
            .unwrap_or(&files[0])
            .path
            .clone()
    };
    let primary = match bundle.primary_output() {
        Some(p) => match files.iter().find(|f| f.path == p) {
            Some(f) => f.path.clone(),
            None if !out_cfg.strict => {
                warnings.push(format!("The agent did not write {p}; review the other generated files."));
                fallback_primary()
            }
            None => {
                let message = format!("The agent did not write {p}.");
                return Err(AppError::new(ErrorCode::OutputMissing, message).retryable(false));
            }
        },
        None => fallback_primary(),
    };
    let primary_file = files.iter().find(|f| f.path == primary).expect("primary is in files");
    let meaningful = primary_file.new_content.trim().len() as u64;
    let size_problem = if meaningful == 0 {
        Some(format!("{primary} is empty."))
    } else if meaningful < out_cfg.min_bytes {
        Some(format!("{primary} is only {meaningful} bytes; at least {} were expected.", out_cfg.min_bytes))
    } else {
        None
    };
    if let Some(problem) = size_problem {
        if out_cfg.strict {
            return Err(AppError::new(ErrorCode::OutputEmpty, problem));
        }
        warnings.push(problem);
    }
    Ok(OutputReport { primary, files, unexpected, warnings })
}

fn is_dir_placeholder(path: &str) -> bool {
    matches!(path.rsplit('/').next(), Some(".keep" | ".gitkeep"))
}

/// Paths of all regular files (used to describe the repository to the fake agent and in tests).
pub fn list_files(root: &Path) -> BTreeSet<String> {
    snapshot(root).map(|s| s.files.into_keys().collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tar_gz(entries: &[(&str, &[u8], tar::EntryType, Option<&str>)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        for (path, data, kind, link) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(*kind);
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            if let Some(l) = link {
                h.set_link_name(l).unwrap();
            }
            // Write raw names so malicious paths survive the builder's own checks.
            let name = path.as_bytes();
            h.as_old_mut().name[..name.len()].copy_from_slice(name);
            h.set_cksum();
            builder.append(&h, *data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    const LIMITS: ExtractLimits = ExtractLimits { max_bytes: 1_000_000, max_files: 100 };

    #[test]
    fn extracts_and_strips_top_level_dir() {
        let tgz = tar_gz(&[
            ("acme-shop-abc/", b"", tar::EntryType::Directory, None),
            ("acme-shop-abc/README.md", b"# Shop\n", tar::EntryType::Regular, None),
            ("acme-shop-abc/src/main.rs", b"fn main() {}\n", tar::EntryType::Regular, None),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let stats = extract_tar_reader(GzDecoder::new(&tgz[..]), dir.path(), LIMITS).unwrap();
        assert_eq!(stats.files, 2);
        assert_eq!(fs::read_to_string(dir.path().join("src/main.rs")).unwrap(), "fn main() {}\n");
    }

    #[test]
    fn skips_traversal_links_and_devices() {
        let tgz = tar_gz(&[
            ("top/../../evil.txt", b"x", tar::EntryType::Regular, None),
            ("/abs.txt", b"x", tar::EntryType::Regular, None),
            ("top/.git/config", b"x", tar::EntryType::Regular, None),
            ("top/link", b"", tar::EntryType::Symlink, Some("/etc/passwd")),
            ("top/hard", b"", tar::EntryType::Link, Some("top/ok.txt")),
            ("top/dev", b"", tar::EntryType::Char, None),
            ("top/ok.txt", b"fine", tar::EntryType::Regular, None),
        ]);
        let parent = tempfile::tempdir().unwrap();
        let dest = parent.path().join("ws");
        let stats = extract_tar_reader(GzDecoder::new(&tgz[..]), &dest, LIMITS).unwrap();
        assert_eq!(stats.files, 1);
        assert_eq!(stats.skipped_special, 3);
        assert_eq!(stats.skipped_unsafe, 3);
        assert!(!parent.path().join("evil.txt").exists());
        assert_eq!(list_files(&dest).into_iter().collect::<Vec<_>>(), ["ok.txt"]);
    }

    #[test]
    fn enforces_limits() {
        let big = vec![b'a'; 2000];
        let tgz = tar_gz(&[("t/a", &big, tar::EntryType::Regular, None)]);
        let dir = tempfile::tempdir().unwrap();
        let err =
            extract_tar_reader(GzDecoder::new(&tgz[..]), dir.path(), ExtractLimits { max_bytes: 1000, max_files: 10 })
                .unwrap_err();
        assert_eq!(err.code, ErrorCode::RepositoryTooLarge);

        let tgz = tar_gz(&[("t/a", b"1", tar::EntryType::Regular, None), ("t/b", b"2", tar::EntryType::Regular, None)]);
        let dir = tempfile::tempdir().unwrap();
        let err =
            extract_tar_reader(GzDecoder::new(&tgz[..]), dir.path(), ExtractLimits { max_bytes: 1000, max_files: 1 })
                .unwrap_err();
        assert_eq!(err.code, ErrorCode::RepositoryTooLarge);
    }

    fn bundle(dir: &Path, primary: bool) -> AgentBundle {
        let manifest = format!(
            "schema_version: 1\nagent: {{ id: bc }}\nprompt: {{ template: x }}\noutput:\n  paths: [docs/*.md]\n{}  min_bytes: 10\n",
            if primary { "  primary: docs/bc.md\n" } else { "" }
        );
        std::fs::create_dir_all(dir.join("agents")).unwrap();
        std::fs::write(dir.join("bundle.yaml"), manifest).unwrap();
        std::fs::write(dir.join("agents/bc.agent.md"), "# a").unwrap();
        AgentBundle::load(dir).unwrap()
    }

    #[test]
    fn classifies_changes_against_allowlist() {
        let bdir = tempfile::tempdir().unwrap();
        let b = bundle(bdir.path(), true);
        let ws = tempfile::tempdir().unwrap();
        let root = ws.path();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("README.md"), "readme").unwrap();
        fs::write(root.join("docs/old.md"), "old content\n").unwrap();
        fs::write(root.join("delete-me.txt"), "x").unwrap();
        let before = snapshot(root).unwrap();
        let base = capture_base_outputs(&b, root, &before);
        assert_eq!(base.keys().collect::<Vec<_>>(), ["docs/old.md"]);

        fs::write(root.join("docs/bc.md"), "# Bounded contexts\n\nOrders, Billing.\n").unwrap();
        fs::write(root.join("docs/old.md"), "new content\n").unwrap();
        fs::write(root.join("README.md"), "tampered").unwrap();
        fs::remove_file(root.join("delete-me.txt")).unwrap();
        fs::create_dir_all(root.join("docs/adr")).unwrap();
        fs::write(root.join("docs/adr/.keep"), "").unwrap();
        fs::write(root.join(".gitkeep"), "").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", root.join("docs/link.md")).unwrap();
        let after = snapshot(root).unwrap();

        let report = collect_outputs(&b, root, &before, &base, &after).unwrap();
        assert_eq!(report.primary, "docs/bc.md");
        let paths: Vec<_> = report.files.iter().map(|f| (f.path.as_str(), f.change_type)).collect();
        assert_eq!(paths, [("docs/bc.md", ChangeType::Added), ("docs/old.md", ChangeType::Modified)]);
        let old = report.files.iter().find(|f| f.path == "docs/old.md").unwrap();
        assert!(
            old.unified_diff.contains("-old content") && old.unified_diff.contains("+new content"),
            "{}",
            old.unified_diff
        );
        let unexpected: Vec<_> = report.unexpected.iter().map(|c| (c.path.as_str(), c.change)).collect();
        assert_eq!(
            unexpected,
            [
                ("README.md", ChangeType::Modified),
                ("delete-me.txt", ChangeType::Deleted),
                ("docs/link.md", ChangeType::Added)
            ]
        );
    }

    #[test]
    fn output_errors() {
        let bdir = tempfile::tempdir().unwrap();
        let b = bundle(bdir.path(), true);
        let ws = tempfile::tempdir().unwrap();
        let root = ws.path();
        fs::create_dir_all(root.join("docs")).unwrap();
        let before = snapshot(root).unwrap();

        let after = snapshot(root).unwrap();
        assert_eq!(
            collect_outputs(&b, root, &before, &BTreeMap::new(), &after).unwrap_err().code,
            ErrorCode::OutputMissing
        );

        fs::write(root.join("docs/other.md"), "# Something else entirely\n").unwrap();
        let after = snapshot(root).unwrap();
        assert_eq!(
            collect_outputs(&b, root, &before, &BTreeMap::new(), &after).unwrap_err().code,
            ErrorCode::OutputMissing
        );

        fs::write(root.join("docs/bc.md"), "   \n").unwrap();
        let after = snapshot(root).unwrap();
        assert_eq!(
            collect_outputs(&b, root, &before, &BTreeMap::new(), &after).unwrap_err().code,
            ErrorCode::OutputEmpty
        );

        fs::write(root.join("docs/bc.md"), [0xff, 0xfe, b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i']).unwrap();
        let after = snapshot(root).unwrap();
        assert_eq!(
            collect_outputs(&b, root, &before, &BTreeMap::new(), &after).unwrap_err().code,
            ErrorCode::OutputInvalid
        );
    }

    #[test]
    fn primary_defaults_to_first_allowed_file() {
        let bdir = tempfile::tempdir().unwrap();
        let b = bundle(bdir.path(), false);
        let ws = tempfile::tempdir().unwrap();
        let root = ws.path();
        let before = snapshot(root).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join("docs/z.md"), "# Zeta context document\n").unwrap();
        fs::write(root.join("docs/a.md"), "# Alpha context document\n").unwrap();
        let after = snapshot(root).unwrap();
        assert_eq!(collect_outputs(&b, root, &before, &BTreeMap::new(), &after).unwrap().primary, "docs/a.md");
    }

    #[test]
    fn prepares_literal_output_folders() {
        let bdir = tempfile::tempdir().unwrap();
        let manifest = "schema_version: 1\nagent: { id: bc }\nprompt: { template: x }\noutput:\n  paths: \
                        [docs/BC-*.md, docs/archetypes/04-ADRs/*.md, \"**/docs/archetypes/*.md\", \"*.md\"]\n";
        fs::create_dir_all(bdir.path().join("agents")).unwrap();
        fs::write(bdir.path().join("bundle.yaml"), manifest).unwrap();
        fs::write(bdir.path().join("agents/bc.agent.md"), "# a").unwrap();
        let b = AgentBundle::load(bdir.path()).unwrap();
        let ws = tempfile::tempdir().unwrap();
        let root = ws.path();
        fs::create_dir_all(root.join("svc")).unwrap();
        let scopes = ["svc".to_owned(), "../escape".to_owned(), "missing".to_owned()];

        let mut created = prepare_output_dirs(&b, root, &scopes);
        created.sort();
        assert_eq!(created, ["docs", "docs/archetypes/04-ADRs", "svc/docs", "svc/docs/archetypes/04-ADRs"]);
        assert!(root.join("svc/docs/archetypes").is_dir());
        assert!(!root.join("missing").exists());
        assert!(snapshot(root).unwrap().files.is_empty());
    }

    #[test]
    fn missing_output_names_files_written_elsewhere() {
        let bdir = tempfile::tempdir().unwrap();
        let b = bundle(bdir.path(), false);
        let ws = tempfile::tempdir().unwrap();
        let root = ws.path();
        let before = snapshot(root).unwrap();
        let err = collect_outputs(&b, root, &before, &BTreeMap::new(), &before).unwrap_err();
        assert!(err.message.contains("no file changes at all"), "{}", err.message);

        fs::create_dir_all(root.join("Docs")).unwrap();
        fs::write(root.join("Docs/bc.md"), "# Bounded contexts\n").unwrap();
        let after = snapshot(root).unwrap();
        let err = collect_outputs(&b, root, &before, &BTreeMap::new(), &after).unwrap_err();
        assert_eq!(err.code, ErrorCode::OutputMissing);
        assert!(err.message.contains("added Docs/bc.md"), "{}", err.message);
    }

    #[test]
    fn lenient_mode_keeps_output_for_review() {
        let bdir = tempfile::tempdir().unwrap();
        let mut b = bundle(bdir.path(), true);
        b.manifest.output.strict = false;
        let ws = tempfile::tempdir().unwrap();
        let root = ws.path();
        fs::create_dir_all(root.join("docs")).unwrap();
        let before = snapshot(root).unwrap();

        fs::write(root.join("docs/other.md"), "short\n").unwrap();
        let after = snapshot(root).unwrap();
        let report = collect_outputs(&b, root, &before, &BTreeMap::new(), &after).unwrap();
        assert_eq!(report.primary, "docs/other.md");
        assert_eq!(report.warnings.len(), 2, "{:?}", report.warnings);

        // Nothing allowed was written: there is nothing to keep, so it still fails.
        let err = collect_outputs(&b, root, &before, &BTreeMap::new(), &before).unwrap_err();
        assert_eq!(err.code, ErrorCode::OutputMissing);
    }
}
