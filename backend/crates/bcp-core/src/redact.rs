//! Redaction of secrets in anything that may be logged or persisted (agent output, error messages).

use std::sync::LazyLock;

use regex::Regex;

pub const REDACTED: &str = "[REDACTED]";
pub const MAX_LINE_CHARS: usize = 2_000;

static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        // GitHub tokens: classic/OAuth/user-to-server/server-to-server/refresh, fine-grained PATs.
        r"\bgh[pousr]_[A-Za-z0-9]{20,}\b",
        r"\bgithub_pat_[A-Za-z0-9_]{20,}\b",
        // Authorization headers and basic-auth URLs.
        r"(?i)\b(bearer|token)\s+[A-Za-z0-9._~+/=-]{16,}",
        r"(?i)(https?://)[^/\s:@]+:[^/\s@]+@",
        // PEM private keys.
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
        // JWTs.
        r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b",
        // Azure storage/service-bus style keys.
        r"(?i)(AccountKey|SharedAccessKey|client_secret|password)=([^;\s&]+)",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("valid redaction regex"))
    .collect()
});

/// Redacts known patterns plus caller-supplied exact secrets.
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    secrets: Vec<String>,
}

impl Redactor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an exact value to redact (ignored when shorter than 8 characters to avoid mangling output).
    pub fn with_secret(mut self, secret: impl Into<String>) -> Self {
        let s = secret.into();
        if s.len() >= 8 {
            self.secrets.push(s);
        }
        self
    }

    pub fn redact(&self, input: &str) -> String {
        let mut out = input.to_owned();
        for s in &self.secrets {
            if out.contains(s.as_str()) {
                out = out.replace(s.as_str(), REDACTED);
            }
        }
        for (i, re) in PATTERNS.iter().enumerate() {
            out = match i {
                3 => re.replace_all(&out, format!("${{1}}{REDACTED}@")).into_owned(),
                6 => re.replace_all(&out, format!("${{1}}={REDACTED}")).into_owned(),
                _ => re.replace_all(&out, REDACTED).into_owned(),
            };
        }
        out
    }

    /// Redacts and truncates a single log line.
    pub fn line(&self, input: &str) -> String {
        truncate_chars(&self.redact(input.trim_end_matches(['\r', '\n'])), MAX_LINE_CHARS)
    }
}

pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str("… [truncated]");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_github_tokens() {
        let r = Redactor::new();
        let s = r.redact("token ghu_abcdefghijklmnopqrstuvwxyz0123 and github_pat_11ABCDEFG0123456789_abcdefghij");
        assert!(!s.contains("ghu_abc"), "{s}");
        assert!(!s.contains("github_pat_11"), "{s}");
    }

    #[test]
    fn redacts_exact_secrets_and_headers() {
        let r = Redactor::new().with_secret("super-secret-value");
        let s = r.redact("x super-secret-value y Authorization: Bearer abcdefghijklmnopqrstuvwxyz");
        assert_eq!(s.matches(REDACTED).count(), 2, "{s}");
    }

    #[test]
    fn redacts_basic_auth_urls_and_keys() {
        let r = Redactor::new();
        let s = r.redact("https://x-access-token:abc123@github.com/o/r.git AccountKey=Zm9vYmFy;");
        assert!(s.contains("https://[REDACTED]@github.com"), "{s}");
        assert!(s.contains("AccountKey=[REDACTED]"), "{s}");
    }

    #[test]
    fn redacts_pem() {
        let r = Redactor::new();
        let s = r.redact("-----BEGIN RSA PRIVATE KEY-----\nMIIabc\n-----END RSA PRIVATE KEY-----");
        assert_eq!(s, REDACTED);
    }

    #[test]
    fn short_secrets_are_ignored_and_lines_truncated() {
        let r = Redactor::new().with_secret("abc");
        assert_eq!(r.redact("abc"), "abc");
        let long = "x".repeat(MAX_LINE_CHARS + 10);
        assert!(r.line(&long).ends_with("[truncated]"));
    }
}
