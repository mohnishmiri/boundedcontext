//! Stateful fake of the parts of GitHub the portal uses: OAuth web flow (with PKCE), user/installation/repo
//! browsing with permission intersection, App JWT + installation tokens, tarballs, Git Data API and pulls.
//! Used by integration tests, the e2e suite and credential-free local development. **Never deploy it.**
//!
//! Test-control endpoints live under `/_mock/` (reset, state dump, fault injection, branch advance).

// Handlers return ready-made error responses; their size is irrelevant for a test double.
#![allow(clippy::result_large_err)]

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query, State as AxState};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use rand::distributions::Alphanumeric;
use rand::Rng;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub type Shared = Arc<Mutex<MockState>>;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Perm {
    pub pull: bool,
    pub push: bool,
    pub admin: bool,
}

impl Perm {
    pub const READ: Perm = Perm { pull: true, push: false, admin: false };
    pub const WRITE: Perm = Perm { pull: true, push: true, admin: false };
}

#[derive(Debug, Clone)]
pub struct MockUser {
    pub id: i64,
    pub login: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MockInstallation {
    pub id: i64,
    pub account: String,
    pub kind: String,
    /// Lower-case `owner/name` of repositories the App can access.
    pub repos: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MockRepo {
    pub id: i64,
    pub owner: String,
    pub name: String,
    pub private: bool,
    pub default_branch: String,
    pub description: Option<String>,
    pub branches: BTreeMap<String, String>,
    pub protected: Vec<String>,
    /// login (lower-case) -> permission
    pub perms: HashMap<String, Perm>,
}

#[derive(Debug, Clone)]
pub struct MockCommit {
    pub sha: String,
    pub tree_sha: String,
    pub message: String,
    pub parents: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MockPull {
    pub number: i64,
    pub repo: String,
    pub head: String,
    pub base: String,
    pub title: String,
    pub body: String,
    pub html_url: String,
    pub state: String,
}

#[derive(Debug, Clone)]
enum TokenKind {
    User { login: String },
    Installation { repo: String, perms: HashMap<String, String> },
}

#[derive(Debug, Clone)]
struct TokenInfo {
    kind: TokenKind,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct OAuthCode {
    login: String,
    challenge: Option<String>,
    redirect_uri: String,
}

#[derive(Debug, Default)]
pub struct MockState {
    pub public_url: String,
    pub users: Vec<MockUser>,
    pub installations: Vec<MockInstallation>,
    pub repos: BTreeMap<String, MockRepo>,
    pub commits: HashMap<String, MockCommit>,
    /// tree sha -> path -> content
    pub trees: HashMap<String, BTreeMap<String, String>>,
    pub pulls: Vec<MockPull>,
    tokens: HashMap<String, TokenInfo>,
    refresh_tokens: HashMap<String, String>,
    codes: HashMap<String, OAuthCode>,
    /// endpoint key -> (status, remaining count)
    faults: HashMap<String, (u16, u32)>,
    /// Seconds user access tokens live (GitHub: 8 h).
    pub access_ttl_secs: i64,
    /// Request counters per endpoint key (assertions in tests).
    pub hits: HashMap<String, u32>,
    next_id: i64,
}

fn rand_str(n: usize) -> String {
    rand::thread_rng().sample_iter(&Alphanumeric).take(n).map(char::from).collect()
}

fn hash40(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update([0]);
    }
    hex::encode(h.finalize())[..40].to_owned()
}

pub fn blob_sha(content: &str) -> String {
    hash40(&["blob", content])
}

fn key(owner: &str, repo: &str) -> String {
    format!("{}/{}", owner.to_ascii_lowercase(), repo.to_ascii_lowercase())
}

impl MockState {
    /// The default fixture: org `acme` (installation 100), user account `octocat` (installation 200).
    pub fn seeded(public_url: &str) -> Self {
        let mut s = MockState {
            public_url: public_url.trim_end_matches('/').to_owned(),
            access_ttl_secs: 8 * 3600,
            next_id: 1000,
            ..Default::default()
        };
        s.users = vec![
            MockUser { id: 1, login: "octocat".into(), name: Some("The Octocat".into()) },
            MockUser { id: 2, login: "viewer".into(), name: Some("Read Only".into()) },
            MockUser { id: 3, login: "outsider".into(), name: None },
        ];
        let shop_files = [
            ("README.md", "# Shop\n\nE-commerce platform with ordering, billing and catalogue services.\n"),
            ("services/orders/src/order.rs", "pub struct Order { pub id: u64, pub lines: Vec<OrderLine> }\npub struct OrderLine { pub sku: String, pub qty: u32 }\n"),
            ("services/orders/README.md", "# Orders\nOwns the order lifecycle: placed, paid, shipped.\n"),
            ("services/billing/src/invoice.rs", "pub struct Invoice { pub order_id: u64, pub total_cents: i64 }\n"),
            ("services/catalog/src/product.rs", "pub struct Product { pub sku: String, pub name: String }\n"),
            ("docs/architecture.md", "# Architecture\nOrders publishes OrderPlaced; Billing consumes it.\n"),
        ];
        s.add_repo(
            "acme",
            "shop",
            "Online shop monorepo",
            &shop_files,
            &[("octocat", Perm::WRITE), ("viewer", Perm::READ)],
        );
        s.add_branch("acme", "shop", "develop", &[("services/shipping/src/shipment.rs", "pub struct Shipment;\n")]);
        s.add_branch(
            "acme",
            "shop",
            "feature/checkout-v2",
            &[("services/orders/src/checkout.rs", "pub fn checkout() {}\n")],
        );
        s.repos.get_mut("acme/shop").unwrap().protected.push("main".into());
        s.add_repo(
            "acme",
            "payments",
            "Payment gateway integration",
            &[("README.md", "# Payments\n"), ("src/gateway.ts", "export class Gateway {}\n")],
            &[("octocat", Perm::WRITE)],
        );
        s.add_repo(
            "acme",
            "readonly-docs",
            "Docs you can read but not change",
            &[("README.md", "# Read-only\n")],
            &[("octocat", Perm::READ), ("viewer", Perm::READ)],
        );
        s.add_repo(
            "acme",
            "legacy-monolith",
            "Makes the fake agent touch files outside the allowlist",
            &[("README.md", "# Legacy\n"), (".fake-copilot-mode", "unexpected\n")],
            &[("octocat", Perm::WRITE)],
        );
        s.add_repo(
            "acme",
            "broken-agent",
            "Makes the fake agent fail",
            &[("README.md", "# Broken\n"), (".fake-copilot-mode", "fail\n")],
            &[("octocat", Perm::WRITE)],
        );
        s.add_repo(
            "acme",
            "not-installed",
            "The App is not installed here",
            &[("README.md", "# Hidden from the App\n")],
            &[("octocat", Perm::WRITE)],
        );
        for i in 1..=35 {
            let name = format!("svc-{i:02}");
            s.add_repo(
                "acme",
                &name,
                "Generated service for pagination",
                &[("README.md", "# Service\n")],
                &[("octocat", Perm::WRITE)],
            );
        }
        s.add_repo(
            "octocat",
            "hello-world",
            "My first repository",
            &[("README.md", "# Hello World\n"), ("main.go", "package main\n")],
            &[("octocat", Perm::WRITE)],
        );

        // Generator repositories (agent bundles). They are readable by `octocat` but belong to no installation,
        // so they never show up in the organisation / repository pickers.
        let lite_bundle = "schema_version: 1\nplaceholder: true\nagent: { id: lite, display_name: Lite generator }\n\
            prompt:\n  template: \"Document {{owner}}/{{repo}}@{{branch}}.{{#scope}} Only {{scope}}.{{/scope}}\"\n\
            inputs:\n  - { name: scope, label: Scope, kind: path, required: false }\n\
            output:\n  paths: [docs/bounded-context.md]\n  primary: docs/bounded-context.md\n";
        s.add_repo(
            "genhub",
            "lite",
            "A minimal generator",
            &[
                ("bundle.yaml", lite_bundle),
                ("agents/lite.agent.md", "---\nname: Lite\ndescription: Lite generator\n---\n# Lite\n"),
            ],
            &[("octocat", Perm::READ)],
        );
        let nested_bundle = "schema_version: 1\nagents_dir: \".\"\nagent: { id: ctx }\n\
            prompt:\n  template: \"Run the {{depth}} analysis for {{repo}}.\"\n\
            inputs:\n  - { name: depth, label: Depth, kind: select, choices: [LEAN, FULL], default: LEAN }\n\
            output:\n  paths: [docs/bounded-context.md]\n  primary: docs/bounded-context.md\n\
            runtime:\n  allow_tools: [read, write, shell]\n  deny_tools: []\n";
        s.add_repo(
            "genhub",
            "monorepo",
            "Generator living in a sub-directory next to its agent files",
            &[
                ("README.md", "# Many generators\n"),
                ("generators/ctx/bundle.yaml", nested_bundle),
                ("generators/ctx/ctx.agent.md", "---\nname: Context analysis\n---\n# Ctx\n"),
            ],
            &[("octocat", Perm::READ)],
        );
        s.add_repo(
            "genhub",
            "no-bundle",
            "Has no bundle.yaml",
            &[("README.md", "# Nothing to run\n")],
            &[("octocat", Perm::READ)],
        );
        let unsafe_bundle = "schema_version: 1\nagent: { id: bad }\nprompt:\n  template: \"Go\"\n\
            output:\n  paths: [\".github/workflows/ci.yml\"]\n";
        s.add_repo(
            "genhub",
            "unsafe",
            "Wants to write CI workflows",
            &[("bundle.yaml", unsafe_bundle), ("agents/bad.agent.md", "# Bad\n")],
            &[("octocat", Perm::READ)],
        );

        let acme_repos: Vec<String> =
            s.repos.keys().filter(|k| k.starts_with("acme/") && *k != "acme/not-installed").cloned().collect();
        s.installations = vec![
            MockInstallation { id: 100, account: "acme".into(), kind: "Organization".into(), repos: acme_repos },
            MockInstallation {
                id: 200,
                account: "octocat".into(),
                kind: "User".into(),
                repos: vec!["octocat/hello-world".into()],
            },
        ];
        s
    }

    fn next_id(&mut self) -> i64 {
        self.next_id += 1;
        self.next_id
    }

    fn store_tree(&mut self, files: BTreeMap<String, String>) -> String {
        let mut parts = vec!["tree".to_owned()];
        for (p, c) in &files {
            parts.push(p.clone());
            parts.push(blob_sha(c));
        }
        let sha = hash40(&parts.iter().map(String::as_str).collect::<Vec<_>>());
        self.trees.insert(sha.clone(), files);
        sha
    }

    fn store_commit(&mut self, tree_sha: &str, message: &str, parents: Vec<String>) -> String {
        let nonce = self.next_id().to_string();
        let mut parts = vec!["commit", tree_sha, message, &nonce];
        let p: Vec<&str> = parents.iter().map(String::as_str).collect();
        parts.extend(p);
        let sha = hash40(&parts);
        self.commits.insert(
            sha.clone(),
            MockCommit { sha: sha.clone(), tree_sha: tree_sha.to_owned(), message: message.to_owned(), parents },
        );
        sha
    }

    pub fn add_repo(
        &mut self,
        owner: &str,
        name: &str,
        description: &str,
        files: &[(&str, &str)],
        perms: &[(&str, Perm)],
    ) {
        let tree = self.store_tree(files.iter().map(|(p, c)| ((*p).to_owned(), (*c).to_owned())).collect());
        let sha = self.store_commit(&tree, "Initial commit", vec![]);
        let id = self.next_id();
        self.repos.insert(
            key(owner, name),
            MockRepo {
                id,
                owner: owner.to_owned(),
                name: name.to_owned(),
                private: true,
                default_branch: "main".into(),
                description: Some(description.to_owned()),
                branches: BTreeMap::from([("main".to_owned(), sha)]),
                protected: vec![],
                perms: perms.iter().map(|(l, p)| (l.to_ascii_lowercase(), *p)).collect(),
            },
        );
    }

    /// Creates `branch` from the default branch with extra files.
    pub fn add_branch(&mut self, owner: &str, name: &str, branch: &str, files: &[(&str, &str)]) {
        let repo = self.repos[&key(owner, name)].clone();
        let base = repo.branches[&repo.default_branch].clone();
        let sha = self.commit_files(&base, files, &format!("Work on {branch}"));
        self.repos.get_mut(&key(owner, name)).unwrap().branches.insert(branch.to_owned(), sha);
    }

    fn commit_files(&mut self, parent: &str, files: &[(&str, &str)], message: &str) -> String {
        let mut tree = self.trees[&self.commits[parent].tree_sha].clone();
        for (p, c) in files {
            tree.insert((*p).to_owned(), (*c).to_owned());
        }
        let tree_sha = self.store_tree(tree);
        self.store_commit(&tree_sha, message, vec![parent.to_owned()])
    }

    /// Adds a commit on top of `branch` (simulates someone pushing after the analysis).
    pub fn advance_branch(&mut self, owner: &str, name: &str, branch: &str, files: &[(&str, &str)]) -> String {
        let head = self.repos[&key(owner, name)].branches[branch].clone();
        let sha = self.commit_files(&head, files, "Concurrent change");
        self.repos.get_mut(&key(owner, name)).unwrap().branches.insert(branch.to_owned(), sha.clone());
        sha
    }

    pub fn branch_head(&self, owner: &str, name: &str, branch: &str) -> Option<String> {
        self.repos.get(&key(owner, name))?.branches.get(branch).cloned()
    }

    pub fn file_at(&self, sha: &str, path: &str) -> Option<String> {
        self.trees.get(&self.commits.get(sha)?.tree_sha)?.get(path).cloned()
    }

    pub fn commit(&self, sha: &str) -> Option<&MockCommit> {
        self.commits.get(sha)
    }

    pub fn pulls_for(&self, owner: &str, name: &str) -> Vec<MockPull> {
        self.pulls.iter().filter(|p| p.repo == key(owner, name)).cloned().collect()
    }

    /// Issues a user access token directly (skips the OAuth dance in tests).
    pub fn issue_user_token(&mut self, login: &str) -> String {
        let token = format!("ghu_{}", rand_str(36));
        let expires_at = Utc::now() + Duration::seconds(self.access_ttl_secs);
        self.tokens.insert(token.clone(), TokenInfo { kind: TokenKind::User { login: login.to_owned() }, expires_at });
        token
    }

    /// Makes `endpoint` answer `status` for the next `count` requests.
    pub fn fail_next(&mut self, endpoint: &str, status: u16, count: u32) {
        self.faults.insert(endpoint.to_owned(), (status, count));
    }

    pub fn expire_user_tokens(&mut self, login: &str) {
        for t in self.tokens.values_mut() {
            if matches!(&t.kind, TokenKind::User { login: l } if l.eq_ignore_ascii_case(login)) {
                t.expires_at = Utc::now() - Duration::seconds(1);
            }
        }
    }

    pub fn revoke_user_tokens(&mut self, login: &str) {
        self.tokens.retain(|_, t| !matches!(&t.kind, TokenKind::User { login: l } if l.eq_ignore_ascii_case(login)));
        self.refresh_tokens.retain(|_, l| !l.eq_ignore_ascii_case(login));
    }

    pub fn set_perm(&mut self, owner: &str, name: &str, login: &str, perm: Perm) {
        self.repos.get_mut(&key(owner, name)).unwrap().perms.insert(login.to_ascii_lowercase(), perm);
    }

    fn hit(&mut self, endpoint: &str) -> Option<Response> {
        *self.hits.entry(endpoint.to_owned()).or_default() += 1;
        let (status, remaining) = self.faults.get_mut(endpoint)?;
        if *remaining == 0 {
            return None;
        }
        *remaining -= 1;
        let status = StatusCode::from_u16(*status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut resp = err(status, "injected fault");
        if status == StatusCode::FORBIDDEN || status == StatusCode::TOO_MANY_REQUESTS {
            resp.headers_mut().insert("retry-after", "1".parse().unwrap());
        }
        Some(resp)
    }

    fn repo_json(&self, repo: &MockRepo, login: Option<&str>) -> Value {
        let mut v = json!({
            "id": repo.id,
            "name": repo.name,
            "full_name": format!("{}/{}", repo.owner, repo.name),
            "owner": { "login": repo.owner },
            "private": repo.private,
            "default_branch": repo.default_branch,
            "html_url": format!("{}/{}/{}", self.public_url, repo.owner, repo.name),
            "description": repo.description,
            "updated_at": "2026-10-01T12:00:00Z",
            "archived": false,
            "size": 120,
        });
        if let Some(l) = login {
            let p = repo.perms.get(&l.to_ascii_lowercase()).copied().unwrap_or_default();
            v["permissions"] = json!({ "pull": p.pull, "push": p.push, "admin": p.admin });
        }
        v
    }
}

#[derive(Clone)]
struct AppState {
    shared: Shared,
}

fn err(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "message": message, "documentation_url": "https://docs.github.com/rest" }))).into_response()
}

