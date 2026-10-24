//! Generation pipeline: preparing → fetching_repository → running_agent → validating_output → completed.

use std::collections::BTreeMap;
use std::time::Instant;

use bcp_agent::workspace::{capture_base_outputs, collect_outputs, extract_tarball, snapshot, ExtractLimits};
use bcp_agent::{AgentAdapter, AgentBundle, AgentError, AgentRunRequest, Secret};
use bcp_core::domain::{ChangeType, GenerationStage, JobStatus};
use bcp_core::generators::GeneratorRepo;
use bcp_core::redact::Redactor;
use bcp_core::steps::StageTracker;
use bcp_core::{AppError, ErrorCode};
use bcp_github::GitHubError;
use bcp_services::generators::ResolvedGenerator;
use bcp_store::{actions, JobRow, NewArtifact, NewAuditEvent, TaskRow};
use serde_json::json;
use std::sync::Arc;

use crate::logger::JobLogger;
use crate::{guard, retry_delay, user_safe, Outcome, RunCtl, Verdict, Worker};

fn blocking_err(e: tokio::task::JoinError) -> Outcome {
    Outcome::Failed(AppError::internal(format!("background task failed: {e}")))
}

impl Worker {
    pub(crate) async fn generate(&self, task: &TaskRow, ctl: &RunCtl) -> Verdict {
        let store = &self.store;
        let job = match store.get_job(task.job_id).await {
            Ok(Some(j)) => j,
            Ok(None) => return Verdict::Skipped,
            Err(e) => return Verdict::Retry { delay: retry_delay(task.attempts, None), error: e.to_string() },
        };
        let mut steps = StageTracker::default();
        steps.mark_done("queued", job.created_at);
        steps.start(GenerationStage::Preparing.as_str());
        let job = match store.start_job(job.id, &steps.to_json()).await {
            Ok(Some(j)) => j,
            Ok(None) => return Verdict::Skipped, // cancelled while queued
            Err(e) => return Verdict::Retry { delay: retry_delay(task.attempts, None), error: e.to_string() },
        };
        let already = store.count_events(job.id).await.unwrap_or(0);
        let logger = JobLogger::new(store.clone(), job.id, already, self.cfg.max_log_lines, Redactor::new());
        logger.set_stage(GenerationStage::Preparing.as_str());
        logger
            .info(&format!("Worker {} started attempt {}/{}", self.cfg.worker_id, task.attempts, task.max_attempts))
            .await;
        let started = Instant::now();

        let result = self.generate_inner(&job, ctl, &mut steps, &logger).await.map_err(|o| match o {
            Outcome::Failed(e) => Outcome::Failed(user_safe(e, job.id)),
            other => other,
        });
        metrics::histogram!("bcp_generation_seconds").record(started.elapsed().as_secs_f64());

        match result {
            Ok(()) => {
                metrics::counter!("bcp_generations_total", "outcome" => "succeeded").increment(1);
                self.audit(&job, actions::GENERATION_SUCCEEDED, json!({ "attempt": task.attempts })).await;
                Verdict::Succeeded
            }
            Err(Outcome::Failed(err)) if err.retryable && task.attempts_left() => {
                let delay = retry_delay(task.attempts, err.retry_after);
                logger
                    .warn(&format!(
                        "Attempt {} failed: {} Retrying in {}s.",
                        task.attempts,
                        err.message,
                        delay.as_secs()
                    ))
                    .await;
                if let Err(e) = store.requeue_job(job.id).await {
                    tracing::error!(error = %e, "failed to requeue job");
                }
                metrics::counter!("bcp_generations_total", "outcome" => "retried").increment(1);
                Verdict::Retry { delay, error: format!("{}: {}", err.code, err.message) }
            }
            Err(Outcome::Failed(err)) => {
                steps.fail();
                logger.error(&format!("Generation failed: {}", err.message)).await;
                if let Err(e) = store.finish_job_with_error(job.id, JobStatus::Failed, &err, &steps.to_json()).await {
                    tracing::error!(error = %e, "failed to record job failure");
                }
                metrics::counter!("bcp_generations_total", "outcome" => "failed", "code" => err.code.as_str())
                    .increment(1);
                self.audit(&job, actions::GENERATION_FAILED, json!({ "code": err.code, "attempt": task.attempts }))
                    .await;
                Verdict::Failed(err)
            }
            Err(Outcome::Cancelled) => {
                steps.fail();
                logger.warn("Cancelled by the user.").await;
                let err = AppError::new(ErrorCode::Cancelled, "The generation was cancelled.").retryable(false);
                if let Err(e) = store.finish_job_with_error(job.id, JobStatus::Cancelled, &err, &steps.to_json()).await
                {
                    tracing::error!(error = %e, "failed to record cancellation");
                }
                metrics::counter!("bcp_generations_total", "outcome" => "cancelled").increment(1);
                Verdict::Cancelled
            }
            Err(Outcome::Released) => {
                logger.warn("The worker is shutting down; the job was returned to the queue and will restart.").await;
                if let Err(e) = store.requeue_job(job.id).await {
                    tracing::error!(error = %e, "failed to requeue job on shutdown");
                }
                Verdict::Release
            }
            Err(Outcome::LeaseLost) => Verdict::Abandon,
        }
    }

