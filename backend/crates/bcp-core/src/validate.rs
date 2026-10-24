//! Strict validators for every externally supplied identifier. Values that pass these checks are safe to
//! place in GitHub API URL paths (after percent-encoding), Git ref names, prompts and file-system paths
//! relative to a workspace root.

use std::sync::LazyLock;

use regex::Regex;

static OWNER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9](?:[A-Za-z0-9-]{0,37}[A-Za-z0-9])?$").unwrap());
static REPO_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9._-]{1,100}$").unwrap());
static MODEL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9._-]{0,63}$").unwrap());

pub const MAX_BRANCH_LEN: usize = 255;
pub const MAX_PATH_LEN: usize = 400;

/// GitHub user or organisation login.
pub fn owner(value: &str) -> Result<(), &'static str> {
    if OWNER_RE.is_match(value) {
        Ok(())
    } else {
        Err("Must be a GitHub account name (letters, digits and single hyphens, max 39 characters)")
    }
}

/// GitHub repository name.
pub fn repo(value: &str) -> Result<(), &'static str> {
    if value == "." || value == ".." || !REPO_RE.is_match(value) {
        return Err("Must be a GitHub repository name (letters, digits, '.', '-', '_', max 100 characters)");
    }
    Ok(())
}

/// Git branch name, following `git check-ref-format --branch` rules.
pub fn branch(value: &str) -> Result<(), &'static str> {
    const MSG: &str = "Not a valid Git branch name";
    if value.is_empty() || value.len() > MAX_BRANCH_LEN || value == "@" {
        return Err(MSG);
    }
    if value.starts_with('/') || value.starts_with('-') || value.ends_with('/') || value.ends_with('.') {
        return Err(MSG);
    }
    if value.ends_with(".lock") || value.contains("..") || value.contains("@{") || value.contains("//") {
        return Err(MSG);
    }
    if value
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\'))
    {
        return Err(MSG);
    }
    if value.split('/').any(|part| part.starts_with('.') || part.ends_with(".lock")) {
        return Err(MSG);
    }
    Ok(())
}

/// Model identifier as passed to `copilot --model`.
pub fn model_id(value: &str) -> Result<(), &'static str> {
    if MODEL_RE.is_match(value) {
        Ok(())
    } else {
        Err("Not a valid model identifier")
    }
}

/// A normalised, repository-relative file path: forward slashes, no `.`/`..`/empty components, no leading
/// slash, no control characters, not inside `.git`.
pub fn relative_path(value: &str) -> Result<(), &'static str> {
    const MSG: &str = "Must be a relative path inside the repository (no '..', no leading '/')";
    if value.is_empty() || value.len() > MAX_PATH_LEN {
        return Err(MSG);
    }
    if value.starts_with('/') || value.contains('\\') || value.chars().any(char::is_control) {
        return Err(MSG);
    }
    // Windows drive letters / URLs.
    if value.contains(':') {
        return Err(MSG);
    }
    for part in value.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.len() > 255 {
            return Err(MSG);
        }
    }
    let first = value.split('/').next().unwrap_or_default();
    if first.eq_ignore_ascii_case(".git") {
        return Err(MSG);
    }
    Ok(())
}

/// Free text supplied as an agent input: no control characters except none at all — prompts are single line.
pub fn input_text(value: &str, max_len: usize) -> Result<(), String> {
    if value.chars().count() > max_len {
        return Err(format!("Must be at most {max_len} characters"));
    }
    if value.chars().any(char::is_control) {
        return Err("Must not contain control characters or line breaks".to_owned());
    }
    Ok(())
}

/// Lowercase slug for branch names: `[a-z0-9-]`, collapsed hyphens, max `max_len` characters.
pub fn slugify(value: &str, max_len: usize) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_dash = true;
    for c in value.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out.truncate(max_len);
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "repo".to_owned()
    } else {
        out
    }
}

/// A same-origin path for post-login redirects. Rejects absolute URLs and protocol-relative `//host`.
pub fn return_to_path(value: &str) -> Option<&str> {
    let ok = value.starts_with('/')
        && !value.starts_with("//")
        && !value.starts_with("/\\")
        && value.len() <= 512
        && !value.chars().any(|c| c.is_control() || c == '\\');
    ok.then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owners() {
        for ok in ["acme", "a", "octo-dev", "A1-b2", &"a".repeat(39)] {
            assert!(owner(ok).is_ok(), "{ok}");
        }
        for bad in ["", "-acme", "acme-", "ac me", "acme/x", "../x", &"a".repeat(40), "a_b"] {
            assert!(owner(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn repos() {
        for ok in ["shop", "my.repo", "a_b-c", ".github"] {
            assert!(repo(ok).is_ok(), "{ok}");
        }
        for bad in ["", ".", "..", "a/b", "a b", "a?b", &"a".repeat(101)] {
            assert!(repo(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn branches() {
        for ok in ["main", "feature/x", "release-1.2", "users/jo/fix#12", "v1.0"] {
            assert!(branch(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "@",
            "-x",
            "/x",
            "x/",
            "x.",
            "a..b",
            "a@{b",
            "a//b",
            "a b",
            "a~b",
            "a^b",
            "a:b",
            "a?b",
            "a*b",
            "a[b",
            "a\\b",
            "x.lock",
            "a/.hidden",
            "a/b.lock/c",
            "a\u{7}b",
        ] {
            assert!(branch(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn models() {
        assert!(model_id("auto").is_ok());
        assert!(model_id("claude-haiku-4.5").is_ok());
        assert!(model_id("gpt-5.3-codex").is_ok());
        for bad in ["", "GPT", "a b", "../x", "x;rm -rf", "-x", &"a".repeat(65)] {
            assert!(model_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn paths() {
        for ok in ["docs/bounded-context.md", "a", "src/main.rs", ".github/x.md"] {
            assert!(relative_path(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../b",
            "a/./b",
            "a//b",
            "a/",
            "a\\b",
            "C:/x",
            ".git/config",
            ".GIT/hooks",
            "a\nb",
            &"a/".repeat(201),
        ] {
            assert!(relative_path(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("My_Repo.Name", 40), "my-repo-name");
        assert_eq!(slugify("---", 40), "repo");
        assert_eq!(slugify("abcdefghij-klm", 11), "abcdefghij");
    }

    #[test]
    fn return_to() {
        assert_eq!(return_to_path("/jobs/1"), Some("/jobs/1"));
        assert_eq!(return_to_path("//evil.com"), None);
        assert_eq!(return_to_path("https://evil.com"), None);
        assert_eq!(return_to_path("/\\evil.com"), None);
    }

    #[test]
    fn input_text_rejects_newlines() {
        assert!(input_text("ok", 10).is_ok());
        assert!(input_text("a\nb", 10).is_err());
        assert!(input_text("abcdefghijk", 10).is_err());
    }
}