enum Caller {
    User(String),
    Installation { repo: String, perms: HashMap<String, String> },
    App,
}

fn caller(state: &MockState, headers: &HeaderMap) -> Result<Caller, Response> {
    let auth = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).unwrap_or_default();
    let token = auth.strip_prefix("Bearer ").or_else(|| auth.strip_prefix("token ")).unwrap_or_default().trim();
    if token.starts_with("eyJ") && token.split('.').count() == 3 {
        return Ok(Caller::App);
    }
    let info = state.tokens.get(token).ok_or_else(|| err(StatusCode::UNAUTHORIZED, "Bad credentials"))?;
    if info.expires_at < Utc::now() {
        return Err(err(StatusCode::UNAUTHORIZED, "Bad credentials"));
    }
    Ok(match &info.kind {
        TokenKind::User { login } => Caller::User(login.clone()),
        TokenKind::Installation { repo, perms } => Caller::Installation { repo: repo.clone(), perms: perms.clone() },
    })
}

fn user_perm(repo: &MockRepo, login: &str) -> Perm {
    repo.perms.get(&login.to_ascii_lowercase()).copied().unwrap_or_default()
}

/// Resolves the repository if the caller may read it (404 otherwise, like GitHub).
fn readable<'a>(s: &'a MockState, c: &Caller, owner: &str, name: &str) -> Result<&'a MockRepo, Response> {
    let k = key(owner, name);
    let repo = s.repos.get(&k).ok_or_else(|| err(StatusCode::NOT_FOUND, "Not Found"))?;
    let ok = match c {
        Caller::User(login) => user_perm(repo, login).pull,
        Caller::Installation { repo: r, .. } => *r == k,
        Caller::App => false,
    };
    if ok {
        Ok(repo)
    } else {
        Err(err(StatusCode::NOT_FOUND, "Not Found"))
    }
}

