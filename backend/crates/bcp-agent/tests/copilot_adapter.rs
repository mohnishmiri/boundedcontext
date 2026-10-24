//! Exercises the real process handling of `CopilotCliAdapter` with shell-script stand-ins for `copilot`.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bcp_agent::{
    AgentAdapter, AgentBundle, AgentError, AgentRunRequest, CopilotCliAdapter, LogLevel, ProgressSink, Secret,
};
use bcp_core::ErrorCode;
use tokio_util::sync::CancellationToken;

const TOKEN: &str = "ghu_testtokentesttokentesttoken01";

#[derive(Default)]
struct Collect(Mutex<Vec<(LogLevel, String)>>);

#[async_trait]
impl ProgressSink for Collect {
    async fn log(&self, level: LogLevel, message: &str) {
        self.0.lock().unwrap().push((level, message.to_owned()));
    }
}

impl Collect {
    fn text(&self) -> String {
        self.0.lock().unwrap().iter().map(|(_, m)| m.as_str()).collect::<Vec<_>>().join("\n")
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    adapter: CopilotCliAdapter,
}

fn fixture(script_body: &str) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_owned();
    let bundle = root.join("bundle");
    std::fs::create_dir_all(bundle.join("agents")).unwrap();
    std::fs::write(
        bundle.join("bundle.yaml"),
        "schema_version: 1\nagent: { id: bc }\nprompt: { template: \"Write {{output_path}}\" }\n\
         output: { paths: [docs/bc.md], primary: docs/bc.md }\n",
    )
    .unwrap();
    std::fs::write(bundle.join("agents/bc.agent.md"), "---\nname: bc\n---\n# agent\n").unwrap();
    let script = root.join("copilot");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 1.0.0-test; exit 0; fi\n{script_body}\n"),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::create_dir_all(root.join("ws")).unwrap();
    std::fs::create_dir_all(root.join("scratch")).unwrap();
    let adapter = CopilotCliAdapter::new(Arc::new(AgentBundle::load(&bundle).unwrap()), script);
    Fixture { _dir: dir, root, adapter }
}

fn request(root: &Path, timeout: Duration) -> AgentRunRequest {
    AgentRunRequest {
        workspace: root.join("ws"),
        scratch_dir: root.join("scratch"),
        model: Some("auto".into()),
        inputs: BTreeMap::new(),
        owner: "acme".into(),
        repo: "shop".into(),
        branch: "main".into(),
        sha: "abc".into(),
        copilot_token: Secret::new(TOKEN),
        timeout,
    }
}

#[tokio::test]
async fn runs_in_workspace_with_scrubbed_env_and_redacted_output() {
    std::env::set_var("BCP_TEST_PARENT_SECRET", "parent-value-should-not-leak");
    let f = fixture(
        r#"
set -e
[ -f "$COPILOT_HOME/agents/bc.agent.md" ] || { echo "agent file missing" >&2; exit 3; }
[ -z "$BCP_TEST_PARENT_SECRET" ] || { echo "env leaked" >&2; exit 4; }
[ "$COPILOT_AUTO_UPDATE" = "false" ] || exit 5
echo "token is $COPILOT_GITHUB_TOKEN"
echo "args: $*"
mkdir -p docs
printf '# Bounded contexts\n' > docs/bc.md
"#,
    );
    let sink = Collect::default();
    let outcome =
        f.adapter.run(request(&f.root, Duration::from_secs(30)), &sink, CancellationToken::new()).await.unwrap();
    assert_eq!(outcome.cli_version.as_deref(), Some("1.0.0-test"));
    assert!(f.root.join("ws/docs/bc.md").is_file());
    let text = sink.text();
    assert!(!text.contains(TOKEN), "token must be redacted: {text}");
    assert!(text.contains("token is [REDACTED]"), "{text}");
    assert!(text.contains("args: -p Write docs/bc.md --agent bc --model auto -s --no-ask-user"), "{text}");
}

#[tokio::test]
async fn timeout_kills_the_whole_process_group() {
    let f = fixture(
        r#"
sleep 60 &
echo $! > "$TMPDIR/child.pid"
echo started
wait
"#,
    );
    let sink = Collect::default();
    let started = Instant::now();
    let err =
        f.adapter.run(request(&f.root, Duration::from_millis(800)), &sink, CancellationToken::new()).await.unwrap_err();
    assert!(matches!(err, AgentError::Timeout(_)), "{err:?}");
    assert_eq!(err.to_app_error().code, ErrorCode::AgentTimeout);
    assert!(started.elapsed() < Duration::from_secs(15));
    let pid: i32 = std::fs::read_to_string(f.root.join("scratch/tmp/child.pid")).unwrap().trim().parse().unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!running(pid), "grandchild {pid} survived the timeout");
}

/// A killed orphan stays a zombie until PID 1 reaps it (containers without an init); zombies count as dead.
fn running(pid: i32) -> bool {
    // SAFETY: signal 0 only checks for existence.
    if unsafe { libc::kill(pid, 0) } != 0 {
        return false;
    }
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => !stat.rsplit(')').next().is_some_and(|rest| rest.trim_start().starts_with('Z')),
        Err(_) => true,
    }
}

#[tokio::test]
async fn cancellation_stops_the_run() {
    let f = fixture("echo working; sleep 60");
    let sink = Collect::default();
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        c2.cancel();
    });
    let err = f.adapter.run(request(&f.root, Duration::from_secs(60)), &sink, cancel).await.unwrap_err();
    assert!(matches!(err, AgentError::Cancelled), "{err:?}");
    assert_eq!(err.to_app_error().code, ErrorCode::Cancelled);
}

#[tokio::test]
async fn failures_are_classified_from_output() {
    let f = fixture("echo 'Error: Model \"gpt-404\" is not available for your account' >&2; exit 1");
    let err = f
        .adapter
        .run(request(&f.root, Duration::from_secs(30)), &Collect::default(), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.to_app_error().code, ErrorCode::ModelUnavailable);

    let f = fixture("echo boom >&2; exit 7");
    let err = f
        .adapter
        .run(request(&f.root, Duration::from_secs(30)), &Collect::default(), CancellationToken::new())
        .await
        .unwrap_err();
    let app = err.to_app_error();
    assert_eq!(app.code, ErrorCode::AgentFailed);
    assert!(app.message.contains("code 7"));
}

#[tokio::test]
async fn missing_binary_is_agent_unavailable() {
    let f = fixture("exit 0");
    let adapter = CopilotCliAdapter::new(Arc::new(f.adapter.bundle().clone()), "/nonexistent/copilot");
    assert_eq!(adapter.preflight().await.unwrap_err().to_app_error().code, ErrorCode::AgentUnavailable);
}
