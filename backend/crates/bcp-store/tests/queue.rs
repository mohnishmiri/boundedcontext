//! Store integration tests against a real PostgreSQL (set BCP_TEST_DATABASE_URL to an admin-capable URL;
//! each test creates its own database). Skipped when unset unless BCP_TEST_REQUIRE_DB=1.

use bcp_core::domain::JobStatus;
use bcp_core::{AppError, ErrorCode};
use bcp_store::{
    CancelOutcome, JobFilter, JobGenerator, NewArtifact, NewAuditEvent, NewJob, NewUserGenerator, PublishOutcome,
    Store, StoreError,
};
use serde_json::json;
use uuid::Uuid;

async fn fresh_store() -> Option<Store> {
    let Ok(base) = std::env::var("BCP_TEST_DATABASE_URL") else {
        assert!(std::env::var("BCP_TEST_REQUIRE_DB").is_err(), "BCP_TEST_DATABASE_URL is required");
        eprintln!("SKIPPED: BCP_TEST_DATABASE_URL not set");
        return None;
    };
    let admin = sqlx::PgPool::connect(&base).await.expect("connect admin");
    let name = format!("bcp_t_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}")).execute(&admin).await.unwrap();
    let (prefix, _) = base.rsplit_once('/').unwrap();
    let store = Store::connect(&format!("{prefix}/{name}"), 5).await.unwrap();
    assert!(!store.ready().await.unwrap());
    store.migrate().await.unwrap();
    assert!(store.ready().await.unwrap());
    Some(store)
}

fn new_job(user: Uuid, branch: &str) -> NewJob {
    NewJob {
        user_id: user,
        owner: "acme".into(),
        repo: "shop".into(),
        repo_id: Some(1),
        installation_id: Some(100),
        base_branch: branch.into(),
        model: "auto".into(),
        inputs: json!({}),
        can_push: true,
        retry_of: None,
        agent_id: "bc".into(),
        agent_version: "abc123".into(),
        agent_placeholder: true,
        max_attempts: 2,
        batch_id: None,
        generator: JobGenerator::builtin("Built-in"),
        output_paths: vec!["docs/*.md".into()],
    }
}

#[tokio::test]
async fn duplicate_active_jobs_are_rejected() {
    let Some(store) = fresh_store().await else { return };
    let user = store.upsert_user(1, "octocat", None, None).await.unwrap();
    let first = store.create_job(new_job(user.id, "main")).await.unwrap();
    assert_eq!(first.status(), JobStatus::Queued);
    assert_eq!(first.requested_by, "octocat");
    match store.create_job(new_job(user.id, "main")).await {
        Err(StoreError::DuplicateJob(id)) => assert_eq!(id, first.id),
        other => panic!("expected duplicate, got {other:?}"),
    }
    let app: AppError = StoreError::DuplicateJob(first.id).into();
    assert_eq!(app.code, ErrorCode::DuplicateJob);
    // A different branch is fine; so is the same branch once the first job is finished.
    store.create_job(new_job(user.id, "develop")).await.unwrap();
    store.request_cancel(user.id, first.id).await.unwrap();
    store.create_job(new_job(user.id, "main")).await.unwrap();
}

#[tokio::test]
async fn claims_are_exclusive_and_leases_are_reaped() {
    let Some(store) = fresh_store().await else { return };
    let user = store.upsert_user(1, "octocat", None, None).await.unwrap();
    let job = store.create_job(new_job(user.id, "main")).await.unwrap();

    // Concurrent claimers: exactly one wins.
    let (a, b) = tokio::join!(store.claim_task("w1", 60, 3, 5), store.claim_task("w2", 60, 3, 5));
    let winners: Vec<_> = [a.unwrap(), b.unwrap()].into_iter().flatten().collect();
    assert_eq!(winners.len(), 1);
    let task = &winners[0];
    assert_eq!(task.job_id, job.id);
    assert_eq!(task.attempts, 1);
    assert_eq!(task.max_attempts, 3, "worker limits replace API defaults");
    assert!(store.claim_task("w3", 60, 3, 5).await.unwrap().is_none());

    assert!(store.heartbeat_task(task.id, &task.lease_owner.clone().unwrap(), 60).await.unwrap());
    assert!(!store.heartbeat_task(task.id, "someone-else", 60).await.unwrap());

    // Expire the lease: the reaper requeues it while attempts remain.
    sqlx::query("UPDATE job_tasks SET lease_until = now() - interval '1 second'").execute(store.pool()).await.unwrap();
    let reaped = store.reap_expired_tasks().await.unwrap();
    assert_eq!(reaped.len(), 1);
    assert!(reaped[0].requeued);
    let again = store.claim_task("w2", 60, 2, 5).await.unwrap().unwrap();
    assert_eq!(again.attempts, 2);
    // Old owner can no longer heartbeat.
    assert!(!store.heartbeat_task(task.id, &task.lease_owner.clone().unwrap(), 60).await.unwrap());

    // Exhausted: the reaper fails it instead.
    sqlx::query("UPDATE job_tasks SET lease_until = now() - interval '1 second'").execute(store.pool()).await.unwrap();
    let reaped = store.reap_expired_tasks().await.unwrap();
    assert!(!reaped[0].requeued);
}