fn writable(s: &MockState, c: &Caller, owner: &str, name: &str, permission: &str) -> Result<(), Response> {
    readable(s, c, owner, name)?;
    let ok = match c {
        Caller::User(login) => user_perm(&s.repos[&key(owner, name)], login).push,
        Caller::Installation { perms, .. } => perms.get(permission).is_some_and(|v| v == "write"),
        Caller::App => false,
    };
    if ok {
        Ok(())
    } else {
        Err(err(StatusCode::FORBIDDEN, "Resource not accessible by integration"))
    }
}

#[derive(Deserialize)]
struct PageQuery {
    page: Option<usize>,
    per_page: Option<usize>,
}

fn paginate<T: Clone>(items: &[T], q: &PageQuery, base: &str) -> (Vec<T>, Option<String>) {
    let per_page = q.per_page.unwrap_or(30).clamp(1, 100);
    let page = q.page.unwrap_or(1).max(1);
    let start = (page - 1) * per_page;
    let slice = items.iter().skip(start).take(per_page).cloned().collect();
    let next = (start + per_page < items.len()).then(|| {
        let sep = if base.contains('?') { '&' } else { '?' };
        format!("<{base}{sep}per_page={per_page}&page={}>; rel=\"next\"", page + 1)
    });
    (slice, next)
}

