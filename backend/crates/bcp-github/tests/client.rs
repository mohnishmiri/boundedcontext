use bcp_core::config::GitHubSettings;
use bcp_core::ErrorCode;
use bcp_github::{GitHubClient, GitHubError, TreeFile};
use url::Url;
use wiremock::matchers::{body_partial_json, header, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Throwaway RSA key generated per test run (never a real secret).
fn test_key() -> String {
    static KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    KEY.get_or_init(|| {
        let out = std::process::Command::new("openssl")
            .args(["genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048"])
            .output()
            .expect("openssl is required to generate a throwaway test key");
        assert!(out.status.success(), "openssl failed");
        String::from_utf8(out.stdout).unwrap()
    })
    .clone()
}

fn client(server: &MockServer) -> GitHubClient {
    let base = Url::parse(&server.uri()).unwrap();
    GitHubClient::new(&GitHubSettings {
        api_url: base.join("/api/v3").unwrap(),
        web_url: base.clone(),
        oauth_url: base,
        app_id: "42".into(),
        app_slug: "bcp".into(),
        client_id: "Iv1.client".into(),
        client_secret: "shh-secret".into(),
        private_key_pem: test_key(),
    })
    .unwrap()
}

#[tokio::test]
async fn sends_bearer_token_and_api_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/user"))
        .and(header("authorization", "Bearer ghu_usertoken"))
        .and(header("x-github-api-version", "2022-11-28"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 7, "login": "octocat", "name": "Mona", "avatar_url": null
        })))
        .expect(1)
        .mount(&server)
        .await;
    let user = client(&server).get_authenticated_user("ghu_usertoken").await.unwrap();
    assert_eq!(user.login, "octocat");
}

#[tokio::test]
async fn retries_transient_get_failures() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repos/acme/shop"))
        .respond_with(ResponseTemplate::new(502))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repos/acme/shop"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": 1, "name": "shop", "full_name": "acme/shop", "owner": {"login": "acme"},
            "private": true, "default_branch": "main", "html_url": "https://github.com/acme/shop",
            "permissions": {"pull": true, "push": false, "admin": false}
        })))
        .mount(&server)
        .await;
    let repo = client(&server).get_repository("t", "acme", "shop").await.unwrap();
    assert!(repo.permissions().pull);
    assert!(!repo.permissions().push);
}

#[tokio::test]
async fn does_not_retry_writes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v3/repos/acme/shop/git/refs"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let err = client(&server).create_branch("t", "acme", "shop", "b", "abc").await.unwrap_err();
    assert_eq!(err, GitHubError::Server { status: 503 });
    assert_eq!(err.to_service_error().code, ErrorCode::GithubUnavailable);
}

#[tokio::test]
async fn classifies_rate_limits_and_auth_errors() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repos/acme/limited"))
        .respond_with(
            ResponseTemplate::new(403)
                .insert_header("retry-after", "120")
                .set_body_json(serde_json::json!({"message": "You have exceeded a secondary rate limit"})),
        )
        .expect(1) // retry-after above the inline cap: surfaced immediately, not slept on
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repos/acme/expired"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repos/acme/hidden"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let c = client(&server);

    let err = c.get_repository("t", "acme", "limited").await.unwrap_err();
    assert_eq!(err, GitHubError::RateLimited { retry_after: Some(120) });
    let app = err.to_user_error();
    assert_eq!(app.code, ErrorCode::GithubRateLimited);
    assert_eq!(app.retry_after, Some(120));

    let err = c.get_repository("t", "acme", "expired").await.unwrap_err();
    assert_eq!(err.to_user_error().code, ErrorCode::GithubAuthExpired);

    let err = c.get_repository("t", "acme", "hidden").await.unwrap_err();
    assert_eq!(err.to_user_error().code, ErrorCode::RepositoryAccessDenied);
}

#[tokio::test]
async fn follows_pagination_links_on_the_api_host_only() {
    let server = MockServer::start().await;
    let next = format!("<{}/api/v3/repos/acme/shop/branches?page=2>; rel=\"next\"", server.uri());
    Mock::given(method("GET"))
        .and(path("/api/v3/repos/acme/shop/branches"))
        .and(query_param("per_page", "100"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("link", next.as_str())
                .set_body_json(serde_json::json!([{ "name": "main", "commit": {"sha": "a1"}, "protected": true }])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repos/acme/shop/branches"))
        .and(query_param("page", "2"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("link", "<https://evil.example.com/steal>; rel=\"next\"")
                .set_body_json(serde_json::json!([{ "name": "dev", "commit": {"sha": "b2"} }])),
        )
        .mount(&server)
        .await;
    let (branches, truncated) = client(&server).list_branches("t", "acme", "shop", 10).await.unwrap();
    assert_eq!(branches.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["main", "dev"]);
    assert!(!truncated, "foreign-host next link must be ignored");
}

#[tokio::test]
async fn oauth_errors_in_200_bodies_are_errors() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/login/oauth/access_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "error": "bad_verification_code", "error_description": "The code passed is incorrect or expired."
        })))
        .mount(&server)
        .await;
    let err = client(&server).exchange_code("c", "http://x/cb", "v").await.unwrap_err();
    assert!(matches!(err, GitHubError::OAuth { ref error, .. } if error == "bad_verification_code"));
    assert_eq!(err.to_user_error().code, ErrorCode::GithubAuthExpired);
}

#[tokio::test]
async fn app_jwt_is_used_for_installation_tokens() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v3/app/installations/9/access_tokens"))
        .and(header_exists("authorization"))
        .and(body_partial_json(serde_json::json!({
            "repositories": ["shop"], "permissions": {"contents": "read"}
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "token": "ghs_installation", "expires_at": "2030-01-01T00:00:00Z"
        })))
        .mount(&server)
        .await;
    let c = client(&server);
    let tok = c.create_installation_token(9, "shop", &[("contents", "read")]).await.unwrap();
    assert_eq!(tok.token, "ghs_installation");
    assert!(!format!("{tok:?}").contains("ghs_"), "Debug must not leak the token");
    let jwt = c.app_jwt().unwrap();
    assert_eq!(jwt.split('.').count(), 3);
}

#[tokio::test]
async fn tarball_download_enforces_size_limit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repos/acme/shop/tarball/abc123"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 4096]))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("t.tgz");
    let c = client(&server);
    assert_eq!(c.download_tarball("t", "acme", "shop", "abc123", &dest, 10_000).await.unwrap(), 4096);
    assert_eq!(
        c.download_tarball("t", "acme", "shop", "abc123", &dest, 1000).await.unwrap_err(),
        GitHubError::TooLarge
    );
}

#[tokio::test]
async fn file_sha_and_tree_creation() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repos/acme/shop/contents/docs/bounded-context.md"))
        .and(query_param("ref", "main"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "type": "file", "sha": "blob1", "size": 10
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v3/repos/acme/shop/contents/missing.md"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v3/repos/acme/shop/git/trees"))
        .and(body_partial_json(serde_json::json!({ "base_tree": "tree0" })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "sha": "tree1" })))
        .mount(&server)
        .await;
    let c = client(&server);
    assert_eq!(
        c.get_file_sha("t", "acme", "shop", "docs/bounded-context.md", "main").await.unwrap().as_deref(),
        Some("blob1")
    );
    assert_eq!(c.get_file_sha("t", "acme", "shop", "missing.md", "main").await.unwrap(), None);
    let files = [TreeFile { path: "docs/x.md".into(), content: "# x".into() }];
    assert_eq!(c.create_tree("t", "acme", "shop", "tree0", &files).await.unwrap(), "tree1");
}
