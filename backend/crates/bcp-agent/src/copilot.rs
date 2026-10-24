//! Runs the bundle's agent through GitHub Copilot CLI in non-interactive mode.
//!
//! The command is an argv vector (never a shell string). The child gets a scrubbed environment, a private
//! `COPILOT_HOME` holding a verbatim copy of the bundle's agent files (so a repository cannot shadow the agent
//! — personal agents win on id collisions), and its own process group so a timeout or cancellation kills the
//! whole process tree.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bcp_core::redact::Redactor;
use bcp_core::{AppError, ErrorCode};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::adapter::{
    AgentAdapter, AgentDescriptor, AgentError, AgentRunOutcome, AgentRunRequest, LogLevel, ProgressSink,
};
use crate::bundle::{bundle_files, AgentBundle};

const MAX_LINE_BYTES: usize = 8 * 1024;
const TAIL_LINES: usize = 40;
const KILL_GRACE: Duration = Duration::from_secs(10);
const PIPE_DRAIN: Duration = Duration::from_secs(5);

/// Parent environment variables passed through to the CLI (network/proxy/CA plumbing only).
const PASSTHROUGH_ENV: &[&str] = &[
    "PATH",
    "LANG",
    "LC_ALL",
    "TZ",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "NO_PROXY",
    "no_proxy",
    "NODE_EXTRA_CA_CERTS",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];

#[derive(Debug, Clone)]
pub struct CopilotCliAdapter {
    bundle: Arc<AgentBundle>,
    copilot_bin: PathBuf,
}

/// Fully resolved command line (exposed for tests and `bcp-agent-run --dry-run`).
#[derive(Debug, Clone, PartialEq)]
pub struct CommandLine {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Environment without the token (added at spawn time).
    pub env: Vec<(String, String)>,
}

impl CopilotCliAdapter {
    pub fn new(bundle: Arc<AgentBundle>, copilot_bin: impl Into<PathBuf>) -> Self {
        Self { bundle, copilot_bin: copilot_bin.into() }
    }

    pub fn bundle(&self) -> &AgentBundle {
        &self.bundle
    }

    pub fn command_line(&self, req: &AgentRunRequest) -> CommandLine {
        let b = &self.bundle;
        let rt = &b.manifest.runtime;
        let prompt = b.render_prompt(&req.owner, &req.repo, &req.branch, &req.sha, &req.inputs);
        let scratch = &req.scratch_dir;

        let mut args: Vec<String> = vec!["-p".into(), prompt, "--agent".into(), b.id().to_owned()];
        // A model pinned in the agent's frontmatter overrides --model; omit it to avoid a misleading argv.
        if b.locked_model().is_none() {
            if let Some(m) = &req.model {
                args.extend(["--model".into(), m.clone()]);
            }
        }
        args.extend(["-s".into(), "--no-ask-user".into()]);
        for t in &rt.allow_tools {
            args.extend(["--allow-tool".into(), t.clone()]);
        }
        for t in &rt.deny_tools {
            args.extend(["--deny-tool".into(), t.clone()]);
        }
        for s in &rt.disable_mcp_servers {
            args.extend(["--disable-mcp-server".into(), s.clone()]);
        }
        if let Some(cfg) = &rt.mcp_config {
            args.extend(["--additional-mcp-config".into(), format!("@{}", b.dir.join(cfg).display())]);
        }
        if !rt.repository_instructions {
            args.push("--no-custom-instructions".into());
        }
        args.extend(["--log-dir".into(), scratch.join("logs").display().to_string()]);
        args.extend(["--share".into(), scratch.join("transcript.md").display().to_string()]);

        let home = scratch.join("home");
        let mut env: Vec<(String, String)> =
            PASSTHROUGH_ENV.iter().filter_map(|k| std::env::var(k).ok().map(|v| ((*k).to_owned(), v))).collect();
        if !env.iter().any(|(k, _)| k == "PATH") {
            env.push(("PATH".into(), "/usr/local/bin:/usr/bin:/bin".into()));
        }
        if !env.iter().any(|(k, _)| k == "LANG") {
            env.push(("LANG".into(), "C.UTF-8".into()));
        }
        env.extend([
            ("HOME".into(), home.display().to_string()),
            ("XDG_CONFIG_HOME".into(), home.join(".config").display().to_string()),
            ("XDG_CACHE_HOME".into(), home.join(".cache").display().to_string()),
            ("XDG_STATE_HOME".into(), home.join(".local/state").display().to_string()),
            ("TMPDIR".into(), scratch.join("tmp").display().to_string()),
            ("COPILOT_HOME".into(), scratch.join("copilot-home").display().to_string()),
            ("COPILOT_AUTO_UPDATE".into(), "false".into()),
            ("NO_COLOR".into(), "1".into()),
            ("TERM".into(), "dumb".into()),
        ]);
        CommandLine { program: self.copilot_bin.clone(), args, env }
    }