/// Base URL for API links (pagination) as the caller reached us; may differ from the browser-facing URL.
fn api_base(headers: &HeaderMap, s: &MockState) -> String {
    match headers.get(header::HOST).and_then(|h| h.to_str().ok()) {
        Some(host) => format!("http://{host}"),
        None => s.public_url.clone(),
    }
}

fn with_link(body: Value, link: Option<String>) -> Response {
    let mut resp = Json(body).into_response();
    if let Some(l) = link {
        resp.headers_mut().insert("link", l.parse().unwrap());
    }
    resp
}

macro_rules! lock {
    ($st:expr, $endpoint:expr) => {{
        let mut guard = $st.shared.lock().unwrap();
        if let Some(resp) = guard.hit($endpoint) {
            return resp;
        }
        guard
    }};
}

macro_rules! tri {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(resp) => return resp,
        }
    };
}

// ------------------------------------------------------------------------------------------------------
// OAuth
// ------------------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct AuthorizeQuery {
    redirect_uri: String,
    state: String,
    code_challenge: Option<String>,
    login: Option<String>,
}

async fn authorize(AxState(st): AxState<AppState>, Query(q): Query<AuthorizeQuery>) -> Response {
    let mut s = lock!(st, "oauth_authorize");
    let Some(login) = q.login.clone() else {
        // Developer-facing account chooser.
        let mut links = String::new();
        for u in &s.users {
            let mut url = url::Url::parse("http://x/login/oauth/authorize").unwrap();
            url.query_pairs_mut()
                .append_pair("redirect_uri", &q.redirect_uri)
                .append_pair("state", &q.state)
                .append_pair("code_challenge", q.code_challenge.as_deref().unwrap_or_default())
                .append_pair("login", &u.login);
            links.push_str(&format!(
                "<li><a href=\"/login/oauth/authorize?{}\">Sign in as <b>{}</b></a></li>",
                html_escape(url.query().unwrap_or_default()),
                html_escape(&u.login)
            ));
        }
        return Html(format!(
            "<!doctype html><title>Mock GitHub</title><body style=\"font-family:sans-serif\"><h1>Mock GitHub sign-in</h1>\
             <p>Local development only. Choose an account:</p><ul>{links}</ul></body>"
        ))
        .into_response();
    };
    if !s.users.iter().any(|u| u.login == login) {
        return err(StatusCode::NOT_FOUND, "unknown mock user");
    }
    let code = rand_str(20);
    s.codes.insert(
        code.clone(),
        OAuthCode {
            login,
            challenge: q.code_challenge.filter(|c| !c.is_empty()),
            redirect_uri: q.redirect_uri.clone(),
        },
    );
    let mut url = match url::Url::parse(&q.redirect_uri) {
        Ok(u) => u,
        Err(_) => return err(StatusCode::BAD_REQUEST, "bad redirect_uri"),
    };
    url.query_pairs_mut().append_pair("code", &code).append_pair("state", &q.state);
    Redirect::to(url.as_str()).into_response()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[derive(Deserialize)]
struct TokenForm {
    code: Option<String>,
    redirect_uri: Option<String>,
    code_verifier: Option<String>,
    grant_type: Option<String>,
    refresh_token: Option<String>,
}

async fn access_token(AxState(st): AxState<AppState>, Form(f): Form<TokenForm>) -> Response {
    let mut s = lock!(st, "oauth_token");
    let oauth_err = |e: &str| Json(json!({ "error": e, "error_description": e })).into_response();
    let login = if f.grant_type.as_deref() == Some("refresh_token") {
        match f.refresh_token.as_deref().and_then(|r| s.refresh_tokens.remove(r)) {
            Some(l) => l,
            None => return oauth_err("bad_refresh_token"),
        }
    } else {
        let Some(code) = f.code.as_deref().and_then(|c| s.codes.remove(c)) else {
            return oauth_err("bad_verification_code");
        };
        if f.redirect_uri.as_deref() != Some(code.redirect_uri.as_str()) {
            return oauth_err("redirect_uri_mismatch");
        }
        if let Some(challenge) = &code.challenge {
            let verifier = f.code_verifier.unwrap_or_default();
            if URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) != *challenge {
                return oauth_err("invalid_grant");
            }
        }
        code.login
    };
    let access = s.issue_user_token(&login);
    let refresh = format!("ghr_{}", rand_str(60));
    s.refresh_tokens.insert(refresh.clone(), login);
    Json(json!({
        "access_token": access,
        "expires_in": s.access_ttl_secs,
        "refresh_token": refresh,
        "refresh_token_expires_in": 15_811_200,
        "token_type": "bearer",
        "scope": ""
    }))
    .into_response()
}

