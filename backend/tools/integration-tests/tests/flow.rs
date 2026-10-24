//! End-to-end: API + worker + PostgreSQL + mock GitHub + scripted agent runtime.

mod common;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use common::{publish_terminal, terminal, Harness};
use serde_json::Value;
use tower::ServiceExt;

fn id(job: &Value) -> String {
    job["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn generate_review_and_publish() {
    let Some(h) = Harness::new().await else { return };
    let _worker = h.start_worker(1).await;
    let cookie = h.login("octocat").await;

    let created = h.create_job(&cookie, "acme", "shop", "main").await;
    assert_eq!(created.status, StatusCode::ACCEPTED, "{}", created.text);
    let job_id = id(&created.body);
    let job = h.wait_job(&cookie, &job_id, 30, terminal).await;
    assert_eq!(job["status"], "succeeded", "{job}");
    assert_eq!(job["stage"], "completed");
    let main_sha = h.mock.with(|s| s.branch_head("acme", "shop", "main")).unwrap();
    assert_eq!(job["base_sha"], main_sha);
    let states: Vec<_> = job["steps"].as_array().unwrap().iter().map(|s| s["state"].as_str().unwrap()).collect();
    assert_eq!(states, ["done"; 5], "{job}");
    assert_eq!(job["output"]["primary_path"], "docs/bounded-context.md");
    assert_eq!(job["output"]["files"][0]["change_type"], "added");
    assert_eq!(job["output"]["unexpected_changes"].as_array().unwrap().len(), 0);
    assert_eq!(job["can_publish"], true);
    assert_eq!(job["can_cancel"], false);
    assert!(job["duration_ms"].as_i64().unwrap() >= 0);

    // Logs are stored, redacted, and never contain the user's token.
    let logs = h.get(&format!("/api/bounded-context/jobs/{job_id}/logs"), &cookie).await;
    let lines: Vec<String> =
        logs.body["items"].as_array().unwrap().iter().map(|l| l["message"].as_str().unwrap().to_owned()).collect();
    assert!(lines.iter().any(|l| l.contains("analysing repository with token [REDACTED]")), "{lines:#?}");
    assert!(!logs.text.contains("ghu_"), "token leaked into logs");
    let after = logs.body["next_after"].as_i64().unwrap();
    let none = h.get(&format!("/api/bounded-context/jobs/{job_id}/logs?after={after}"), &cookie).await;
    assert_eq!(none.body["items"].as_array().unwrap().len(), 0);

    let diff = h.get(&format!("/api/bounded-context/jobs/{job_id}/diff"), &cookie).await;
    assert_eq!(diff.status, StatusCode::OK);
    let file = &diff.body["files"][0];
    assert_eq!(file["path"], "docs/bounded-context.md");
    assert!(file["base_content"].is_null());
    assert!(file["new_content"].as_str().unwrap().starts_with("# Bounded Contexts"));
    assert!(file["unified_diff"].as_str().unwrap().contains("+# Bounded Contexts"));

    let out = h.get(&format!("/api/bounded-context/jobs/{job_id}/output"), &cookie).await;
    assert_eq!(out.headers[header::CONTENT_TYPE], "text/markdown; charset=utf-8");
    assert_eq!(out.headers[header::CONTENT_DISPOSITION], "attachment; filename=\"bounded-context.md\"");
    assert!(out.text.contains("Orders, Billing and Catalog"));

    // Publish.
    let p = h.post(&format!("/api/bounded-context/jobs/{job_id}/pull-request"), &cookie, None).await;
    assert_eq!(p.status, StatusCode::ACCEPTED, "{}", p.text);
    assert_eq!(p.body["publish"]["status"], "queued");
    let job = h.wait_job(&cookie, &job_id, 30, publish_terminal).await;
    assert_eq!(job["publish"]["status"], "succeeded", "{job}");
    let short = job["short_id"].as_str().unwrap();
    assert_eq!(job["publish"]["branch"], format!("bounded-context/shop-{short}"));
    assert_eq!(job["publish"]["pr_number"], 1);
    assert_eq!(job["pr_url"], job["publish"]["pr_url"]);
    assert_eq!(job["can_publish"], false);
    let pstates: Vec<_> =
        job["publish"]["steps"].as_array().unwrap().iter().map(|s| s["state"].as_str().unwrap()).collect();
    assert_eq!(pstates, ["done"; 4]);

    // What landed on GitHub: a commit on a new branch parented on the analysed SHA, containing only the doc.
    let branch = job["publish"]["branch"].as_str().unwrap().to_owned();
    let commit_sha = job["publish"]["commit_sha"].as_str().unwrap().to_owned();
    h.mock.with(|s| {
        assert_eq!(s.branch_head("acme", "shop", &branch).as_deref(), Some(commit_sha.as_str()));
        let c = s.commit(&commit_sha).unwrap();
        assert_eq!(c.parents, vec![main_sha.clone()]);
        assert!(c.message.contains(&format!("Bounded-Context-Job: {job_id}")));
        assert!(c.message.contains("Co-authored-by: octocat"));
        assert!(s.file_at(&commit_sha, "docs/bounded-context.md").unwrap().starts_with("# Bounded Contexts"));
        assert_eq!(s.file_at(&commit_sha, "README.md"), s.file_at(&main_sha, "README.md"));
        let pulls = s.pulls_for("acme", "shop");
        assert_eq!(pulls.len(), 1);
        assert_eq!(pulls[0].base, "main");
        assert!(pulls[0].body.contains("placeholder agent"));
        assert_eq!(s.branch_head("acme", "shop", "main").as_deref(), Some(main_sha.as_str()), "base branch untouched");
    });

    // Publishing again is idempotent.
    let again = h.post(&format!("/api/bounded-context/jobs/{job_id}/pull-request"), &cookie, None).await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(h.mock.with(|s| s.pulls_for("acme", "shop").len()), 1);

    let audit: Vec<String> = h.store.audit_actions_for_job(job_id.parse().unwrap()).await.unwrap();
    for action in [
        "generation_requested",
        "generation_succeeded",
        "publish_requested",
        "commit_created",
        "branch_created",
        "pull_request_created",
    ] {
        assert!(audit.iter().any(|a| a == action), "missing audit {action}: {audit:?}");
    }
}

#[tokio::test]
async fn unexpected_changes_are_reported_and_never_committed() {
    let Some(h) = Harness::new().await else { return };
    let _worker = h.start_worker(1).await;
    let cookie = h.login("octocat").await;
    let job_id = id(&h.create_job(&cookie, "acme", "legacy-monolith", "main").await.body);
    let job = h.wait_job(&cookie, &job_id, 30, terminal).await;
    assert_eq!(job["status"], "succeeded", "{job}");
    let unexpected = job["output"]["unexpected_changes"].as_array().unwrap();
    assert_eq!(unexpected.len(), 1);
    assert_eq!(unexpected[0]["path"], "README.md");
    assert_eq!(unexpected[0]["change_type"], "modified");

    h.post(&format!("/api/bounded-context/jobs/{job_id}/pull-request"), &cookie, None).await;
    let job = h.wait_job(&cookie, &job_id, 30, publish_terminal).await;
    assert_eq!(job["publish"]["status"], "succeeded", "{job}");
    let commit = job["publish"]["commit_sha"].as_str().unwrap().to_owned();
    h.mock.with(|s| {
        assert_eq!(s.file_at(&commit, "README.md").unwrap(), "# Legacy\n", "README change must not be committed")
    });
}

#[tokio::test]
async fn agent_failures_are_reported_with_codes() {
    let Some(h) = Harness::new().await else { return };
    let _worker = h.start_worker(1).await;
    let cookie = h.login("octocat").await;

    let job_id = id(&h.create_job(&cookie, "acme", "broken-agent", "main").await.body);
    let job = h.wait_job(&cookie, &job_id, 30, terminal).await;
    assert_eq!(job["status"], "failed");
    assert_eq!(job["error"]["code"], "agent_failed");
    assert_eq!(job["can_retry"], true);
    assert_eq!(job["can_publish"], false);
    let failed: Vec<_> = job["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["key"].as_str().unwrap(), s["state"].as_str().unwrap()))
        .collect();
    assert_eq!(failed[3], ("running_agent", "failed"));
    assert_eq!(failed[4], ("validating_output", "skipped"));
    let p = h.post(&format!("/api/bounded-context/jobs/{job_id}/pull-request"), &cookie, None).await;
    assert_eq!(p.status, StatusCode::CONFLICT);

    h.add_acme_repo("model-denied", &[("README.md", "# x\n"), (".fake-copilot-mode", "model\n")]);
    let job_id = id(&h.create_job(&cookie, "acme", "model-denied", "main").await.body);
    let job = h.wait_job(&cookie, &job_id, 30, terminal).await;
    assert_eq!(job["error"]["code"], "model_unavailable", "{job}");
    assert_eq!(job["error"]["retryable"], false);

    h.add_acme_repo("empty-output", &[("README.md", "# x\n"), (".fake-copilot-mode", "empty\n")]);
    let job_id = id(&h.create_job(&cookie, "acme", "empty-output", "main").await.body);
    let job = h.wait_job(&cookie, &job_id, 30, terminal).await;
    assert_eq!(job["error"]["code"], "output_empty", "{job}");
}

#[tokio::test]
async fn running_jobs_can_be_cancelled() {
    let Some(h) = Harness::new().await else { return };
    let _worker = h.start_worker(1).await;
    let cookie = h.login("octocat").await;
    h.add_acme_repo("slow", &[("README.md", "# slow\n"), (".fake-copilot-mode", "slow\n")]);
    let job_id = id(&h.create_job(&cookie, "acme", "slow", "main").await.body);
    h.wait_job(&cookie, &job_id, 30, |j| j["stage"] == "running_agent").await;
    let c = h.post(&format!("/api/bounded-context/jobs/{job_id}/cancel"), &cookie, None).await;
    assert_eq!(c.status, StatusCode::ACCEPTED);
    assert_eq!(c.body["cancel_requested"], true);
    assert_eq!(c.body["can_cancel"], false);
    let job = h.wait_job(&cookie, &job_id, 30, terminal).await;
    assert_eq!(job["status"], "cancelled", "{job}");
    assert_eq!(job["error"]["code"], "cancelled");
}

#[tokio::test]
async fn publishing_detects_conflicting_base_changes() {
    let Some(h) = Harness::new().await else { return };
    let _worker = h.start_worker(1).await;
    let cookie = h.login("octocat").await;

    // Unrelated change on the base branch: still publishable, based on the analysed commit.
    let job_id = id(&h.create_job(&cookie, "acme", "payments", "main").await.body);
    let job = h.wait_job(&cookie, &job_id, 30, terminal).await;
    let analysed = job["base_sha"].as_str().unwrap().to_owned();
    h.mock.with(|s| s.advance_branch("acme", "payments", "main", &[("src/other.ts", "x")]));
    h.post(&format!("/api/bounded-context/jobs/{job_id}/pull-request"), &cookie, None).await;
    let job = h.wait_job(&cookie, &job_id, 30, publish_terminal).await;
    assert_eq!(job["publish"]["status"], "succeeded", "{job}");
    let commit = job["publish"]["commit_sha"].as_str().unwrap().to_owned();
    h.mock.with(|s| assert_eq!(s.commit(&commit).unwrap().parents, vec![analysed.clone()]));

    // Someone edits the output file after the analysis: refuse to overwrite.
    let job_id = id(&h.create_job(&cookie, "acme", "shop", "develop").await.body);
    h.wait_job(&cookie, &job_id, 30, terminal).await;
    h.mock
        .with(|s| s.advance_branch("acme", "shop", "develop", &[("docs/bounded-context.md", "# edited by a human\n")]));
    h.post(&format!("/api/bounded-context/jobs/{job_id}/pull-request"), &cookie, None).await;
    let job = h.wait_job(&cookie, &job_id, 30, publish_terminal).await;
    assert_eq!(job["publish"]["status"], "failed");
    assert_eq!(job["publish"]["error"]["code"], "base_branch_changed");
    assert_eq!(h.mock.with(|s| s.pulls_for("acme", "shop").len()), 0);
}

#[tokio::test]
async fn existing_foreign_branch_is_never_overwritten_and_transient_errors_retry() {
    let Some(h) = Harness::new().await else { return };
    let _worker = h.start_worker(1).await;
    let cookie = h.login("octocat").await;
    let job = h.create_job(&cookie, "acme", "shop", "main").await.body;
    let job_id = id(&job);
    let short = job["short_id"].as_str().unwrap().to_owned();
    h.wait_job(&cookie, &job_id, 30, terminal).await;
    // A human already created the branch name the portal would use.
    h.mock.with(|s| {
        let head = s.branch_head("acme", "shop", "develop").unwrap();
        s.repos.get_mut("acme/shop").unwrap().branches.insert(format!("bounded-context/shop-{short}"), head);
    });
    h.post(&format!("/api/bounded-context/jobs/{job_id}/pull-request"), &cookie, None).await;
    let job = h.wait_job(&cookie, &job_id, 30, publish_terminal).await;
    assert_eq!(job["publish"]["error"]["code"], "branch_conflict", "{job}");
    assert_eq!(job["can_publish"], true, "the user may retry after deleting the branch");

    // Delete the branch, inject one transient GitHub failure on PR creation: publishing retries and succeeds.
    h.mock.with(|s| {
        s.repos.get_mut("acme/shop").unwrap().branches.remove(&format!("bounded-context/shop-{short}"));
        s.fail_next("create_pull", 502, 1);
    });
    let r = h.post(&format!("/api/bounded-context/jobs/{job_id}/pull-request"), &cookie, None).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
    // The retry is scheduled ~20 s out; pull it forward so the test stays fast.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    sqlx::query("UPDATE job_tasks SET run_after = now() WHERE state = 'pending'")
        .execute(h.store.pool())
        .await
        .unwrap();
    let job = h
        .wait_job(&cookie, &job_id, 30, |j| j["publish"]["status"] == "succeeded" || j["publish"]["status"] == "failed")
        .await;
    assert_eq!(job["publish"]["status"], "succeeded", "{job}");
    assert_eq!(h.mock.with(|s| s.hits.get("create_commit").copied()), Some(1), "commit reused across attempts");
}

#[tokio::test]
async fn base_check_runs_again_when_retrying_a_failed_publish() {
    let Some(h) = Harness::new().await else { return };
    let _worker = h.start_worker(1).await;
    let cookie = h.login("octocat").await;
    let job = h.create_job(&cookie, "acme", "shop", "main").await.body;
    let job_id = id(&job);
    let short = job["short_id"].as_str().unwrap().to_owned();
    h.wait_job(&cookie, &job_id, 30, terminal).await;
    h.mock.with(|s| {
        let head = s.branch_head("acme", "shop", "develop").unwrap();
        s.repos.get_mut("acme/shop").unwrap().branches.insert(format!("bounded-context/shop-{short}"), head);
    });
    h.post(&format!("/api/bounded-context/jobs/{job_id}/pull-request"), &cookie, None).await;
    let job = h.wait_job(&cookie, &job_id, 30, publish_terminal).await;
    assert_eq!(job["publish"]["error"]["code"], "branch_conflict");
    assert!(job["publish"]["commit_sha"].is_string(), "a commit exists from the failed attempt");
    // Meanwhile a human edits the output file on the base branch; the retry must notice.
    h.mock.with(|s| {
        s.repos.get_mut("acme/shop").unwrap().branches.remove(&format!("bounded-context/shop-{short}"));
        s.advance_branch("acme", "shop", "main", &[("docs/bounded-context.md", "# human edit\n")]);
    });
    h.post(&format!("/api/bounded-context/jobs/{job_id}/pull-request"), &cookie, None).await;
    let job = h
        .wait_job(&cookie, &job_id, 30, |j| {
            j["publish"]["status"] == "failed" && j["publish"]["error"]["code"] != "branch_conflict"
        })
        .await;
    assert_eq!(job["publish"]["error"]["code"], "base_branch_changed", "{job}");
}

#[tokio::test]
async fn internal_errors_are_not_leaked_to_users() {
    let Some(h) = Harness::new().await else { return };
    // A regular file where the work directory should be: creating the scratch dir fails.
    let blocker = h.tmp.path().join("not-a-dir");
    std::fs::write(&blocker, "x").unwrap();
    let _worker = h.start_worker_with(1, |cfg| cfg.work_dir = blocker.join("work")).await;
    let cookie = h.login("octocat").await;
    let job_id = id(&h.create_job(&cookie, "acme", "shop", "main").await.body);
    let job = h.wait_job(&cookie, &job_id, 30, terminal).await;
    assert_eq!(job["error"]["code"], "internal", "{job}");
    let message = job["error"]["message"].as_str().unwrap();
    assert!(!message.contains('/') && !message.contains("os error"), "leaked detail: {message}");
    let logs = h.get(&format!("/api/bounded-context/jobs/{job_id}/logs"), &cookie).await;
    assert!(!logs.text.contains("not-a-dir"), "leaked detail in logs: {}", logs.text);
}

#[tokio::test]
async fn read_only_users_can_generate_but_not_publish() {
    let Some(h) = Harness::new().await else { return };
    let _worker = h.start_worker(1).await;
    let viewer = h.login("viewer").await;
    let created = h.create_job(&viewer, "acme", "readonly-docs", "main").await;
    assert_eq!(created.status, StatusCode::ACCEPTED, "{}", created.text);
    let job_id = id(&created.body);
    let job = h.wait_job(&viewer, &job_id, 30, terminal).await;
    assert_eq!(job["status"], "succeeded", "{job}");
    assert_eq!(job["can_publish"], false);
    let p = h.post(&format!("/api/bounded-context/jobs/{job_id}/pull-request"), &viewer, None).await;
    assert_eq!(p.status, StatusCode::FORBIDDEN);
    assert_eq!(p.body["error"]["code"], "repository_access_denied");
}

#[tokio::test]
async fn sse_streams_progress_until_settled() {
    let Some(h) = Harness::new().await else { return };
    let cookie = h.login("octocat").await;
    let job_id = id(&h.create_job(&cookie, "acme", "shop", "main").await.body);
    let req = Request::get(format!("/api/bounded-context/jobs/{job_id}/events"))
        .header(header::COOKIE, &cookie)
        .body(Body::empty())
        .unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()[header::CONTENT_TYPE], "text/event-stream");
    let _worker = h.start_worker(1).await;
    let body = tokio::time::timeout(
        std::time::Duration::from_secs(40),
        axum::body::to_bytes(resp.into_body(), 64 * 1024 * 1024),
    )
    .await
    .expect("stream should close once the job is settled")
    .unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.starts_with("event: job\n"), "{text}");
    assert!(text.contains("event: log\n"), "{text}");
    let last_job = text.rsplit("event: job\ndata: ").next().unwrap().lines().next().unwrap();
    let last: Value = serde_json::from_str(last_job).unwrap();
    assert_eq!(last["status"], "succeeded");
}