    async fn generate_inner(
        &self,
        job: &JobRow,
        ctl: &RunCtl,
        steps: &mut StageTracker,
        log: &JobLogger,
    ) -> Result<(), Outcome> {
        let gh = &self.github;
        let store = &self.store;

        // ---- preparing ---------------------------------------------------------------------------------
        let token = guard(ctl, self.tokens.access_token(job.user_id)).await?;
        let (bundle, adapter, _generator_files) = self.generator_for(job, &token, ctl, log).await?;
        let repo =
            guard(ctl, async { gh.get_repository(&token, &job.owner, &job.repo).await.map_err(|e| e.to_user_error()) })
                .await?;
        if !repo.permissions().pull {
            return Err(Outcome::Failed(AppError::new(
                ErrorCode::RepositoryAccessDenied,
                "You no longer have read access to this repository.",
            )));
        }
        let branch = guard(ctl, async {
            match gh.get_branch(&token, &job.owner, &job.repo, &job.base_branch).await {
                Err(GitHubError::NotFound) => Err(AppError::new(
                    ErrorCode::NotFound,
                    format!("The branch '{}' no longer exists.", job.base_branch),
                )
                .retryable(false)),
                other => other.map_err(|e| e.to_user_error()),
            }
        })
        .await?;
        let sha = branch.commit.sha;
        let installation = if gh.has_app() {
            Some(
                guard(ctl, async {
                    match gh.get_repository_installation(&job.owner, &job.repo).await {
                        Err(GitHubError::NotFound) => Err(AppError::new(
                            ErrorCode::AppNotInstalled,
                            "The portal's GitHub App is no longer installed on this repository.",
                        )),
                        other => other.map(|i| i.id).map_err(|e| e.to_service_error()),
                    }
                })
                .await?,
            )
        } else {
            None
        };
        guard(ctl, async {
            Ok(store.set_job_source(job.id, &sha, repo.id, installation, repo.permissions().push).await?)
        })
        .await?;
        log.info(&format!("Resolved {} to commit {}", job.base_branch, &sha[..sha.len().min(12)])).await;

        // ---- fetching_repository -----------------------------------------------------------------------
        self.enter(job, steps, log, GenerationStage::FetchingRepository).await?;
        let fetch_token = match installation {
            Some(inst_id) => {
                guard(ctl, async {
                    gh.create_installation_token(inst_id, &job.repo, &[("contents", "read")])
                        .await
                        .map(|t| t.token)
                        .map_err(|e| e.to_service_error())
                })
                .await?
            }
            None => token.clone(),
        };
        tokio::fs::create_dir_all(&self.cfg.work_dir)
            .await
            .map_err(|e| Outcome::Failed(AppError::internal(format!("work dir: {e}"))))?;
        let scratch = tempfile::Builder::new()
            .prefix(&format!("job-{}-", job.short_id()))
            .tempdir_in(&self.cfg.work_dir)
            .map_err(|e| Outcome::Failed(AppError::internal(format!("creating scratch dir: {e}"))))?;
        let tgz = scratch.path().join("snapshot.tar.gz");
        let workspace = scratch.path().join("workspace");
        let agent_dir = scratch.path().join("agent");
        let fetch_started = Instant::now();
        let bytes = guard(ctl, async {
            gh.download_tarball(&fetch_token, &job.owner, &job.repo, &sha, &tgz, self.cfg.max_repo_bytes)
                .await
                .map_err(|e| e.to_service_error())
        })
        .await?;
        drop(fetch_token);
        let limits = ExtractLimits { max_bytes: self.cfg.max_repo_bytes, max_files: self.cfg.max_repo_files };
        let (tgz2, ws2) = (tgz.clone(), workspace.clone());
        let stats = tokio::task::spawn_blocking(move || extract_tarball(&tgz2, &ws2, limits))
            .await
            .map_err(blocking_err)?
            .map_err(Outcome::Failed)?;
        let _ = tokio::fs::remove_file(&tgz).await;
        log.info(&format!(
            "Fetched snapshot: {} files, {:.1} MiB extracted ({:.1} MiB download) in {:.1}s",
            stats.files,
            stats.bytes as f64 / 1_048_576.0,
            bytes as f64 / 1_048_576.0,
            fetch_started.elapsed().as_secs_f64()
        ))
        .await;
        if stats.skipped_special + stats.skipped_unsafe > 0 {
            log.info(&format!(
                "Skipped {} symlink/special entries and {} entries with unsafe paths",
                stats.skipped_special, stats.skipped_unsafe
            ))
            .await;
        }
        if ctl.run.is_cancelled() {
            return Err(ctl.stop_reason());
        }
        let ws3 = workspace.clone();
        let bundle_for_base = bundle.clone();
        let (before, base) = tokio::task::spawn_blocking(move || {
            let before = snapshot(&ws3)?;
            let base = capture_base_outputs(&bundle_for_base, &ws3, &before);
            Ok::<_, std::io::Error>((before, base))
        })
        .await
        .map_err(blocking_err)?
        .map_err(|e| Outcome::Failed(AppError::internal(format!("snapshot: {e}"))))?;

        // ---- running_agent -----------------------------------------------------------------------------
        self.enter(job, steps, log, GenerationStage::RunningAgent).await?;
        std::fs::create_dir_all(&agent_dir)
            .map_err(|e| Outcome::Failed(AppError::internal(format!("agent dir: {e}"))))?;
        let inputs: BTreeMap<String, String> = serde_json::from_value(job.inputs.clone()).unwrap_or_default();
        let timeout =
            std::time::Duration::from_secs(bundle.manifest.runtime.timeout_secs).min(self.cfg.max_job_timeout);
        let req = AgentRunRequest {
            workspace: workspace.clone(),
            scratch_dir: agent_dir,
            model: if bundle.locked_model().is_some() { None } else { Some(job.model.clone()) },
            inputs,
            owner: job.owner.clone(),
            repo: job.repo.clone(),
            branch: job.base_branch.clone(),
            sha: sha.clone(),
            copilot_token: Secret::new(token),
            timeout,
        };
        let agent_started = Instant::now();
        let outcome = adapter.run(req, log, ctl.run.clone()).await;
        metrics::histogram!("bcp_agent_run_seconds").record(agent_started.elapsed().as_secs_f64());
        let outcome = match outcome {
            Ok(o) => o,
            Err(AgentError::Cancelled) => return Err(ctl.stop_reason()),
            Err(e) => return Err(Outcome::Failed(e.to_app_error())),
        };
        if let Some(v) = &outcome.cli_version {
            let _ = store.set_job_cli_version(job.id, v).await;
        }

        // ---- validating_output -------------------------------------------------------------------------
        self.enter(job, steps, log, GenerationStage::ValidatingOutput).await?;
        let ws4 = workspace.clone();
        let bundle_for_output = bundle.clone();
        let report = tokio::task::spawn_blocking(move || {
            let after = snapshot(&ws4).map_err(|e| AppError::internal(format!("snapshot: {e}")))?;
            collect_outputs(&bundle_for_output, &ws4, &before, &base, &after)
        })
        .await
        .map_err(blocking_err)?
        .map_err(Outcome::Failed)?;
        for u in &report.unexpected {
            log.warn(&format!(
                "Ignored change outside the allowed output paths: {} {} (never committed)",
                u.change, u.path
            ))
            .await;
        }
        let mut artifacts: Vec<NewArtifact> = report
            .files
            .iter()
            .map(|f| NewArtifact {
                path: f.path.clone(),
                change_type: f.change_type.as_str().to_owned(),
                allowed: true,
                base_content: f.base_content.clone(),
                new_content: Some(f.new_content.clone()),
                unified_diff: Some(f.unified_diff.clone()),
                size_bytes: f.size_bytes as i64,
                sha256: Some(f.sha256.clone()),
            })
            .collect();
        artifacts.extend(report.unexpected.iter().take(500).map(|u| NewArtifact {
            path: u.path.clone(),
            change_type: u.change.as_str().to_owned(),
            allowed: false,
            base_content: None,
            new_content: None,
            unified_diff: None,
            size_bytes: 0,
            sha256: None,
        }));
        let primary_size = report.files.iter().find(|f| f.path == report.primary).map(|f| f.size_bytes).unwrap_or(0);
        steps.complete();
        guard(ctl, async { Ok(store.complete_job(job.id, &report.primary, &artifacts, &steps.to_json()).await?) })
            .await?;
        let added = report.files.iter().filter(|f| f.change_type == ChangeType::Added).count();
        log.info(&format!(
            "Generated {} ({} bytes); {} file(s) ready for review ({} new)",
            report.primary,
            primary_size,
            report.files.len(),
            added
        ))
        .await;
        Ok(())
    }