async fn install_page(Path(slug): Path<String>) -> Html<String> {
    Html(format!(
        "<!doctype html><title>Install</title><p>Mock: the App <b>{}</b> would be installed here.</p>",
        html_escape(&slug)
    ))
}

// ------------------------------------------------------------------------------------------------------
// REST API
// ------------------------------------------------------------------------------------------------------

async fn get_user(AxState(st): AxState<AppState>, headers: HeaderMap) -> Response {
    let s = lock!(st, "user");
    let Caller::User(login) = tri!(caller(&s, &headers)) else {
        return err(StatusCode::FORBIDDEN, "user token required");
    };
    let u = s.users.iter().find(|u| u.login == login).cloned().unwrap();
    Json(json!({ "id": u.id, "login": u.login, "name": u.name, "avatar_url": null })).into_response()
}

fn visible_repos<'a>(
    s: &'a MockState,
    inst: &'a MockInstallation,
    login: &'a str,
) -> impl Iterator<Item = &'a MockRepo> + 'a {
    inst.repos.iter().filter_map(move |k| s.repos.get(k)).filter(move |r| user_perm(r, login).pull)
}

async fn user_installations(
    AxState(st): AxState<AppState>,
    headers: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Response {
    let s = lock!(st, "user_installations");
    let Caller::User(login) = tri!(caller(&s, &headers)) else {
        return err(StatusCode::FORBIDDEN, "user token required");
    };
    let items: Vec<Value> = s
        .installations
        .iter()
        .filter(|i| visible_repos(&s, i, &login).next().is_some())
        .map(|i| json!({ "id": i.id, "account": { "login": i.account, "type": i.kind, "avatar_url": null } }))
        .collect();
    let (page, link) = paginate(&items, &q, &format!("{}/user/installations", api_base(&headers, &s)));
    with_link(json!({ "total_count": items.len(), "installations": page }), link)
}

async fn installation_repos(
    AxState(st): AxState<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Query(q): Query<PageQuery>,
) -> Response {
    let s = lock!(st, "installation_repositories");
    let Caller::User(login) = tri!(caller(&s, &headers)) else {
        return err(StatusCode::FORBIDDEN, "user token required");
    };
    let Some(inst) = s.installations.iter().find(|i| i.id == id) else {
        return err(StatusCode::NOT_FOUND, "Not Found");
    };
    let items: Vec<Value> = visible_repos(&s, inst, &login).map(|r| s.repo_json(r, Some(&login))).collect();
    if items.is_empty() {
        return err(StatusCode::NOT_FOUND, "Not Found");
    }
    let (page, link) =
        paginate(&items, &q, &format!("{}/user/installations/{id}/repositories", api_base(&headers, &s)));
    with_link(json!({ "total_count": items.len(), "repositories": page }), link)
}

async fn get_repo(
    AxState(st): AxState<AppState>,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
) -> Response {
    let s = lock!(st, "repo");
    let c = tri!(caller(&s, &headers));
    let repo = tri!(readable(&s, &c, &owner, &name));
    let login = match &c {
        Caller::User(l) => Some(l.as_str()),
        _ => None,
    };
    Json(s.repo_json(repo, login)).into_response()
}

fn branch_json(repo: &MockRepo, name: &str, sha: &str) -> Value {
    json!({ "name": name, "commit": { "sha": sha }, "protected": repo.protected.iter().any(|p| p == name) })
}

async fn list_branches(
    AxState(st): AxState<AppState>,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
    Query(q): Query<PageQuery>,
) -> Response {
    let s = lock!(st, "branches");
    let c = tri!(caller(&s, &headers));
    let repo = tri!(readable(&s, &c, &owner, &name));
    let items: Vec<Value> = repo.branches.iter().map(|(b, sha)| branch_json(repo, b, sha)).collect();
    let (page, link) = paginate(&items, &q, &format!("{}/repos/{owner}/{name}/branches", api_base(&headers, &s)));
    with_link(Value::Array(page), link)
}

async fn get_branch(
    AxState(st): AxState<AppState>,
    headers: HeaderMap,
    Path((owner, name, branch)): Path<(String, String, String)>,
) -> Response {
    let s = lock!(st, "branch");
    let c = tri!(caller(&s, &headers));
    let repo = tri!(readable(&s, &c, &owner, &name));
    match repo.branches.get(&branch) {
        Some(sha) => Json(branch_json(repo, &branch, sha)).into_response(),
        None => err(StatusCode::NOT_FOUND, "Branch not found"),
    }
}

async fn repo_installation(
    AxState(st): AxState<AppState>,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
) -> Response {
    let s = lock!(st, "repo_installation");
    if !matches!(tri!(caller(&s, &headers)), Caller::App) {
        return err(StatusCode::UNAUTHORIZED, "A JSON web token could not be decoded");
    }
    let k = key(&owner, &name);
    match s.installations.iter().find(|i| i.repos.contains(&k)) {
        Some(i) => Json(json!({ "id": i.id, "account": { "login": i.account, "type": i.kind } })).into_response(),
        None => err(StatusCode::NOT_FOUND, "Not Found"),
    }
}

#[derive(Deserialize)]
struct TokenReq {
    repositories: Vec<String>,
    #[serde(default)]
    permissions: HashMap<String, String>,
}

async fn installation_token(
    AxState(st): AxState<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<TokenReq>,
) -> Response {
    let mut s = lock!(st, "installation_token");
    if !matches!(tri!(caller(&s, &headers)), Caller::App) {
        return err(StatusCode::UNAUTHORIZED, "A JSON web token could not be decoded");
    }
    let Some(inst) = s.installations.iter().find(|i| i.id == id).cloned() else {
        return err(StatusCode::NOT_FOUND, "Not Found");
    };
    if body.repositories.len() != 1 {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "mock supports exactly one repository");
    }
    let k = key(&inst.account, &body.repositories[0]);
    if !inst.repos.contains(&k) {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "There is at least one repository that does not exist or is not accessible",
        );
    }
    let token = format!("ghs_{}", rand_str(36));
    let expires_at = Utc::now() + Duration::hours(1);
    s.tokens.insert(
        token.clone(),
        TokenInfo { kind: TokenKind::Installation { repo: k, perms: body.permissions }, expires_at },
    );
    (StatusCode::CREATED, Json(json!({ "token": token, "expires_at": expires_at.to_rfc3339() }))).into_response()
}

