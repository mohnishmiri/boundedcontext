//! Thin GitHub REST client for the portal: GitHub App JWT and installation tokens, the OAuth web flow with
//! PKCE and token refresh, repository browsing with the user's token, and the Git Data / Pulls APIs used to
//! publish generated documentation. Idempotent GETs are retried with backoff; writes are never retried here
//! (the worker's resumable publish pipeline owns that).

mod error;
mod types;

use std::path::Path;
use std::time::{Duration, Instant};

use bcp_core::config::GitHubSettings;
use chrono::Utc;
use futures::StreamExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use rand::Rng;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use reqwest::{Method, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::json;
use tokio::io::AsyncWriteExt;
use url::Url;

pub use error::GitHubError;
pub use types::*;

pub type Result<T, E = GitHubError> = std::result::Result<T, E>;

/// Characters escaped inside one URL path segment.
const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'[')
    .add(b']')
    .add(b'\\')
    .add(b'^')
    .add(b'|');

/// Encodes each `/`-separated part of a path (branch names, file paths) and keeps the separators.
pub fn encode_path(path: &str) -> String {
    path.split('/').map(|p| utf8_percent_encode(p, SEGMENT).to_string()).collect::<Vec<_>>().join("/")
}

fn seg(s: &str) -> String {
    utf8_percent_encode(s, SEGMENT).to_string()
}

const MAX_GET_ATTEMPTS: u32 = 3;
const MAX_INLINE_RETRY_WAIT: u64 = 10;

#[derive(Clone)]
pub struct GitHubClient {
    http: reqwest::Client,
    api_url: Url,
    web_url: Url,
    oauth_url: Url,
    app_id: String,
    client_id: String,
    client_secret: String,
    app_key: Option<EncodingKey>,
}

impl std::fmt::Debug for GitHubClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubClient").field("api_url", &self.api_url.as_str()).finish_non_exhaustive()
    }
}

/// Which credential a request uses; decides how errors are surfaced.
#[derive(Clone, Copy)]
enum Auth<'a> {
    Token(&'a str),
    AppJwt,
}