#[tokio::test]
async fn release_does_not_consume_attempts_and_reschedule_delays() {
    let Some(store) = fresh_store().await else { return };
    let user = store.upsert_user(1, "octocat", None, None).await.unwrap();
    store.create_job(new_job(user.id, "main")).await.unwrap();
    let t = store.claim_task("w1", 60, 2, 5).await.unwrap().unwrap();
    store.release_task(t.id, "w1").await.unwrap();
    let t = store.claim_task("w1", 60, 2, 5).await.unwrap().unwrap();
    assert_eq!(t.attempts, 1);
    store.reschedule_task(t.id, "w1", 3600.0, "transient").await.unwrap();
    assert!(store.claim_task("w1", 60, 2, 5).await.unwrap().is_none(), "delayed task must not be claimable yet");
    assert_eq!(store.pending_task_count().await.unwrap(), 1);
}

#[tokio::test]
async fn cancel_queued_vs_running() {
    let Some(store) = fresh_store().await else { return };
    let user = store.upsert_user(1, "octocat", None, None).await.unwrap();
    let other = store.upsert_user(2, "mallory", None, None).await.unwrap();
    let job = store.create_job(new_job(user.id, "main")).await.unwrap();
    assert!(matches!(store.request_cancel(other.id, job.id).await, Err(StoreError::NotFound)), "jobs are private");
    match store.request_cancel(user.id, job.id).await.unwrap() {
        CancelOutcome::Cancelled(j) => assert_eq!(j.status(), JobStatus::Cancelled),
        other => panic!("{other:?}"),
    }
    assert!(store.claim_task("w", 60, 2, 5).await.unwrap().is_none(), "task of a cancelled job is cancelled");
    assert!(matches!(store.request_cancel(user.id, job.id).await, Err(StoreError::Conflict(_))));

    let job = store.create_job(new_job(user.id, "main")).await.unwrap();
    store.claim_task("w", 60, 2, 5).await.unwrap().unwrap();
    store.start_job(job.id, &json!({})).await.unwrap().unwrap();
    match store.request_cancel(user.id, job.id).await.unwrap() {
        CancelOutcome::Requested(j) => assert!(j.cancel_requested && j.status() == JobStatus::Running),
        other => panic!("{other:?}"),
    }
    assert!(store.is_cancel_requested(job.id).await.unwrap());
    assert!(store.start_job(job.id, &json!({})).await.unwrap().is_none(), "cancel-requested jobs do not restart");
}

#[tokio::test]
async fn completion_artifacts_publish_and_audit() {
    let Some(store) = fresh_store().await else { return };
    let user = store.upsert_user(1, "octocat", Some("Mona"), None).await.unwrap();
    let job = store.create_job(new_job(user.id, "main")).await.unwrap();
    assert!(matches!(
        store.request_publish(user.id, job.id, 5, &json!({}), "APM-123: Update context", false).await,
        Err(StoreError::Conflict(_))
    ));
    store.start_job(job.id, &json!({})).await.unwrap();
    let seq1 = store.append_event(job.id, "info", "preparing", "one").await.unwrap();
    let seq2 = store.append_event(job.id, "info", "preparing", "two").await.unwrap();
    assert!(seq2 > seq1);
    assert_eq!(store.list_events(job.id, seq1, 10).await.unwrap().len(), 1);
    store
        .complete_job(
            job.id,
            "docs/bc.md",
            &[
                NewArtifact {
                    path: "docs/bc.md".into(),
                    change_type: "added".into(),
                    allowed: true,
                    base_content: None,
                    new_content: Some("# BC".into()),
                    unified_diff: Some("+# BC".into()),
                    size_bytes: 4,
                    sha256: Some("x".into()),
                },
                NewArtifact {
                    path: "README.md".into(),
                    change_type: "modified".into(),
                    allowed: false,
                    base_content: None,
                    new_content: None,
                    unified_diff: None,
                    size_bytes: 0,
                    sha256: None,
                },
            ],
            &json!({}),
        )
        .await
        .unwrap();
    let done = store.get_job(job.id).await.unwrap().unwrap();
    assert_eq!(done.status(), JobStatus::Succeeded);
    assert_eq!(store.list_artifacts(job.id).await.unwrap().len(), 2);

    assert!(matches!(
        store.request_publish(user.id, job.id, 5, &json!({}), "APM-123: Update context", false).await.unwrap(),
        PublishOutcome::Queued(_)
    ));
    assert!(matches!(
        store.request_publish(user.id, job.id, 5, &json!({}), "APM-123: Update context", false).await.unwrap(),
        PublishOutcome::Unchanged(_)
    ));

    store
        .audit(NewAuditEvent {
            actor_user_id: Some(user.id),
            action: "test",
            job_id: Some(job.id),
            details: json!({ "a": 1 }),
            ..Default::default()
        })
        .await
        .unwrap();
    let err = sqlx::query("DELETE FROM audit_events").execute(store.pool()).await.unwrap_err();
    assert!(err.to_string().contains("append-only"), "{err}");
}