fn resolve_ref(repo: &MockRepo, s: &MockState, git_ref: &str) -> Option<String> {
    if let Some(sha) = repo.branches.get(git_ref) {
        return Some(sha.clone());
    }
    s.commits.contains_key(git_ref).then(|| git_ref.to_owned())
}

async fn tarball(
    AxState(st): AxState<AppState>,
    headers: HeaderMap,
    Path((owner, name, git_ref)): Path<(String, String, String)>,
) -> Response {
    let s = lock!(st, "tarball");
    let c = tri!(caller(&s, &headers));
    let repo = tri!(readable(&s, &c, &owner, &name));
    let Some(sha) = resolve_ref(repo, &s, &git_ref) else { return err(StatusCode::NOT_FOUND, "Not Found") };
    let files = s.trees[&s.commits[&sha].tree_sha].clone();
    let top = format!("{}-{}-{}", repo.owner, repo.name, &sha[..7]);
    drop(s);
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
    let mut dir = tar::Header::new_gnu();
    dir.set_entry_type(tar::EntryType::Directory);
    dir.set_mode(0o755);
    dir.set_size(0);
    builder.append_data(&mut dir, format!("{top}/"), std::io::empty()).unwrap();
    for (path, content) in files {
        let mut h = tar::Header::new_gnu();
        h.set_mode(0o644);
        h.set_size(content.len() as u64);
        builder.append_data(&mut h, format!("{top}/{path}"), content.as_bytes()).unwrap();
    }
    let bytes = builder.into_inner().unwrap().finish().unwrap();
    ([(header::CONTENT_TYPE, "application/x-gzip")], bytes).into_response()
}

async fn get_ref(
    AxState(st): AxState<AppState>,
    headers: HeaderMap,
    Path((owner, name, git_ref)): Path<(String, String, String)>,
) -> Response {
    let s = lock!(st, "git_ref");
    let c = tri!(caller(&s, &headers));
    let repo = tri!(readable(&s, &c, &owner, &name));
    let Some(branch) = git_ref.strip_prefix("heads/") else { return err(StatusCode::NOT_FOUND, "Not Found") };
    match repo.branches.get(branch) {
        Some(sha) => Json(json!({ "ref": format!("refs/heads/{branch}"), "object": { "sha": sha, "type": "commit" } }))
            .into_response(),
        None => err(StatusCode::NOT_FOUND, "Not Found"),
    }
}

fn commit_json(s: &MockState, owner: &str, name: &str, c: &MockCommit) -> Value {
    json!({
        "sha": c.sha,
        "tree": { "sha": c.tree_sha },
        "message": c.message,
        "parents": c.parents.iter().map(|p| json!({ "sha": p })).collect::<Vec<_>>(),
        "html_url": format!("{}/{owner}/{name}/commit/{}", s.public_url, c.sha),
    })
}

async fn get_commit(
    AxState(st): AxState<AppState>,
    headers: HeaderMap,
    Path((owner, name, sha)): Path<(String, String, String)>,
) -> Response {
    let s = lock!(st, "git_commit");
    let c = tri!(caller(&s, &headers));
    tri!(readable(&s, &c, &owner, &name));
    match s.commits.get(&sha) {
        Some(commit) => Json(commit_json(&s, &owner, &name, commit)).into_response(),
        None => err(StatusCode::NOT_FOUND, "Not Found"),
    }
}

#[derive(Deserialize)]
struct RefQuery {
    #[serde(rename = "ref")]
    git_ref: Option<String>,
}

async fn get_contents(
    AxState(st): AxState<AppState>,
    headers: HeaderMap,
    Path((owner, name, path)): Path<(String, String, String)>,
    Query(q): Query<RefQuery>,
) -> Response {
    let s = lock!(st, "contents");
    let c = tri!(caller(&s, &headers));
    let repo = tri!(readable(&s, &c, &owner, &name));
    let git_ref = q.git_ref.unwrap_or_else(|| repo.default_branch.clone());
    let Some(sha) = resolve_ref(repo, &s, &git_ref) else {
        return err(StatusCode::NOT_FOUND, "No commit found for the ref");
    };
    match s.file_at(&sha, &path) {
        Some(content) => {
            Json(json!({ "type": "file", "path": path, "sha": blob_sha(&content), "size": content.len() }))
                .into_response()
        }
        None => err(StatusCode::NOT_FOUND, "Not Found"),
    }
}

#[derive(Deserialize)]
struct TreeEntry {
    path: String,
    content: Option<String>,
}

#[derive(Deserialize)]
struct TreeReq {
    base_tree: Option<String>,
    tree: Vec<TreeEntry>,
}

