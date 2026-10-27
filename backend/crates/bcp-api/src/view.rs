//! Renders job rows as the API's `JobSummary` / `Job` shapes (docs/api.md).

use bcp_core::domain::{GenerationStage, JobStatus, PublishStage, PublishStatus};
use bcp_core::steps::render_steps;
use bcp_store::{ArtifactRow, JobRow};
use chrono::Utc;
use serde_json::{json, Value};

pub const GENERATION_STEPS: &[(&str, &str)] = &[
    ("queued", "Queued"),
    ("preparing", "Checking access and resolving branch"),
    ("fetching_repository", "Fetching repository snapshot"),
    ("running_agent", "Running bounded-context agent"),
    ("validating_output", "Validating generated documentation"),
];

pub const PUBLISH_STEPS: &[(&str, &str)] = &[
    ("checking_base", "Checking base branch"),
    ("creating_commit", "Creating commit"),
    ("creating_branch", "Creating branch"),
    ("creating_pull_request", "Opening pull request"),
    ("checking_central_base", "Checking central repository"),
    ("creating_central_commit", "Creating central repository commit"),
    ("creating_central_branch", "Creating central repository branch"),
    ("creating_central_pull_request", "Opening central repository pull request"),
];

fn web(web_url: &str, path: &str) -> String {
    format!("{}/{}", web_url.trim_end_matches('/'), path)
}

fn error_json(code: &Option<String>, message: &Option<String>, retryable: Option<bool>) -> Value {
    match code {
        Some(c) => {
            json!({ "code": c, "message": message.clone().unwrap_or_default(), "retryable": retryable.unwrap_or(false) })
        }
        None => Value::Null,
    }
}

pub fn summary(job: &JobRow) -> Value {
    json!({
        "id": job.id,
        "short_id": job.short_id(),
        "owner": job.owner,
        "repo": job.repo,
        "base_branch": job.base_branch,
        "model": job.model,
        "status": job.status,
        "stage": job.stage,
        "publish_status": job.publish_status,
        "pr_url": job.publish_pr_url,
        "created_at": job.created_at,
        "finished_at": job.finished_at,
        "requested_by": job.requested_by,
        "batch_id": job.batch_id,
    })
}

fn generator_json(job: &JobRow) -> Value {
    match &job.generator_name {
        Some(name) => json!({
            "id": job.generator_id, "name": name, "source": job.generator_source,
            "owner": job.generator_owner, "repo": job.generator_repo, "branch": job.generator_branch,
            "path": job.generator_path, "sha": job.generator_sha,
        }),
        None => Value::Null,
    }
}

pub fn can_publish(job: &JobRow, artifacts: &[ArtifactRow]) -> bool {
    job.status() == JobStatus::Succeeded
        && matches!(job.publish_status(), PublishStatus::NotStarted | PublishStatus::Failed)
        && job.can_push
        && artifacts.iter().any(|a| a.allowed)
}

pub fn job(job: &JobRow, artifacts: &[ArtifactRow], web_url: &str, central_repo: Option<&str>) -> Value {
    let status = job.status();
    let publish_status = job.publish_status();
    let ended = status.is_terminal();
    let mut v = summary(job);
    let mut steps_json = job.stage_times.clone();
    // `queued` is done as soon as a worker has started (or the job ended).
    if steps_json.get("queued").is_none_or(|q| q.is_null()) && (job.started_at.is_some() || ended) {
        if let Some(obj) = steps_json.as_object_mut() {
            obj.insert(
                "queued".into(),
                json!({ "state": "done", "started_at": job.created_at, "finished_at": job.started_at.unwrap_or(job.created_at) }),
            );
        }
    }
    let steps = render_steps(GENERATION_STEPS, &steps_json, ended);
    let duration_ms = match (job.started_at, job.finished_at) {
        (Some(s), Some(f)) => Some((f - s).num_milliseconds()),
        (Some(s), None) if !ended => Some((Utc::now() - s).num_milliseconds()),
        _ => None,
    };
    let output = if status == JobStatus::Succeeded {
        let files: Vec<Value> = artifacts
            .iter()
            .filter(|a| a.allowed)
            .map(|a| json!({ "path": a.path, "change_type": a.change_type, "size_bytes": a.size_bytes }))
            .collect();
        let unexpected: Vec<Value> = artifacts
            .iter()
            .filter(|a| !a.allowed)
            .map(|a| json!({ "path": a.path, "change_type": a.change_type }))
            .collect();
        json!({ "primary_path": job.output_primary, "files": files, "unexpected_changes": unexpected })
    } else {
        Value::Null
    };
    let publish_steps = if publish_status == PublishStatus::NotStarted {
        render_steps(PUBLISH_STEPS, &json!({}), false)
    } else {
        render_steps(PUBLISH_STEPS, &job.publish_stage_times, !publish_status.is_active())
    };
    let commit_url =
        job.publish_commit_sha.as_ref().map(|sha| web(web_url, &format!("{}/{}/commit/{sha}", job.owner, job.repo)));

    v["repository_url"] = json!(web(web_url, &format!("{}/{}", job.owner, job.repo)));
    v["base_sha"] = json!(job.base_sha);
    v["agent"] = json!({ "id": job.agent_id, "version": job.agent_version, "placeholder": job.agent_placeholder });
    v["inputs"] = job.inputs.clone();
    v["generator"] = generator_json(job);
    v["steps"] = json!(steps);
    v["error"] = error_json(&job.error_code, &job.error_message, job.error_retryable);
    v["cancel_requested"] = json!(job.cancel_requested);
    v["can_cancel"] = json!(matches!(status, JobStatus::Queued | JobStatus::Running) && !job.cancel_requested);
    v["can_publish"] = json!(can_publish(job, artifacts));
    v["can_retry"] = json!(matches!(status, JobStatus::Failed | JobStatus::Cancelled));
    v["started_at"] = json!(job.started_at);
    v["updated_at"] = json!(job.updated_at);
    v["duration_ms"] = json!(duration_ms);
    v["output"] = output;
    v["usage"] = job.agent_usage.clone().unwrap_or(Value::Null);
    v["publish"] = json!({
        "status": job.publish_status,
        "stage": job.publish_stage().map(PublishStage::as_str),
        "steps": publish_steps,
        "title": job.publish_title,
        "central_available": central_repo.is_some(),
        "central_requested": job.publish_central,
        "central_repo": central_repo,
        "central_branch": job.central_publish_branch,
        "central_commit_sha": job.central_publish_commit_sha,
        "central_pr_number": job.central_publish_pr_number,
        "central_pr_url": job.central_publish_pr_url,
        "branch": job.publish_branch,
        "commit_sha": job.publish_commit_sha,
        "commit_url": commit_url,
        "pr_number": job.publish_pr_number,
        "pr_url": job.publish_pr_url,
        "error": error_json(&job.publish_error_code, &job.publish_error_message, job.publish_error_retryable),
        "updated_at": job.publish_updated_at,
    });
    // Normalise unknown stored values defensively.
    if GenerationStage::parse(&job.stage).is_none() {
        v["stage"] = json!("queued");
    }
    v
}