impl GitHubClient {
    pub fn new(settings: &GitHubSettings) -> anyhow::Result<Self> {
        let app_key = if !settings.app_id.is_empty() && !settings.private_key_pem.is_empty() {
            Some(
                EncodingKey::from_rsa_pem(settings.private_key_pem.as_bytes())
                    .map_err(|e| anyhow::anyhow!("GitHub App private key is not a valid RSA PEM: {e}"))?,
            )
        } else {
            None
        };
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static("bounded-context-portal"));
        headers.insert("X-GitHub-Api-Version", HeaderValue::from_static("2022-11-28"));
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            // reqwest drops Authorization when a redirect leaves the original host (tarball -> codeload).
            .redirect(reqwest::redirect::Policy::limited(5))
            .https_only(false)
            .build()?;
        Ok(Self {
            http,
            api_url: ensure_trailing_slash(settings.api_url.clone()),
            web_url: ensure_trailing_slash(settings.web_url.clone()),
            oauth_url: ensure_trailing_slash(settings.oauth_url.clone()),
            app_id: settings.app_id.clone(),
            client_id: settings.client_id.clone(),
            client_secret: settings.client_secret.clone(),
            app_key,
        })
    }

    pub fn has_app(&self) -> bool {
        self.app_key.is_some()
    }

    pub fn web_url(&self) -> &Url {
        &self.web_url
    }

    fn api(&self, path: &str) -> Url {
        self.api_url.join(path.trim_start_matches('/')).expect("valid API path")
    }

    /// Short-lived (9 min) RS256 JWT identifying the GitHub App.
    pub fn app_jwt(&self) -> Result<String> {
        #[derive(Serialize)]
        struct Claims<'a> {
            iat: i64,
            exp: i64,
            iss: &'a str,
        }
        let app_key =
            self.app_key.as_ref().ok_or_else(|| GitHubError::Io("GitHub App private key is not configured".into()))?;
        let now = Utc::now().timestamp();
        let claims = Claims { iat: now - 60, exp: now + 540, iss: &self.app_id };
        jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, app_key)
            .map_err(|e| GitHubError::Io(format!("signing App JWT: {e}")))
    }

    fn request(&self, method: Method, url: Url, auth: Auth<'_>) -> Result<RequestBuilder> {
        let mut rb = self.http.request(method, url).header(ACCEPT, "application/vnd.github+json");
        match auth {
            Auth::Token(t) => rb = rb.header(AUTHORIZATION, format!("Bearer {t}")),
            Auth::AppJwt => rb = rb.header(AUTHORIZATION, format!("Bearer {}", self.app_jwt()?)),
        }
        Ok(rb)
    }

    /// Sends a request, retrying idempotent methods on transient failures.
    async fn send(
        &self,
        endpoint: &'static str,
        method: Method,
        url: Url,
        auth: Auth<'_>,
        body: Option<&serde_json::Value>,
    ) -> Result<Response> {
        let idempotent = method == Method::GET || method == Method::HEAD;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut rb = self.request(method.clone(), url.clone(), auth)?;
            if let Some(b) = body {
                rb = rb.json(b);
            }
            let started = Instant::now();
            let result = match rb.send().await {
                Ok(resp) => check_status(resp).await,
                Err(e) => Err(GitHubError::Network(redact_reqwest(&e))),
            };
            let status = match &result {
                Ok(r) => r.status().as_u16().to_string(),
                Err(GitHubError::Network(_)) => "network".to_owned(),
                Err(e) => status_label(e),
            };
            metrics::counter!("bcp_github_requests_total", "endpoint" => endpoint, "status" => status).increment(1);
            metrics::histogram!("bcp_github_request_seconds", "endpoint" => endpoint)
                .record(started.elapsed().as_secs_f64());
            match result {
                Err(e) if idempotent && e.is_transient() && attempt < MAX_GET_ATTEMPTS => {
                    let wait = match &e {
                        GitHubError::RateLimited { retry_after: Some(s) } if *s > MAX_INLINE_RETRY_WAIT => {
                            return Err(e)
                        }
                        GitHubError::RateLimited { retry_after: Some(s) } => Duration::from_secs(*s),
                        _ => backoff(attempt),
                    };
                    tracing::warn!(endpoint, attempt, error = %e, wait_ms = wait.as_millis() as u64, "retrying GitHub request");
                    tokio::time::sleep(wait).await;
                }
                other => return other,
            }
        }
    }

    async fn get_json<T: DeserializeOwned>(&self, endpoint: &'static str, url: Url, auth: Auth<'_>) -> Result<T> {
        decode(self.send(endpoint, Method::GET, url, auth, None).await?).await
    }

    async fn send_json<T: DeserializeOwned>(
        &self,
        endpoint: &'static str,
        method: Method,
        url: Url,
        auth: Auth<'_>,
        body: serde_json::Value,
    ) -> Result<T> {
        decode(self.send(endpoint, method, url, auth, Some(&body)).await?).await
    }

    /// Follows `Link: rel="next"` up to `max_pages`, collecting items via `extract`.
    async fn get_paginated<T, P, F>(
        &self,
        endpoint: &'static str,
        first: Url,
        auth: Auth<'_>,
        max_pages: usize,
        extract: F,
    ) -> Result<(Vec<T>, bool)>
    where
        P: DeserializeOwned,
        F: Fn(P) -> Vec<T>,
    {
        let mut out = Vec::new();
        let mut next = Some(first);
        let mut pages = 0;
        while let Some(url) = next.take() {
            let resp = self.send(endpoint, Method::GET, url, auth, None).await?;
            next = next_link(resp.headers()).and_then(|u| self.same_api_host(u));
            let page: P = decode(resp).await?;
            out.extend(extract(page));
            pages += 1;
            if pages >= max_pages {
                break;
            }
        }
        Ok((out, next.is_some()))
    }

    /// Only follow pagination links pointing back at the configured API host (SSRF hygiene).
    fn same_api_host(&self, url: Url) -> Option<Url> {
        (url.origin() == self.api_url.origin()).then_some(url)
    }

    // ------------------------------------------------------------------------------------------------
    // OAuth (GitHub App user-to-server tokens)
    // ------------------------------------------------------------------------------------------------

    pub fn authorize_url(&self, redirect_uri: &str, state: &str, code_challenge: &str) -> Url {
        let mut url = self.web_url.join("login/oauth/authorize").expect("valid URL");
        {
            let mut qp = url.query_pairs_mut();
            qp.append_pair("client_id", &self.client_id)
                .append_pair("redirect_uri", redirect_uri)
                .append_pair("state", state)
                .append_pair("code_challenge", code_challenge)
                .append_pair("code_challenge_method", "S256")
                .append_pair("allow_signup", "false");
            if !self.has_app() && !self.client_id.starts_with("Iv") {
                qp.append_pair("scope", "repo read:org");
            }
        }
        url
    }

    pub fn install_url(&self, app_slug: &str) -> String {
        self.web_url.join(&format!("apps/{}/installations/new", seg(app_slug))).expect("valid URL").to_string()
    }

    async fn token_request(&self, form: &[(&str, &str)]) -> Result<TokenResponse> {
        let url = self.oauth_url.join("login/oauth/access_token").expect("valid URL");
        let started = Instant::now();
        let resp = self
            .http
            .post(url)
            .header(ACCEPT, "application/json")
            .form(form)
            .send()
            .await
            .map_err(|e| GitHubError::Network(redact_reqwest(&e)))?;
        metrics::histogram!("bcp_github_request_seconds", "endpoint" => "oauth_token")
            .record(started.elapsed().as_secs_f64());
        let resp = check_status(resp).await?;
        // GitHub reports OAuth failures as 200 with an `error` field.
        let value: serde_json::Value = decode(resp).await?;
        if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
            return Err(GitHubError::OAuth {
                error: err.to_owned(),
                description: value.get("error_description").and_then(|v| v.as_str()).map(str::to_owned),
            });
        }
        serde_json::from_value(value).map_err(|e| GitHubError::Decode(e.to_string()))
    }

    pub async fn exchange_code(&self, code: &str, redirect_uri: &str, code_verifier: &str) -> Result<TokenResponse> {
        self.token_request(&[
            ("client_id", &self.client_id),
            ("client_secret", &self.client_secret),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", code_verifier),
        ])
        .await
    }

    pub async fn refresh_user_token(&self, refresh_token: &str) -> Result<TokenResponse> {
        self.token_request(&[
            ("client_id", &self.client_id),
            ("client_secret", &self.client_secret),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ])
        .await
    }

    // ------------------------------------------------------------------------------------------------
    // User-token calls (what the signed-in user can see, intersected with App installations)
    // ------------------------------------------------------------------------------------------------

    pub async fn get_authenticated_user(&self, token: &str) -> Result<GhUser> {
        self.get_json("user", self.api("user"), Auth::Token(token)).await
    }

    pub async fn list_user_installations(&self, token: &str) -> Result<Vec<GhInstallation>> {
        #[derive(serde::Deserialize)]
        struct Page {
            installations: Vec<GhInstallation>,
        }
        let mut url = self.api("user/installations");
        url.query_pairs_mut().append_pair("per_page", "100");
        let (items, _) =
            self.get_paginated("user_installations", url, Auth::Token(token), 10, |p: Page| p.installations).await?;
        Ok(items)
    }

    /// Repositories of one installation the user can access. Returns `(repos, truncated)`.
    pub async fn list_installation_repositories(
        &self,
        token: &str,
        installation_id: i64,
        max_pages: usize,
    ) -> Result<(Vec<GhRepo>, bool)> {
        #[derive(serde::Deserialize)]
        struct Page {
            repositories: Vec<GhRepo>,
        }
        let mut url = self.api(&format!("user/installations/{installation_id}/repositories"));
        url.query_pairs_mut().append_pair("per_page", "100");
        self.get_paginated("installation_repositories", url, Auth::Token(token), max_pages, |p: Page| p.repositories)
            .await
    }

    pub async fn list_user_orgs(&self, token: &str) -> Result<Vec<GhOrg>> {
        let mut url = self.api("user/orgs");
        url.query_pairs_mut().append_pair("per_page", "100");
        let (items, _) = self.get_paginated("user_orgs", url, Auth::Token(token), 10, |p: Vec<GhOrg>| p).await?;
        Ok(items)
    }

    /// Repositories accessible to the authenticated user (personal, collaborator, and organization repos).
    pub async fn list_user_repos(&self, token: &str, max_pages: usize) -> Result<(Vec<GhRepo>, bool)> {
        let mut url = self.api("user/repos");
        url.query_pairs_mut()
            .append_pair("per_page", "100")
            .append_pair("sort", "updated")
            .append_pair("affiliation", "owner,collaborator,organization_member");
        self.get_paginated("user_repos", url, Auth::Token(token), max_pages, |p: Vec<GhRepo>| p).await
    }

    pub async fn get_repository(&self, token: &str, owner: &str, repo: &str) -> Result<GhRepo> {
        self.get_json("repo", self.api(&format!("repos/{}/{}", seg(owner), seg(repo))), Auth::Token(token)).await
    }

    pub async fn list_branches(
        &self,
        token: &str,
        owner: &str,
        repo: &str,
        max_pages: usize,
    ) -> Result<(Vec<GhBranch>, bool)> {
        let mut url = self.api(&format!("repos/{}/{}/branches", seg(owner), seg(repo)));
        url.query_pairs_mut().append_pair("per_page", "100");
        self.get_paginated("branches", url, Auth::Token(token), max_pages, |p: Vec<GhBranch>| p).await
    }

    pub async fn get_branch(&self, token: &str, owner: &str, repo: &str, branch: &str) -> Result<GhBranch> {
        let url = self.api(&format!("repos/{}/{}/branches/{}", seg(owner), seg(repo), encode_path(branch)));
        self.get_json("branch", url, Auth::Token(token)).await
    }

    // ------------------------------------------------------------------------------------------------
    // App calls
    // ------------------------------------------------------------------------------------------------

    pub async fn get_repository_installation(&self, owner: &str, repo: &str) -> Result<GhInstallation> {
        let url = self.api(&format!("repos/{}/{}/installation", seg(owner), seg(repo)));
        self.get_json("repo_installation", url, Auth::AppJwt).await
    }

    /// Installation token restricted to one repository and the given permissions, e.g.
    /// `[("contents", "read")]`.
    pub async fn create_installation_token(
        &self,
        installation_id: i64,
        repo: &str,
        permissions: &[(&str, &str)],
    ) -> Result<InstallationToken> {
        let perms: serde_json::Map<String, serde_json::Value> =
            permissions.iter().map(|(k, v)| ((*k).to_owned(), json!(v))).collect();
        let url = self.api(&format!("app/installations/{installation_id}/access_tokens"));
        self.send_json(
            "installation_token",
            Method::POST,
            url,
            Auth::AppJwt,
            json!({ "repositories": [repo], "permissions": perms }),
        )
        .await
    }

    // ------------------------------------------------------------------------------------------------
    // Installation-token calls (repository content and publishing)
    // ------------------------------------------------------------------------------------------------

    /// Streams the gzip tarball of `git_ref` into `dest`, failing with `TooLarge` beyond `max_bytes`.
    pub async fn download_tarball(
        &self,
        token: &str,
        owner: &str,
        repo: &str,
        git_ref: &str,
        dest: &Path,
        max_bytes: u64,
    ) -> Result<u64> {
        let url = self.api(&format!("repos/{}/{}/tarball/{}", seg(owner), seg(repo), encode_path(git_ref)));
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.download_once(url.clone(), token, dest, max_bytes).await {
                Err(e) if e.is_transient() && attempt < MAX_GET_ATTEMPTS => {
                    tracing::warn!(attempt, error = %e, "retrying tarball download");
                    tokio::time::sleep(backoff(attempt)).await;
                }
                other => return other,
            }
        }
    }

    async fn download_once(&self, url: Url, token: &str, dest: &Path, max_bytes: u64) -> Result<u64> {
        let started = Instant::now();
        let resp = self
            .request(Method::GET, url, Auth::Token(token))?
            .timeout(Duration::from_secs(600))
            .send()
            .await
            .map_err(|e| GitHubError::Network(redact_reqwest(&e)))?;
        let resp = check_status(resp).await?;
        if resp.content_length().is_some_and(|len| len > max_bytes) {
            return Err(GitHubError::TooLarge);
        }
        let mut file = tokio::fs::File::create(dest).await.map_err(|e| GitHubError::Io(e.to_string()))?;
        let mut stream = resp.bytes_stream();
        let mut total: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| GitHubError::Network(redact_reqwest(&e)))?;
            total += chunk.len() as u64;
            if total > max_bytes {
                return Err(GitHubError::TooLarge);
            }
            file.write_all(&chunk).await.map_err(|e| GitHubError::Io(e.to_string()))?;
        }
        file.flush().await.map_err(|e| GitHubError::Io(e.to_string()))?;
        metrics::histogram!("bcp_github_request_seconds", "endpoint" => "tarball")
            .record(started.elapsed().as_secs_f64());
        Ok(total)
    }

    /// Head commit of a branch, or `None` if the branch does not exist.
    pub async fn get_branch_head(&self, token: &str, owner: &str, repo: &str, branch: &str) -> Result<Option<String>> {
        let url = self.api(&format!("repos/{}/{}/git/ref/heads/{}", seg(owner), seg(repo), encode_path(branch)));
        match self.get_json::<GhRef>("git_ref", url, Auth::Token(token)).await {
            Ok(r) => Ok(Some(r.object.sha)),
            Err(GitHubError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn get_git_commit(&self, token: &str, owner: &str, repo: &str, sha: &str) -> Result<GhCommit> {
        let url = self.api(&format!("repos/{}/{}/git/commits/{}", seg(owner), seg(repo), seg(sha)));
        self.get_json("git_commit", url, Auth::Token(token)).await
    }

    /// Blob SHA of a file at a ref, or `None` when the file does not exist there.
    pub async fn get_file_sha(
        &self,
        token: &str,
        owner: &str,
        repo: &str,
        path: &str,
        git_ref: &str,
    ) -> Result<Option<String>> {
        let mut url = self.api(&format!("repos/{}/{}/contents/{}", seg(owner), seg(repo), encode_path(path)));
        url.query_pairs_mut().append_pair("ref", git_ref);
        match self.send("contents", Method::GET, url, Auth::Token(token), None).await {
            Ok(resp) => {
                // A directory listing comes back as an array: treat as "not a file".
                let value: serde_json::Value = decode(resp).await?;
                if value.is_array() {
                    return Ok(None);
                }
                let content: GhContent =
                    serde_json::from_value(value).map_err(|e| GitHubError::Decode(e.to_string()))?;
                Ok((content.kind == "file").then_some(content.sha))
            }
            Err(GitHubError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn create_tree(
        &self,
        token: &str,
        owner: &str,
        repo: &str,
        base_tree: &str,
        files: &[TreeFile],
    ) -> Result<String> {
        let entries: Vec<_> = files
            .iter()
            .map(|f| json!({ "path": f.path, "mode": "100644", "type": "blob", "content": f.content }))
            .collect();
        let url = self.api(&format!("repos/{}/{}/git/trees", seg(owner), seg(repo)));
        let resp: GhSha = self
            .send_json(
                "git_tree",
                Method::POST,
                url,
                Auth::Token(token),
                json!({ "base_tree": base_tree, "tree": entries }),
            )
            .await?;
        Ok(resp.sha)
    }

    pub async fn create_commit(
        &self,
        token: &str,
        owner: &str,
        repo: &str,
        message: &str,
        tree: &str,
        parents: &[&str],
    ) -> Result<GhCommit> {
        let url = self.api(&format!("repos/{}/{}/git/commits", seg(owner), seg(repo)));
        self.send_json(
            "git_commit_create",
            Method::POST,
            url,
            Auth::Token(token),
            json!({ "message": message, "tree": tree, "parents": parents }),
        )
        .await
    }

    /// Creates `refs/heads/<branch>`. An existing ref yields `Unprocessable`.
    pub async fn create_branch(&self, token: &str, owner: &str, repo: &str, branch: &str, sha: &str) -> Result<()> {
        let url = self.api(&format!("repos/{}/{}/git/refs", seg(owner), seg(repo)));
        let _: GhRef = self
            .send_json(
                "git_ref_create",
                Method::POST,
                url,
                Auth::Token(token),
                json!({ "ref": format!("refs/heads/{branch}"), "sha": sha }),
            )
            .await?;
        Ok(())
    }

    pub async fn find_open_pull(
        &self,
        token: &str,
        owner: &str,
        repo: &str,
        head_branch: &str,
        base: &str,
    ) -> Result<Option<GhPull>> {
        let mut url = self.api(&format!("repos/{}/{}/pulls", seg(owner), seg(repo)));
        url.query_pairs_mut()
            .append_pair("state", "open")
            .append_pair("head", &format!("{owner}:{head_branch}"))
            .append_pair("base", base)
            .append_pair("per_page", "10");
        let pulls: Vec<GhPull> = self.get_json("pulls", url, Auth::Token(token)).await?;
        Ok(pulls.into_iter().find(|p| p.head.name == head_branch && p.base.name == base))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_pull(
        &self,
        token: &str,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
    ) -> Result<GhPull> {
        let url = self.api(&format!("repos/{}/{}/pulls", seg(owner), seg(repo)));
        self.send_json(
            "pulls_create",
            Method::POST,
            url,
            Auth::Token(token),
            json!({ "title": title, "body": body, "head": head, "base": base, "maintainer_can_modify": false }),
        )
        .await
    }
}

fn ensure_trailing_slash(mut url: Url) -> Url {
    if !url.path().ends_with('/') {
        let p = format!("{}/", url.path());
        url.set_path(&p);
    }
    url
}

fn backoff(attempt: u32) -> Duration {
    let base = 250u64 * 2u64.pow(attempt.saturating_sub(1).min(6));
    Duration::from_millis(base + rand::thread_rng().gen_range(0..=base / 2))
}

fn status_label(e: &GitHubError) -> String {
    match e {
        GitHubError::Unauthorized => "401".into(),
        GitHubError::Forbidden { .. } => "403".into(),
        GitHubError::RateLimited { .. } => "429".into(),
        GitHubError::NotFound => "404".into(),
        GitHubError::Unprocessable { .. } => "422".into(),
        GitHubError::Conflict { .. } => "409".into(),
        GitHubError::Server { status } | GitHubError::Unexpected { status } => status.to_string(),
        _ => "error".into(),
    }
}

/// reqwest errors may include the request URL; GitHub URLs never carry credentials but codeload redirect
/// URLs carry a short-lived token in the query string, so drop the URL entirely.
fn redact_reqwest(e: &reqwest::Error) -> String {
    let kind = if e.is_timeout() {
        "timeout"
    } else if e.is_connect() {
        "connection failed"
    } else if e.is_body() || e.is_decode() {
        "body error"
    } else {
        "request failed"
    };
    format!("{kind}: {}", e.without_url_ref())
}

trait WithoutUrl {
    fn without_url_ref(&self) -> String;
}

impl WithoutUrl for reqwest::Error {
    fn without_url_ref(&self) -> String {
        let mut s = self.to_string();
        if let Some(url) = self.url() {
            s = s.replace(url.as_str(), "<url>");
        }
        s
    }
}

fn next_link(headers: &HeaderMap) -> Option<Url> {
    let link = headers.get("link")?.to_str().ok()?;
    link.split(',').find_map(|part| {
        let mut pieces = part.split(';');
        let url = pieces.next()?.trim().trim_start_matches('<').trim_end_matches('>');
        pieces.any(|p| p.trim() == r#"rel="next""#).then(|| Url::parse(url).ok()).flatten()
    })
}

fn header_u64(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

async fn check_status(resp: Response) -> Result<Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let headers = resp.headers().clone();
    let body = resp.text().await.unwrap_or_default();
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_owned))
        .unwrap_or_default();
    let rate_limited = status == StatusCode::TOO_MANY_REQUESTS
        || (status == StatusCode::FORBIDDEN
            && (header_u64(&headers, "x-ratelimit-remaining") == Some(0)
                || headers.contains_key("retry-after")
                || message.to_ascii_lowercase().contains("rate limit")));
    Err(match status {
        _ if rate_limited => {
            let retry_after = header_u64(&headers, "retry-after").or_else(|| {
                header_u64(&headers, "x-ratelimit-reset")
                    .map(|reset| reset.saturating_sub(Utc::now().timestamp() as u64).max(1))
            });
            GitHubError::RateLimited { retry_after }
        }
        StatusCode::UNAUTHORIZED => GitHubError::Unauthorized,
        StatusCode::FORBIDDEN => GitHubError::Forbidden { message },
        StatusCode::NOT_FOUND => GitHubError::NotFound,
        StatusCode::CONFLICT => GitHubError::Conflict { message },
        StatusCode::UNPROCESSABLE_ENTITY => GitHubError::Unprocessable { message },
        s if s.is_server_error() => GitHubError::Server { status: s.as_u16() },
        s => GitHubError::Unexpected { status: s.as_u16() },
    })
}

async fn decode<T: DeserializeOwned>(resp: Response) -> Result<T> {
    let bytes = resp.bytes().await.map_err(|e| GitHubError::Network(redact_reqwest(&e)))?;
    serde_json::from_slice(&bytes).map_err(|e| GitHubError::Decode(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_paths_per_segment() {
        assert_eq!(encode_path("feature/a b#1"), "feature/a%20b%231");
        assert_eq!(encode_path("docs/bounded-context.md"), "docs/bounded-context.md");
        assert_eq!(seg("a/b"), "a%2Fb");
    }

    #[test]
    fn parses_link_header() {
        let mut h = HeaderMap::new();
        h.insert(
            "link",
            HeaderValue::from_static(
                r#"<https://api.github.com/x?page=2>; rel="next", <https://api.github.com/x?page=5>; rel="last""#,
            ),
        );
        assert_eq!(next_link(&h).unwrap().as_str(), "https://api.github.com/x?page=2");
        let mut h = HeaderMap::new();
        h.insert("link", HeaderValue::from_static(r#"<https://api.github.com/x?page=1>; rel="prev""#));
        assert!(next_link(&h).is_none());
    }

    #[test]
    fn trailing_slash_preserves_ghes_prefix() {
        let u = ensure_trailing_slash(Url::parse("https://ghe.example.com/api/v3").unwrap());
        assert_eq!(u.join("repos/a/b").unwrap().as_str(), "https://ghe.example.com/api/v3/repos/a/b");
    }
}