    /// Creates the private directories and copies the bundle's agent files into `COPILOT_HOME/agents`.
    fn prepare_scratch(&self, scratch: &Path) -> std::io::Result<()> {
        for d in ["home", "tmp", "logs", "copilot-home/agents"] {
            std::fs::create_dir_all(scratch.join(d))?;
        }
        let agents_src = self.bundle.agents_dir();
        let agents_dst = scratch.join("copilot-home").join("agents");
        let files = bundle_files(&agents_src).map_err(std::io::Error::other)?;
        for (rel, src) in files {
            let dst = agents_dst.join(&rel);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&src, &dst)?;
        }
        Ok(())
    }
}

#[async_trait]
impl AgentAdapter for CopilotCliAdapter {
    fn describe(&self) -> AgentDescriptor {
        let b = &self.bundle;
        AgentDescriptor {
            id: b.id().to_owned(),
            name: b.display_name().to_owned(),
            description: b.description().to_owned(),
            version: b.version.clone(),
            placeholder: b.manifest.placeholder,
            output_paths: b.output_paths().to_vec(),
            inputs: b.inputs().to_vec(),
            locked_model: b.locked_model().map(str::to_owned),
        }
    }

    async fn preflight(&self) -> Result<String, AgentError> {
        let scratch =
            tempfile::tempdir().map_err(|e| AgentError::Failed(AppError::internal(format!("tempdir: {e}"))))?;
        let mut cmd = Command::new(&self.copilot_bin);
        cmd.arg("--version")
            .env_clear()
            .env("HOME", scratch.path())
            .env("COPILOT_HOME", scratch.path().join("copilot-home"))
            .env("COPILOT_AUTO_UPDATE", "false")
            .env("NO_COLOR", "1")
            .env("PATH", std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()))
            .stdin(Stdio::null())
            .kill_on_drop(true);
        let out = tokio::time::timeout(Duration::from_secs(30), cmd.output())
            .await
            .map_err(|_| unavailable("GitHub Copilot CLI did not answer `--version` within 30 seconds."))?
            .map_err(|e| unavailable(&format!("GitHub Copilot CLI could not be started ({e}).")))?;
        if !out.status.success() {
            return Err(unavailable("GitHub Copilot CLI `--version` failed."));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let version = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("unknown");
        Ok(bcp_core::redact::truncate_chars(version, 64))
    }

