//! Shared harness: a fresh PostgreSQL database, the mock GitHub, the real API router and (optionally) a real
//! worker whose agent runtime is a shell script that behaves according to `.fake-copilot-mode` in the repo.

#![allow(dead_code)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use base64::Engine;
use bcp_agent::{AgentBundle, CopilotCliAdapter};
use bcp_api::{ApiConfig, AppState};
use bcp_core::config::{CommonConfig, GitHubSettings};
use bcp_core::crypto::TokenCipher;
use bcp_github::GitHubClient;
use bcp_store::Store;
use bcp_worker::{AdapterFactory, Mode, Worker, WorkerConfig};
use mock_github::MockGitHub;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use url::Url;
use uuid::Uuid;

pub const PUBLIC_URL: &str = "http://portal.test";

/// Stand-in for `copilot`: honours `.fake-copilot-mode` (ok | fail | slow | unexpected | empty | model).
const SCRIPT: &str = r#"#!/bin/sh
if [ "$1" = "--version" ]; then echo 9.9.9-test; exit 0; fi
mode=$(cat .fake-copilot-mode 2>/dev/null || echo ok)
echo "analysing repository with token $COPILOT_GITHUB_TOKEN"
case "$mode" in
  fail*) echo "Error: simulated agent failure" >&2; exit 1 ;;
  model*) echo "Error: Model is not available for your account" >&2; exit 1 ;;
  slow*) echo "thinking slowly"; sleep 120 ;;
esac
mkdir -p docs
if [ "$mode" = "empty" ]; then : > docs/bounded-context.md; exit 0; fi
cat > docs/bounded-context.md <<'DOC'
# Bounded Contexts

## Summary
Orders, Billing and Catalog are separate bounded contexts.

## Context Map
```mermaid
flowchart LR
  Orders --> Billing
```
DOC
case "$mode" in unexpected*) echo "tampered" > README.md ;; esac
echo "done"
"#;

/// Test registry: two curated generators from the mock's `genhub` repositories; users may add `genhub/*`.
const GENERATORS_YAML: &str = r#"
curated:
  - { id: lite, name: Lite (curated), description: A minimal generator, owner: genhub, repo: lite }
  - { id: ctx, name: Context analysis, owner: genhub, repo: monorepo, path: generators/ctx }
allow: ["genhub/*"]
max_user_generators: 2
"#;

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().unwrap()
}

pub async fn fresh_database() -> Option<String> {
    let Ok(base) = std::env::var("BCP_TEST_DATABASE_URL") else {
        assert!(std::env::var("BCP_TEST_REQUIRE_DB").is_err(), "BCP_TEST_DATABASE_URL is required");
        eprintln!("SKIPPED: BCP_TEST_DATABASE_URL not set");
        return None;
    };
    let admin = sqlx::PgPool::connect(&base).await.expect("connect admin");
    let name = format!("bcp_t_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}")).execute(&admin).await.unwrap();
    let (prefix, _) = base.rsplit_once('/').unwrap();
    Some(format!("{prefix}/{name}"))
}