#[tokio::test]
async fn retention_purges_only_settled_old_jobs() {
    let Some(store) = fresh_store().await else { return };
    let user = store.upsert_user(1, "octocat", None, None).await.unwrap();
    let old = store.create_job(new_job(user.id, "main")).await.unwrap();
    store.request_cancel(user.id, old.id).await.unwrap();
    let active = store.create_job(new_job(user.id, "develop")).await.unwrap();
    sqlx::query("UPDATE jobs SET created_at = now() - interval '100 days'").execute(store.pool()).await.unwrap();
    store.append_event(old.id, "info", "queued", "x").await.unwrap();
    let removed = store.purge_jobs_before(chrono::Utc::now() - chrono::Duration::days(90)).await.unwrap();
    assert_eq!(removed, 1);
    assert!(store.get_job(old.id).await.unwrap().is_none());
    assert!(store.get_job(active.id).await.unwrap().is_some(), "active jobs are never purged");
}

#[tokio::test]
async fn sessions_and_tokens() {
    let Some(store) = fresh_store().await else { return };
    let user = store.upsert_user(7, "octocat", None, None).await.unwrap();
    let renamed = store.upsert_user(7, "octocat-renamed", None, None).await.unwrap();
    assert_eq!(user.id, renamed.id, "upsert keys on GitHub id");
    store.create_session(b"hash-1", user.id, chrono::Utc::now() + chrono::Duration::hours(1)).await.unwrap();
    store.create_session(b"hash-2", user.id, chrono::Utc::now() - chrono::Duration::hours(1)).await.unwrap();
    assert_eq!(store.session_user(b"hash-1").await.unwrap().unwrap().user.login, "octocat-renamed");
    assert!(store.session_user(b"hash-2").await.unwrap().is_none(), "expired sessions are invalid");
    assert_eq!(store.purge_expired_sessions().await.unwrap(), 1);
    store.store_tokens(user.id, b"enc", None, Some(b"r"), None).await.unwrap();
    assert!(store.get_tokens(user.id).await.unwrap().is_some());
    assert!(!format!("{:?}", store.get_tokens(user.id).await.unwrap()).contains("enc"));
}

#[tokio::test]
async fn generators_batches_and_saved_generators() {
    let Some(store) = fresh_store().await else { return };
    let user = store.upsert_user(7, "hubot", None, None).await.unwrap();
    let other = store.upsert_user(8, "monalisa", None, None).await.unwrap();

    // The same repo/branch/model may run with different generators at once, but not twice with the same one.
    let batch = Uuid::new_v4();
    let mut a = new_job(user.id, "main");
    a.batch_id = Some(batch);
    let first = store.create_job(a.clone()).await.unwrap();
    assert_eq!(first.batch_id, Some(batch));
    assert_eq!(first.generator_key, "builtin");
    assert_eq!(first.output_globs(), vec!["docs/*.md".to_owned()]);
    assert!(matches!(store.create_job(a.clone()).await, Err(StoreError::DuplicateJob(id)) if id == first.id));
    let mut b = a.clone();
    b.generator = JobGenerator {
        source: "user".into(),
        id: Some("g1".into()),
        name: "Acme generator".into(),
        owner: Some("acme".into()),
        repo: Some("generator".into()),
        branch: Some("main".into()),
        path: Some("bundle".into()),
        sha: Some("f".repeat(40)),
        key: "acme/generator#bundle".into(),
    };
    let second = store.create_job(b).await.unwrap();
    assert_eq!(second.generator_owner.as_deref(), Some("acme"));
    assert_eq!(second.generator_sha.as_deref(), Some("f".repeat(40).as_str()));

    let filter = JobFilter { batch_id: Some(batch), ..Default::default() };
    let (rows, total) = store.list_jobs(user.id, &filter, 1, 50).await.unwrap();
    assert_eq!((rows.len(), total), (2, 2));
    let (rows, total) = store
        .list_jobs(user.id, &JobFilter { batch_id: Some(Uuid::new_v4()), ..Default::default() }, 1, 50)
        .await
        .unwrap();
    assert_eq!((rows.len(), total), (0, 0));

    // Saved generators are per user and idempotent.
    let g =
        NewUserGenerator { user_id: user.id, name: "Mine", owner: "acme", repo: "generator", branch: None, path: "" };
    let saved = store.add_user_generator(g.clone()).await.unwrap();
    let again = store.add_user_generator(NewUserGenerator { owner: "ACME", ..g.clone() }).await.unwrap();
    assert_eq!(saved.id, again.id);
    assert_eq!(store.list_user_generators(user.id).await.unwrap().len(), 1);
    assert!(store.list_user_generators(other.id).await.unwrap().is_empty());
    assert!(matches!(store.delete_user_generator(other.id, saved.id).await, Err(StoreError::NotFound)));
    assert!(matches!(store.get_user_generator(other.id, saved.id).await, Err(StoreError::NotFound)));
    store.delete_user_generator(user.id, saved.id).await.unwrap();
    assert_eq!(store.count_user_generators(user.id).await.unwrap(), 0);
}