    async fn run(
        &self,
        req: AgentRunRequest,
        sink: &dyn ProgressSink,
        cancel: CancellationToken,
    ) -> Result<AgentRunOutcome, AgentError> {
        let started = Instant::now();
        let cli_version = self.preflight().await?;
        self.prepare_scratch(&req.scratch_dir)
            .map_err(|e| AgentError::Failed(AppError::internal(format!("preparing agent home: {e}"))))?;
        let cl = self.command_line(&req);
        sink.log(
            LogLevel::Info,
            &format!(
                "Starting GitHub Copilot CLI {cli_version} with agent '{}' (bundle {}), model {}",
                self.bundle.id(),
                self.bundle.version,
                self.bundle
                    .locked_model()
                    .map(|m| format!("{m} (pinned by agent)"))
                    .or(req.model.clone())
                    .unwrap_or_else(|| "default".into()),
            ),
        )
        .await;

        let mut cmd = Command::new(&cl.program);
        cmd.args(&cl.args)
            .current_dir(&req.workspace)
            .env_clear()
            .envs(cl.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .env("COPILOT_GITHUB_TOKEN", req.copilot_token.expose())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .process_group(0);
        let mut child =
            cmd.spawn().map_err(|e| unavailable(&format!("GitHub Copilot CLI could not be started ({e}).")))?;
        let pid = child.id().map(|p| p as i32);

        let redactor = Redactor::new().with_secret(req.copilot_token.expose());
        let tail: Mutex<VecDeque<String>> = Mutex::new(VecDeque::with_capacity(TAIL_LINES));
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let mut pumps = std::pin::pin!(futures::future::join(
            pump(stdout, sink, &redactor, &tail, false),
            pump(stderr, sink, &redactor, &tail, true),
        ));

        let waiter = async {
            let ended = tokio::select! {
                status = child.wait() => Ok(status),
                _ = tokio::time::sleep(req.timeout) => Err(AgentError::Timeout(req.timeout)),
                _ = cancel.cancelled() => Err(AgentError::Cancelled),
            };
            if ended.is_err() {
                signal_group(pid, libc::SIGTERM);
                if tokio::time::timeout(KILL_GRACE, child.wait()).await.is_err() {
                    signal_group(pid, libc::SIGKILL);
                    let _ = child.wait().await;
                }
            }
            // Reap anything the agent left behind in its process group.
            signal_group(pid, libc::SIGKILL);
            ended
        };
        let mut waiter = std::pin::pin!(waiter);
        let mut pumps_done = false;
        let ended = loop {
            tokio::select! {
                r = &mut waiter => break r,
                _ = &mut pumps, if !pumps_done => pumps_done = true,
            }
        };
        if !pumps_done {
            let _ = tokio::time::timeout(PIPE_DRAIN, &mut pumps).await;
        }

        let status = ended?.map_err(|e| AgentError::Failed(AppError::internal(format!("waiting for agent: {e}"))))?;
        let duration = started.elapsed();
        if !status.success() {
            let tail: Vec<String> = tail.lock().map(|t| t.iter().cloned().collect()).unwrap_or_default();
            return Err(AgentError::Failed(classify_failure(status.code(), &tail)));
        }
        let transcript = req.scratch_dir.join("transcript.md");
        sink.log(LogLevel::Info, &format!("Agent finished in {:.1}s", duration.as_secs_f64())).await;
        Ok(AgentRunOutcome {
            duration,
            transcript_path: transcript.is_file().then_some(transcript),
            cli_version: Some(cli_version),
        })
    }
}

fn unavailable(msg: &str) -> AgentError {
    tracing::error!(%msg, "agent runtime unavailable");
    AgentError::Failed(AppError::new(
        ErrorCode::AgentUnavailable,
        "The agent runtime is not available on the worker. Try again later.",
    ))
}

fn signal_group(pid: Option<i32>, sig: libc::c_int) {
    if let Some(pid) = pid.filter(|p| *p > 0) {
        // SAFETY: killpg only sends a signal; the group id is the child's pid (process_group(0)).
        unsafe {
            libc::killpg(pid, sig);
        }
    }
}

async fn pump<R: AsyncRead + Unpin>(
    reader: Option<R>,
    sink: &dyn ProgressSink,
    redactor: &Redactor,
    tail: &Mutex<VecDeque<String>>,
    stderr: bool,
) {
    let Some(reader) = reader else { return };
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::with_capacity(1024);
    loop {
        buf.clear();
        match read_line_capped(&mut reader, &mut buf, MAX_LINE_BYTES).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let line = redactor.line(&String::from_utf8_lossy(&buf));
        if line.trim().is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        let level = if stderr && (lower.contains("error") || lower.contains("fatal")) {
            LogLevel::Error
        } else if stderr && lower.contains("warn") {
            LogLevel::Warn
        } else {
            LogLevel::Info
        };
        if let Ok(mut t) = tail.lock() {
            if t.len() == TAIL_LINES {
                t.pop_front();
            }
            t.push_back(line.clone());
        }
        sink.log(level, &line).await;
    }
}

/// Reads one line, keeping at most `cap` bytes of it (the rest is discarded). Returns bytes consumed.
async fn read_line_capped<R: AsyncBufRead + Unpin>(r: &mut R, out: &mut Vec<u8>, cap: usize) -> std::io::Result<usize> {
    let mut consumed = 0;
    loop {
        let available = r.fill_buf().await?;
        if available.is_empty() {
            return Ok(consumed);
        }
        let (len, done) = match available.iter().position(|&b| b == b'\n') {
            Some(i) => (i + 1, true),
            None => (available.len(), false),
        };
        let keep = len.min(cap.saturating_sub(out.len()));
        out.extend_from_slice(&available[..keep]);
        r.consume(len);
        consumed += len;
        if done {
            return Ok(consumed);
        }
    }
}

/// Maps a failed run to a stable error code using the CLI's (redacted) output. Exit codes of `copilot -p` are
/// not documented, so this is best effort; anything unrecognised is `agent_failed`.
pub fn classify_failure(exit_code: Option<i32>, tail: &[String]) -> AppError {
    let text = tail.join("\n").to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| text.contains(n));