pub struct Harness {
    pub mock: MockGitHub,
    pub store: Store,
    pub app: Router,
    pub state: AppState,
    pub common: CommonConfig,
    pub tmp: tempfile::TempDir,
    shutdown: CancellationToken,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

pub struct Response {
    pub status: StatusCode,
    pub headers: axum::http::HeaderMap,
    pub body: Value,
    pub text: String,
}

impl Harness {
    pub async fn new() -> Option<Self> {
        let db = fresh_database().await?;
        let mock = MockGitHub::start().await;
        mock.with(|state| {
            state.add_repo(
                "ATT-DP1",
                "apm0014313-ragaflow-ai-context-docs",
                "central bounded-context documentation",
                &[("README.md", "# Context index\n")],
                &[("octocat", mock_github::Perm::WRITE)],
            );
            state.installations.push(mock_github::MockInstallation {
                id: 300,
                account: "ATT-DP1".into(),
                kind: "Organization".into(),
                repos: vec!["att-dp1/apm0014313-ragaflow-ai-context-docs".into()],
            });
        });
        let url = Url::parse(&mock.url).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let tmp_work = tmp.path().join("api-work");
        let generators_file = tmp.path().join("generators.yaml");
        std::fs::write(&generators_file, GENERATORS_YAML).unwrap();
        let common = CommonConfig {
            database_url: db,
            database_max_connections: 8,
            public_url: Url::parse(PUBLIC_URL).unwrap(),
            github: GitHubSettings {
                api_url: url.clone(),
                web_url: url.clone(),
                oauth_url: url,
                app_id: "4242".into(),
                app_slug: "bcp-test".into(),
                client_id: "Iv1.test".into(),
                client_secret: "test-secret".into(),
                private_key_pem: mock_github::test_app_key(),
            },
            cipher: TokenCipher::from_base64(&base64::engine::general_purpose::STANDARD.encode([9u8; 32])).unwrap(),
            agent_bundle_dir: repo_root().join("backend/tools/integration-tests/fixtures/bundle"),
            models_file: repo_root().join("config/models.yaml"),
            models_show_unverified: false,
            central_repository: Some(("ATT-DP1".into(), "apm0014313-ragaflow-ai-context-docs".into())),
        };
        let cfg = ApiConfig {
            common: common.clone(),
            listen: "127.0.0.1:0".parse().unwrap(),
            metrics_listen: "127.0.0.1:0".parse().unwrap(),
            session_ttl: Duration::from_secs(3600),
            cookie_secure: false,
            rate_limit_per_minute: 0,
            job_create_limit_per_hour: 50,
            run_migrations: true,
            job_retention_days: 90,
            generators_file,
            work_dir: tmp_work.clone(),
            copilot_api_url: Some(Url::parse(&format!("{}/copilot", mock.url)).unwrap()),
            copilot_integration_id: "copilot-developer-cli".into(),
            batch_max_repos: 25,
            copilot_shared_token: false,
        };
        let state = bcp_api::build_state(cfg).await.unwrap();
        let store = state.store.clone();
        let shutdown = CancellationToken::new();
        tokio::spawn(bcp_api::sse::run_notifier(state.clone(), shutdown.clone()));
        let app = bcp_api::router(state.clone());
        Some(Self { mock, store, app, state, common, tmp, shutdown })
    }

    /// Starts a real worker in loop mode with the scripted agent runtime.
    pub async fn start_worker(&self, generate_attempts: i32) -> Arc<Worker> {
        self.start_worker_with(generate_attempts, |_| {}).await
    }

    pub async fn start_worker_with(
        &self,
        generate_attempts: i32,
        tweak: impl FnOnce(&mut WorkerConfig),
    ) -> Arc<Worker> {
        let script = self.tmp.path().join("copilot");
        std::fs::write(&script, SCRIPT).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut cfg = WorkerConfig {
            common: self.common.clone(),
            listen: "127.0.0.1:0".parse().unwrap(),
            metrics_listen: "127.0.0.1:0".parse().unwrap(),
            mode: Mode::Loop,
            concurrency: 2,
            worker_id: format!("test-worker-{}", Uuid::new_v4().simple()),
            work_dir: self.tmp.path().join("work"),
            copilot_bin: script.clone(),
            copilot_token: None,
            max_job_timeout: Duration::from_secs(60),
            max_repo_bytes: 50 * 1024 * 1024,
            max_repo_files: 10_000,
            lease_secs: 15,
            generate_max_attempts: generate_attempts,
            publish_max_attempts: 2,
            shutdown_grace: Duration::from_secs(5),
            idle_exit: Duration::from_secs(1),
            max_log_lines: 1000,
        };
        tweak(&mut cfg);
        let bundle = Arc::new(AgentBundle::load(&self.common.agent_bundle_dir).unwrap());
        let adapters: AdapterFactory = Arc::new(move |b| Arc::new(CopilotCliAdapter::new(b, script.clone())));
        let github = GitHubClient::new(&self.common.github).unwrap();
        let worker = Arc::new(Worker::new(cfg, self.store.clone(), github, bundle, adapters));
        assert!(worker.preflight().await);
        tokio::spawn(worker.clone().run_loop(self.shutdown.clone()));
        worker
    }

