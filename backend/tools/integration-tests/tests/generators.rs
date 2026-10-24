//! Generic generation platform: generators (agent bundles from repositories), multi-repository batches and
//! per-job generator pinning.

mod common;

use axum::http::StatusCode;
use common::{terminal, Harness};
use serde_json::{json, Value};

fn ids(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap().to_owned()).collect()
}

#[tokio::test]
async fn lists_builtin_and_curated_generators() {
    let Some(h) = Harness::new().await else { return };
    let cookie = h.login("octocat").await;
    let r = h.get("/api/generators", &cookie).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(ids(&r.body["items"]), ["builtin", "lite", "ctx"]);
    assert_eq!(r.body["default_id"], "builtin");
    assert_eq!(r.body["can_add"], true);
    let lite = &r.body["items"][1];
    assert_eq!(lite["source"], "curated");
    assert_eq!(lite["removable"], false);
    assert_eq!(lite["repository"]["owner"], "genhub");

    // The agent info for the form: inputs come from the generator's own bundle, pinned to its head commit.
    let ctx = h.get("/api/generators/ctx/agent", &cookie).await;
    assert_eq!(ctx.status, StatusCode::OK, "{}", ctx.text);
    assert_eq!(ctx.body["inputs"][0]["name"], "depth");
    assert_eq!(ctx.body["inputs"][0]["choices"], json!(["LEAN", "FULL"]));
    assert_eq!(ctx.body["resolved"]["branch"], "main");
    let head = h.mock.with(|s| s.branch_head("genhub", "monorepo", "main")).unwrap();
    assert_eq!(ctx.body["resolved"]["sha"], head);
    let builtin = h.get("/api/generators/builtin/agent", &cookie).await;
    assert_eq!(builtin.body["id"], "bounded-context");
    assert_eq!(h.get("/api/agent", &cookie).await.body["id"], "bounded-context");
    assert_eq!(h.get("/api/generators/nope/agent", &cookie).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn users_add_validate_and_remove_their_own_generators() {
    let Some(h) = Harness::new().await else { return };
    let cookie = h.login("octocat").await;
    let add = |body: Value| h.post("/api/generators", &cookie, Some(body));

    let ok = add(json!({ "owner": "genhub", "repo": "lite" })).await;
    assert_eq!(ok.status, StatusCode::CREATED, "{}", ok.text);
    assert_eq!(ok.body["source"], "user");
    assert_eq!(ok.body["removable"], true);
    assert_eq!(ok.body["name"], "Lite generator", "defaults to the bundle's display name");
    let id = ok.body["id"].as_str().unwrap().to_owned();
    // Adding the same repository again is idempotent.
    let again = add(json!({ "owner": "GenHub", "repo": "lite", "name": "ignored" })).await;
    assert_eq!(again.body["id"], id);

    // Not on the operator allowlist.
    let r = add(json!({ "owner": "acme", "repo": "shop" })).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert_eq!(r.body["error"]["code"], "generator_not_allowed");
    // Allowed owner, but the repository is not a valid generator.
    let r = add(json!({ "owner": "genhub", "repo": "no-bundle" })).await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(r.body["error"]["code"], "generator_invalid");
    assert!(r.body["error"]["message"].as_str().unwrap().contains("bundle.yaml"));
    let r = add(json!({ "owner": "genhub", "repo": "unsafe" })).await;
    assert_eq!(r.body["error"]["code"], "generator_invalid", "{}", r.text);
    assert!(r.body["error"]["message"].as_str().unwrap().contains("documentation"));
    // Unknown repository, bad branch, bad path.
    assert_eq!(add(json!({ "owner": "genhub", "repo": "missing" })).await.status, StatusCode::FORBIDDEN);
    let r = add(json!({ "owner": "genhub", "repo": "lite", "branch": "nope" })).await;
    assert_eq!(r.body["error"]["code"], "generator_invalid");
    let r = add(json!({ "owner": "genhub", "repo": "monorepo", "path": "../x" })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.body["error"]["details"]["fields"]["path"].is_string());

    // A sub-directory generator, then the per-user cap (2 in the test registry).
    let sub = add(json!({ "owner": "genhub", "repo": "monorepo", "path": "generators/ctx/" })).await;
    assert_eq!(sub.status, StatusCode::CREATED, "{}", sub.text);
    assert_eq!(sub.body["repository"]["path"], "generators/ctx");
    let r = add(json!({ "owner": "genhub", "repo": "lite", "branch": "main", "path": "" })).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "cap reached");

    let list = h.get("/api/generators", &cookie).await;
    assert_eq!(list.body["items"].as_array().unwrap().len(), 5);
    // Saved generators are private to their owner.
    let other = h.login("viewer").await;
    let theirs = h.get("/api/generators", &other).await;
    assert_eq!(theirs.body["items"].as_array().unwrap().len(), 3);
    assert_eq!(h.get(&format!("/api/generators/{id}/agent"), &other).await.status, StatusCode::NOT_FOUND);
    assert_eq!(
        h.call("DELETE", &format!("/api/generators/{id}"), Some(&other), None).await.status,
        StatusCode::NOT_FOUND
    );

    let del = h.call("DELETE", &format!("/api/generators/{id}"), Some(&cookie), None).await;
    assert_eq!(del.status, StatusCode::NO_CONTENT);
    assert_eq!(
        h.call("DELETE", &format!("/api/generators/{id}"), Some(&cookie), None).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(h.get("/api/generators", &cookie).await.body["items"].as_array().unwrap().len(), 4);
}

#[tokio::test]
async fn different_generators_may_run_on_the_same_repository() {
    let Some(h) = Harness::new().await else { return };
    let cookie = h.login("octocat").await;
    let job = |generator: Option<&str>| {
        let mut body = json!({ "owner": "acme", "repo": "shop", "base_branch": "main", "model": "auto" });
        if let Some(g) = generator {
            body["generator_id"] = json!(g);
        }
        h.post("/api/bounded-context/jobs", &cookie, Some(body))
    };
    let builtin = job(None).await;
    assert_eq!(builtin.status, StatusCode::ACCEPTED, "{}", builtin.text);
    assert_eq!(builtin.body["generator"]["source"], "builtin", "{}", builtin.text);
    let lite = job(Some("lite")).await;
    assert_eq!(lite.status, StatusCode::ACCEPTED, "{}", lite.text);
    assert_eq!(lite.body["generator"]["source"], "curated");
    assert_eq!(lite.body["generator"]["owner"], "genhub");
    assert_eq!(lite.body["agent"]["id"], "lite");
    // The same generator twice is still a duplicate.
    let dup = job(Some("lite")).await;
    assert_eq!(dup.status, StatusCode::CONFLICT);
    assert_eq!(dup.body["error"]["code"], "duplicate_job");
    assert_eq!(dup.body["error"]["details"]["job_id"], lite.body["id"]);
    // Unknown generator.
    assert_eq!(job(Some("missing")).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn batch_validation() {
    let Some(h) = Harness::new().await else { return };
    let cookie = h.login("octocat").await;
    let post = |repos: Value| {
        h.post(
            "/api/bounded-context/batches",
            &cookie,
            Some(json!({ "generator_id": "builtin", "model": "auto", "repositories": repos })),
        )
    };
    let r = post(json!([])).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.body["error"]["details"]["fields"]["repositories"].is_string());
    let many: Vec<Value> =
        (0..26).map(|i| json!({ "owner": "acme", "repo": format!("svc-{i:02}"), "base_branch": "main" })).collect();
    assert_eq!(post(json!(many)).await.status, StatusCode::BAD_REQUEST);
    let dup = json!([
        { "owner": "acme", "repo": "shop", "base_branch": "main" },
        { "owner": "ACME", "repo": "Shop", "base_branch": "main" },
    ]);
    let r = post(dup).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.body["error"]["details"]["fields"]["repositories"].as_str().unwrap().contains("more than once"));
    // Request-level problems fail the whole batch before anything is queued.
    let r = h
        .post(
            "/api/bounded-context/batches",
            &cookie,
            Some(json!({ "generator_id": "builtin", "model": "nope-model", "repositories": [
                { "owner": "acme", "repo": "shop", "base_branch": "main" } ] })),
        )
        .await;
    assert_eq!(r.status, StatusCode::UNPROCESSABLE_ENTITY);
    let list = h.get("/api/bounded-context/jobs", &cookie).await;
    assert_eq!(list.body["total"], 0);
}

#[tokio::test]
async fn batch_runs_each_repository_with_the_pinned_generator() {
    let Some(h) = Harness::new().await else { return };
    let _worker = h.start_worker(1).await;
    let cookie = h.login("octocat").await;
    let pinned = h.mock.with(|s| s.branch_head("genhub", "monorepo", "main")).unwrap();

    let r = h
        .post(
            "/api/bounded-context/batches",
            &cookie,
            Some(json!({
                "generator_id": "ctx", "model": "gpt-5", "inputs": { "depth": "FULL" },
                "repositories": [
                    { "owner": "acme", "repo": "shop", "base_branch": "main" },
                    { "owner": "acme", "repo": "payments", "base_branch": "main" },
                    { "owner": "acme", "repo": "not-installed", "base_branch": "main" },
                    { "owner": "acme", "repo": "shop", "base_branch": "no-such-branch" },
                ]
            })),
        )
        .await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
    let batch_id = r.body["batch_id"].as_str().unwrap().to_owned();
    let jobs = r.body["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 2, "{}", r.text);
    assert!(jobs.iter().all(|j| j["batch_id"] == batch_id.as_str()));
    let failures = r.body["failures"].as_array().unwrap();
    assert_eq!(failures.len(), 2, "{}", r.text);
    assert_eq!(failures[0]["repo"], "not-installed");
    assert_eq!(failures[0]["error"]["code"], "app_not_installed");
    assert_eq!(failures[1]["error"]["code"], "validation_failed");

    // Move the generator's branch after the batch was created: jobs still run the pinned revision.
    h.mock.with(|s| s.advance_branch("genhub", "monorepo", "main", &[("generators/ctx/ctx.agent.md", "# changed\n")]));

    for j in jobs {
        let id = j["id"].as_str().unwrap();
        let job = h.wait_job(&cookie, id, 40, terminal).await;
        assert_eq!(job["status"], "succeeded", "{job}");
        assert_eq!(job["generator"]["id"], "ctx");
        assert_eq!(job["generator"]["sha"], pinned);
        assert_eq!(job["generator"]["path"], "generators/ctx");
        assert_eq!(job["inputs"]["depth"], "FULL");
        assert_eq!(job["model"], "gpt-5");
        let logs = h.get(&format!("/api/bounded-context/jobs/{id}/logs"), &cookie).await;
        let lines: Vec<String> =
            logs.body["items"].as_array().unwrap().iter().map(|l| l["message"].as_str().unwrap().to_owned()).collect();
        assert!(lines.iter().any(|l| l.contains(&format!("at {}", &pinned[..12]))), "{lines:#?}");
        // The bundle asked for a shell; the platform removed it.
        assert!(lines.iter().any(|l| l.contains("Removed tool permissions")), "{lines:#?}");
    }

    let list = h.get(&format!("/api/bounded-context/jobs?batch_id={batch_id}"), &cookie).await;
    assert_eq!(list.body["total"], 2);
    assert_eq!(list.body["items"][0]["batch_id"], batch_id.as_str());
    let none = h.get("/api/bounded-context/jobs?batch_id=00000000-0000-4000-8000-000000000000", &cookie).await;
    assert_eq!(none.body["total"], 0);
    assert_eq!(h.get("/api/bounded-context/jobs?batch_id=nope", &cookie).await.status, StatusCode::BAD_REQUEST);

    // A job from a repository generator can still be published (the recorded output allowlist applies).
    let first = jobs[0]["id"].as_str().unwrap();
    let p = h.post(&format!("/api/bounded-context/jobs/{first}/pull-request"), &cookie, None).await;
    assert_eq!(p.status, StatusCode::ACCEPTED, "{}", p.text);
    let job = h.wait_job(&cookie, first, 30, common::publish_terminal).await;
    assert_eq!(job["publish"]["status"], "succeeded", "{job}");
}

#[tokio::test]
async fn retry_reuses_the_generator() {
    let Some(h) = Harness::new().await else { return };
    let _worker = h.start_worker(1).await;
    let cookie = h.login("octocat").await;
    let r = h
        .post(
            "/api/bounded-context/jobs",
            &cookie,
            Some(json!({ "owner": "acme", "repo": "broken-agent", "base_branch": "main", "model": "auto", "generator_id": "lite" })),
        )
        .await;
    let id = r.body["id"].as_str().unwrap().to_owned();
    let failed = h.wait_job(&cookie, &id, 40, terminal).await;
    assert_eq!(failed["status"], "failed", "{failed}");
    let retry = h.post(&format!("/api/bounded-context/jobs/{id}/retry"), &cookie, None).await;
    assert_eq!(retry.status, StatusCode::ACCEPTED, "{}", retry.text);
    assert_eq!(retry.body["generator"]["id"], "lite");
    assert_eq!(retry.body["agent"]["id"], "lite");
}