    let status = |code: &str| text.split(|c: char| !c.is_ascii_alphanumeric()).any(|word| word == code);
    if status("401")
        || has(&[
            "not authorized",
            "unauthorized",
            "authentication failed",
            "no copilot",
            "copilot is not enabled",
            "copilot subscription",
            "copilot access",
            "copilot requests",
            "not been granted",
            "bad credentials",
        ])
    {
        return AppError::new(
            ErrorCode::CopilotUnauthorized,
            "GitHub Copilot rejected your account for this run. Check that you have an active Copilot seat and that \
             your organisation allows Copilot CLI, then sign in again.",
        )
        .retryable(false);
    }
    if text.contains("model")
        && has(&[
            "not available",
            "not supported",
            "unknown model",
            "invalid model",
            "not found",
            "not enabled",
            "unsupported",
        ])
    {
        return AppError::new(
            ErrorCode::ModelUnavailable,
            "The selected model is not available for your Copilot account or organisation. Choose another model.",
        )
        .retryable(false);
    }
    if status("429") || has(&["rate limit", "too many requests", "quota", "premium requests"]) {
        return AppError::new(
            ErrorCode::RateLimited,
            "Copilot rate limit or premium-request quota reached. Try again later or choose another model.",
        )
        .retryable(true);
    }
    if ["502", "503", "504"].iter().any(|c| status(c))
        || has(&["enotfound", "econnreset", "etimedout", "econnrefused", "socket hang up", "network error"])
    {
        return AppError::new(
            ErrorCode::AgentUnavailable,
            "The Copilot service could not be reached. Try again later.",
        );
    }
    match exit_code {
        Some(code) => AppError::new(
            ErrorCode::AgentFailed,
            format!("The agent exited with code {code}. See the log for details."),
        ),
        None => AppError::new(
            ErrorCode::AgentFailed,
            "The agent process was terminated unexpectedly (possibly out of memory). See the log for details.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::adapter::Secret;

    fn bundle(agent_frontmatter: &str) -> (tempfile::TempDir, Arc<AgentBundle>) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("agents")).unwrap();
        std::fs::write(
            dir.path().join("bundle.yaml"),
            r#"
schema_version: 1
agent: { id: bc }
prompt: { template: "Write {{output_path}} for {{owner}}/{{repo}}" }
output: { paths: [docs/bc.md], primary: docs/bc.md }
runtime:
  allow_tools: [read, write]
  deny_tools: [shell, url]
  disable_mcp_servers: [github-mcp-server]
"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("agents/bc.agent.md"), format!("{agent_frontmatter}# agent")).unwrap();
        let b = Arc::new(AgentBundle::load(dir.path()).unwrap());
        (dir, b)
    }

    fn request(scratch: &Path) -> AgentRunRequest {
        AgentRunRequest {
            workspace: scratch.join("ws"),
            scratch_dir: scratch.to_owned(),
            model: Some("claude-haiku-4.5".into()),
            inputs: BTreeMap::new(),
            owner: "acme".into(),
            repo: "shop".into(),
            branch: "main".into(),
            sha: "abc".into(),
            copilot_token: Secret::new("ghu_secretsecretsecret"),
            timeout: Duration::from_secs(60),
        }
    }

    #[test]
    fn builds_least_privilege_argv_without_secrets() {
        let (_d, b) = bundle("");
        let a = CopilotCliAdapter::new(b, "copilot");
        let cl = a.command_line(&request(Path::new("/scratch")));
        let argv = cl.args.join(" ");
        assert!(
            argv.starts_with("-p Write docs/bc.md for acme/shop --agent bc --model claude-haiku-4.5 -s --no-ask-user"),
            "{argv}"
        );
        assert!(argv.contains("--allow-tool read --allow-tool write --deny-tool shell --deny-tool url"));
        assert!(argv.contains("--disable-mcp-server github-mcp-server"));
        assert!(argv.contains("--no-custom-instructions"));
        assert!(argv.contains("--share /scratch/transcript.md"));
        let env: BTreeMap<_, _> = cl.env.iter().cloned().collect();
        assert_eq!(env["COPILOT_HOME"], "/scratch/copilot-home");
        assert_eq!(env["HOME"], "/scratch/home");
        assert!(!env.contains_key("COPILOT_GITHUB_TOKEN"), "token is added only at spawn");
        assert!(!format!("{cl:?}").contains("ghu_"));
    }

    #[test]
    fn pinned_model_suppresses_model_flag() {
        let (_d, b) = bundle("---\nmodel: gpt-5.3-codex\n---\n");
        let cl = CopilotCliAdapter::new(b, "copilot").command_line(&request(Path::new("/s")));
        assert!(!cl.args.contains(&"--model".to_owned()));
    }

    #[test]
    fn classifies_failures() {
        let c =
            |lines: &[&str]| classify_failure(Some(1), &lines.iter().map(|s| s.to_string()).collect::<Vec<_>>()).code;
        assert_eq!(c(&["Error: You are not authorized to use this Copilot feature"]), ErrorCode::CopilotUnauthorized);
        assert_eq!(c(&["Error: Model \"gpt-9\" is not available"]), ErrorCode::ModelUnavailable);
        assert_eq!(c(&["429 Too Many Requests"]), ErrorCode::RateLimited);
        assert_eq!(c(&["getaddrinfo ENOTFOUND api.githubcopilot.com"]), ErrorCode::AgentUnavailable);
        assert_eq!(c(&["something odd"]), ErrorCode::AgentFailed);
        assert_eq!(classify_failure(None, &[]).code, ErrorCode::AgentFailed);
    }

    #[tokio::test]
    async fn capped_lines() {
        let data = format!("{}\nshort\n", "x".repeat(100));
        let mut r = BufReader::new(data.as_bytes());
        let mut buf = Vec::new();
        assert_eq!(read_line_capped(&mut r, &mut buf, 10).await.unwrap(), 101);
        assert_eq!(buf, b"xxxxxxxxxx");
        buf.clear();
        read_line_capped(&mut r, &mut buf, 10).await.unwrap();
        assert_eq!(buf, b"short\n");
    }
}