    pub async fn call(&self, method: &str, path: &str, cookie: Option<&str>, body: Option<Value>) -> Response {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        if method != "GET" {
            req = req.header("x-requested-with", "bcp").header(header::ORIGIN, PUBLIC_URL);
        }
        let req = match body {
            Some(b) => req.header(header::CONTENT_TYPE, "application/json").body(Body::from(b.to_string())).unwrap(),
            None => req.body(Body::empty()).unwrap(),
        };
        self.send(req).await
    }

    pub async fn send(&self, req: Request<Body>) -> Response {
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        let bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024 * 1024).await.unwrap();
        let text = String::from_utf8_lossy(&bytes).to_string();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        Response { status, headers, body, text }
    }

    pub async fn get(&self, path: &str, cookie: &str) -> Response {
        self.call("GET", path, Some(cookie), None).await
    }

    pub async fn post(&self, path: &str, cookie: &str, body: Option<Value>) -> Response {
        let body = if path.ends_with("/pull-request") && body.is_none() {
            Some(serde_json::json!({ "title": "APM-123: Update bounded context", "publish_central": false }))
        } else {
            body
        };
        self.call("POST", path, Some(cookie), body).await
    }

    /// Full OAuth round trip against the mock; returns the `Cookie` header value for the session.
    pub async fn login(&self, login: &str) -> String {
        let start = self.call("GET", "/api/auth/github?return_to=/jobs/new", None, None).await;
        assert_eq!(start.status, StatusCode::SEE_OTHER, "{}", start.text);
        let location = start.headers[header::LOCATION].to_str().unwrap().to_owned();
        let oauth_cookie = set_cookie(&start.headers, "bcp_oauth").expect("oauth state cookie");
        let authorize = format!("{location}&login={login}");
        let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
        let r = client.get(&authorize).send().await.unwrap();
        assert_eq!(r.status().as_u16(), 303, "mock authorize");
        let callback = Url::parse(r.headers()["location"].to_str().unwrap()).unwrap();
        assert!(callback.as_str().starts_with(&format!("{PUBLIC_URL}/api/auth/github/callback")));
        let path = format!("{}?{}", callback.path(), callback.query().unwrap());
        let done = self.call("GET", &path, Some(&format!("bcp_oauth={oauth_cookie}")), None).await;
        assert_eq!(done.status, StatusCode::SEE_OTHER, "{}", done.text);
        assert_eq!(done.headers[header::LOCATION], "/jobs/new");
        let session = set_cookie(&done.headers, "bcp_session").expect("session cookie");
        format!("bcp_session={session}")
    }

    /// Polls a job until `pred` holds (or panics after `secs`).
    pub async fn wait_job(&self, cookie: &str, id: &str, secs: u64, pred: impl Fn(&Value) -> bool) -> Value {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
        loop {
            let r = self.get(&format!("/api/bounded-context/jobs/{id}"), cookie).await;
            assert_eq!(r.status, StatusCode::OK, "{}", r.text);
            if pred(&r.body) {
                return r.body;
            }
            if tokio::time::Instant::now() > deadline {
                let logs = self.get(&format!("/api/bounded-context/jobs/{id}/logs"), cookie).await;
                panic!("timed out waiting for job {id}: {}\nlogs: {}", r.body, logs.body);
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    pub async fn create_job(&self, cookie: &str, owner: &str, repo: &str, branch: &str) -> Response {
        self.post(
            "/api/bounded-context/jobs",
            cookie,
            Some(serde_json::json!({ "owner": owner, "repo": repo, "base_branch": branch, "model": "auto" })),
        )
        .await
    }

    /// Adds a repository to the `acme` installation with the given files.
    pub fn add_acme_repo(&self, name: &str, files: &[(&str, &str)]) {
        self.mock.with(|s| {
            s.add_repo("acme", name, "test fixture", files, &[("octocat", mock_github::Perm::WRITE)]);
            s.installations.iter_mut().find(|i| i.account == "acme").unwrap().repos.push(format!("acme/{name}"));
        });
    }
}

pub fn set_cookie(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, v)| *k == name && !v.is_empty())
        .map(|(_, v)| v.to_owned())
}

pub fn terminal(job: &Value) -> bool {
    matches!(job["status"].as_str(), Some("succeeded" | "failed" | "cancelled"))
}

pub fn publish_terminal(job: &Value) -> bool {
    matches!(job["publish"]["status"].as_str(), Some("succeeded" | "failed"))
}