    /// The bundle and runtime for this job's generator. Repository generators are downloaded at the exact
    /// commit pinned when the job was created and re-checked against the platform's hard limits.
    async fn generator_for(
        &self,
        job: &JobRow,
        token: &str,
        ctl: &RunCtl,
        log: &JobLogger,
    ) -> Result<(Arc<AgentBundle>, Arc<dyn AgentAdapter>, Option<Arc<ResolvedGenerator>>), Outcome> {
        if job.generator_source == "builtin" {
            return Ok((self.bundle.clone(), self.adapter.clone(), None));
        }
        let (Some(owner), Some(repo), Some(sha)) =
            (job.generator_owner.clone(), job.generator_repo.clone(), job.generator_sha.clone())
        else {
            return Err(Outcome::Failed(
                AppError::new(ErrorCode::GeneratorInvalid, "The job does not record which generator revision to run.")
                    .retryable(false),
            ));
        };
        let repo = GeneratorRepo {
            owner,
            repo,
            branch: job.generator_branch.clone(),
            path: job.generator_path.clone().unwrap_or_default(),
        };
        let branch = repo.branch.clone().unwrap_or_default();
        let resolved = guard(ctl, self.resolver.load(token, &repo, &branch, &sha)).await?;
        for note in &resolved.notes {
            log.info(&format!("Generator policy: {note}")).await;
        }
        if resolved.bundle.version != job.agent_version {
            log.warn(&format!(
                "Generator bundle {} differs from the one validated at request time ({}).",
                resolved.bundle.version, job.agent_version
            ))
            .await;
        }
        log.info(&format!(
            "Using generator {} at {} (bundle {})",
            repo.display(),
            &sha[..sha.len().min(12)],
            resolved.bundle.version
        ))
        .await;
        let adapter = (self.adapters)(resolved.bundle.clone());
        Ok((resolved.bundle.clone(), adapter, Some(resolved)))
    }

    async fn enter(
        &self,
        job: &JobRow,
        steps: &mut StageTracker,
        log: &JobLogger,
        stage: GenerationStage,
    ) -> Result<(), Outcome> {
        steps.start(stage.as_str());
        log.set_stage(stage.as_str());
        self.store.set_job_stage(job.id, stage, &steps.to_json()).await.map_err(|e| Outcome::Failed(e.into()))?;
        log.info(&format!("{}…", stage.label())).await;
        Ok(())
    }

    pub(crate) async fn audit(&self, job: &JobRow, action: &str, details: serde_json::Value) {
        self.store
            .audit_best_effort(NewAuditEvent {
                actor_user_id: Some(job.user_id),
                actor_login: Some(&job.requested_by),
                action,
                job_id: Some(job.id),
                owner: Some(&job.owner),
                repo: Some(&job.repo),
                details,
                request_id: None,
            })
            .await;
    }
}