async fn create_tree(
    AxState(st): AxState<AppState>,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
    Json(body): Json<TreeReq>,
) -> Response {
    let mut s = lock!(st, "create_tree");
    let c = tri!(caller(&s, &headers));
    tri!(writable(&s, &c, &owner, &name, "contents"));
    let mut files = match &body.base_tree {
        Some(b) => match s.trees.get(b) {
            Some(t) => t.clone(),
            None => return err(StatusCode::UNPROCESSABLE_ENTITY, "base_tree is not a valid tree"),
        },
        None => BTreeMap::new(),
    };
    for e in body.tree {
        match e.content {
            Some(c) => {
                files.insert(e.path, c);
            }
            None => return err(StatusCode::UNPROCESSABLE_ENTITY, "mock supports inline content only"),
        }
    }
    let sha = s.store_tree(files);
    (StatusCode::CREATED, Json(json!({ "sha": sha }))).into_response()
}

#[derive(Deserialize)]
struct CommitReq {
    message: String,
    tree: String,
    parents: Vec<String>,
}

async fn create_commit(
    AxState(st): AxState<AppState>,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
    Json(body): Json<CommitReq>,
) -> Response {
    let mut s = lock!(st, "create_commit");
    let c = tri!(caller(&s, &headers));
    tri!(writable(&s, &c, &owner, &name, "contents"));
    if !s.trees.contains_key(&body.tree) || body.parents.iter().any(|p| !s.commits.contains_key(p)) {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "tree or parent not found");
    }
    let sha = s.store_commit(&body.tree, &body.message, body.parents);
    let commit = s.commits[&sha].clone();
    (StatusCode::CREATED, Json(commit_json(&s, &owner, &name, &commit))).into_response()
}

#[derive(Deserialize)]
struct RefReq {
    #[serde(rename = "ref")]
    name: String,
    sha: String,
}

async fn create_ref(
    AxState(st): AxState<AppState>,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
    Json(body): Json<RefReq>,
) -> Response {
    let mut s = lock!(st, "create_ref");
    let c = tri!(caller(&s, &headers));
    tri!(writable(&s, &c, &owner, &name, "contents"));
    let Some(branch) = body.name.strip_prefix("refs/heads/").map(str::to_owned) else {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "Reference name must start with refs/heads/");
    };
    if !s.commits.contains_key(&body.sha) {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "Object does not exist");
    }
    let repo = s.repos.get_mut(&key(&owner, &name)).unwrap();
    if repo.branches.contains_key(&branch) {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "Reference already exists");
    }
    repo.branches.insert(branch.clone(), body.sha.clone());
    (StatusCode::CREATED, Json(json!({ "ref": body.name, "object": { "sha": body.sha, "type": "commit" } })))
        .into_response()
}

fn pull_json(p: &MockPull, owner: &str) -> Value {
    json!({
        "number": p.number, "html_url": p.html_url, "state": p.state, "title": p.title, "body": p.body,
        "head": { "ref": p.head, "label": format!("{owner}:{}", p.head) },
        "base": { "ref": p.base, "label": format!("{owner}:{}", p.base) },
    })
}

#[derive(Deserialize)]
struct PullsQuery {
    state: Option<String>,
    head: Option<String>,
    base: Option<String>,
}

async fn list_pulls(
    AxState(st): AxState<AppState>,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
    Query(q): Query<PullsQuery>,
) -> Response {
    let s = lock!(st, "pulls");
    let c = tri!(caller(&s, &headers));
    tri!(readable(&s, &c, &owner, &name));
    let k = key(&owner, &name);
    let head = q.head.as_deref().map(|h| h.split_once(':').map(|(_, b)| b).unwrap_or(h).to_owned());
    let items: Vec<Value> = s
        .pulls
        .iter()
        .filter(|p| p.repo == k)
        .filter(|p| q.state.as_deref().is_none_or(|st| st == "all" || st == p.state))
        .filter(|p| head.as_deref().is_none_or(|h| h == p.head))
        .filter(|p| q.base.as_deref().is_none_or(|b| b == p.base))
        .map(|p| pull_json(p, &owner))
        .collect();
    Json(Value::Array(items)).into_response()
}

#[derive(Deserialize)]
struct PullReq {
    title: String,
    #[serde(default)]
    body: String,
    head: String,
    base: String,
}

async fn create_pull(
    AxState(st): AxState<AppState>,
    headers: HeaderMap,
    Path((owner, name)): Path<(String, String)>,
    Json(body): Json<PullReq>,
) -> Response {
    let mut s = lock!(st, "create_pull");
    let c = tri!(caller(&s, &headers));
    tri!(writable(&s, &c, &owner, &name, "pull_requests"));
    let k = key(&owner, &name);
    let repo = &s.repos[&k];
    if !repo.branches.contains_key(&body.head) || !repo.branches.contains_key(&body.base) {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "Validation Failed: head or base does not exist");
    }
    if s.pulls.iter().any(|p| p.repo == k && p.head == body.head && p.base == body.base && p.state == "open") {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "A pull request already exists");
    }
    let number = s.pulls.iter().filter(|p| p.repo == k).count() as i64 + 1;
    let pull = MockPull {
        number,
        repo: k,
        head: body.head,
        base: body.base,
        title: body.title,
        body: body.body,
        html_url: format!("{}/{owner}/{name}/pull/{number}", s.public_url),
        state: "open".into(),
    };
    let json = pull_json(&pull, &owner);
    s.pulls.push(pull);
    (StatusCode::CREATED, Json(json)).into_response()
}

// ------------------------------------------------------------------------------------------------------
// Test control
// ------------------------------------------------------------------------------------------------------

async fn mock_reset(AxState(st): AxState<AppState>) -> StatusCode {
    let mut s = st.shared.lock().unwrap();
    let url = s.public_url.clone();
    *s = MockState::seeded(&url);
    StatusCode::NO_CONTENT
}

async fn mock_state(AxState(st): AxState<AppState>) -> Json<Value> {
    let s = st.shared.lock().unwrap();
    let repos: BTreeMap<_, _> = s.repos.iter().map(|(k, r)| (k.clone(), json!({ "branches": r.branches }))).collect();
    Json(json!({ "repos": repos, "pulls": s.pulls, "hits": s.hits }))
}

#[derive(Deserialize)]
struct FaultReq {
    endpoint: String,
    status: u16,
    #[serde(default = "one")]
    count: u32,
}

fn one() -> u32 {
    1
}

async fn mock_fault(AxState(st): AxState<AppState>, Json(f): Json<FaultReq>) -> StatusCode {
    st.shared.lock().unwrap().fail_next(&f.endpoint, f.status, f.count);
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
struct AdvanceReq {
    branch: String,
    path: String,
    content: String,
}

async fn mock_advance(
    AxState(st): AxState<AppState>,
    Path((owner, name)): Path<(String, String)>,
    Json(a): Json<AdvanceReq>,
) -> Response {
    let mut s = st.shared.lock().unwrap();
    if s.branch_head(&owner, &name, &a.branch).is_none() {
        return err(StatusCode::NOT_FOUND, "no such branch");
    }
    let sha = s.advance_branch(&owner, &name, &a.branch, &[(&a.path, &a.content)]);
    Json(json!({ "sha": sha })).into_response()
}

#[derive(Deserialize)]
struct LoginReq {
    login: String,
}

async fn mock_expire(AxState(st): AxState<AppState>, Json(l): Json<LoginReq>) -> StatusCode {
    st.shared.lock().unwrap().expire_user_tokens(&l.login);
    StatusCode::NO_CONTENT
}

/// Stand-in for the Copilot API's model listing: what the calling user's token may use.
async fn copilot_models(AxState(st): AxState<AppState>, headers: HeaderMap) -> Response {
    let s = lock!(st, "copilot_models");
    let Caller::User(login) = tri!(caller(&s, &headers)) else {
        return err(StatusCode::FORBIDDEN, "user token required");
    };
    let model = |id: &str, name: &str, vendor: &str, kind: &str, policy: &str| {
        json!({
            "id": id, "name": name, "vendor": vendor, "model_picker_enabled": true,
            "capabilities": { "type": kind }, "policy": { "state": policy }, "billing": { "multiplier": 1.0 },
        })
    };
    let data = if login == "viewer" {
        vec![model("gpt-5-mini", "GPT-5 mini", "OpenAI", "chat", "enabled")]
    } else {
        vec![
            model("gpt-5", "GPT-5", "OpenAI", "chat", "enabled"),
            model("claude-sonnet-4.5", "Claude Sonnet 4.5", "Anthropic", "chat", "enabled"),
            model("text-embedding-3-small", "Embedding", "OpenAI", "embeddings", "enabled"),
            model("retired-model", "Retired", "OpenAI", "chat", "disabled"),
        ]
    };
    Json(json!({ "object": "list", "data": data })).into_response()
}

pub fn router(shared: Shared) -> Router {
    let st = AppState { shared };
    Router::new()
        .route("/login/oauth/authorize", get(authorize))
        .route("/login/oauth/access_token", post(access_token))
        .route("/apps/{slug}/installations/new", get(install_page))
        .route("/user", get(get_user))
        .route("/user/installations", get(user_installations))
        .route("/user/installations/{id}/repositories", get(installation_repos))
        .route("/app/installations/{id}/access_tokens", post(installation_token))
        .route("/repos/{owner}/{repo}", get(get_repo))
        .route("/repos/{owner}/{repo}/installation", get(repo_installation))
        .route("/repos/{owner}/{repo}/branches", get(list_branches))
        .route("/repos/{owner}/{repo}/branches/{*branch}", get(get_branch))
        .route("/repos/{owner}/{repo}/tarball/{*git_ref}", get(tarball))
        .route("/repos/{owner}/{repo}/git/ref/{*git_ref}", get(get_ref))
        .route("/repos/{owner}/{repo}/git/commits/{sha}", get(get_commit))
        .route("/repos/{owner}/{repo}/git/commits", post(create_commit))
        .route("/repos/{owner}/{repo}/git/trees", post(create_tree))
        .route("/repos/{owner}/{repo}/git/refs", post(create_ref))
        .route("/repos/{owner}/{repo}/contents/{*path}", get(get_contents))
        .route("/repos/{owner}/{repo}/pulls", get(list_pulls).post(create_pull))
        .route("/copilot/models", get(copilot_models))
        .route("/_mock/reset", post(mock_reset))
        .route("/_mock/state", get(mock_state))
        .route("/_mock/faults", post(mock_fault))
        .route("/_mock/repos/{owner}/{repo}/advance", post(mock_advance))
        .route("/_mock/expire-tokens", post(mock_expire))
        .with_state(st)
}

/// A running mock bound to an ephemeral local port.
pub struct MockGitHub {
    pub url: String,
    pub state: Shared,
    handle: tokio::task::JoinHandle<()>,
}

impl MockGitHub {
    pub async fn start() -> Self {
        Self::start_on("127.0.0.1:0".parse().unwrap(), None).await
    }

    /// `public_url` defaults to the bound address.
    pub async fn start_on(addr: SocketAddr, public_url: Option<String>) -> Self {
        let listener = tokio::net::TcpListener::bind(addr).await.expect("bind mock GitHub");
        let local = listener.local_addr().unwrap();
        let url = public_url.unwrap_or_else(|| format!("http://{local}"));
        let state = Arc::new(Mutex::new(MockState::seeded(&url)));
        let app = router(state.clone());
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("mock GitHub server");
        });
        Self { url: format!("http://{local}"), state, handle }
    }

    pub fn with<R>(&self, f: impl FnOnce(&mut MockState) -> R) -> R {
        f(&mut self.state.lock().unwrap())
    }
}

impl Drop for MockGitHub {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Throwaway RSA private key (PEM) for signing App JWTs against the mock. Generated with `openssl` once per
/// process so no key material is committed.
pub fn test_app_key() -> String {
    static KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    KEY.get_or_init(|| {
        let out = std::process::Command::new("openssl")
            .args(["genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048"])
            .output()
            .expect("openssl is required to generate a throwaway GitHub App key for tests");
        assert!(out.status.success(), "openssl genpkey failed");
        String::from_utf8(out.stdout).expect("PEM is UTF-8")
    })
    .clone()
}
