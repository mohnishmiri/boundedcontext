//! Publish pipeline: checking_base → creating_commit → creating_branch → creating_pull_request.
//! Every step persists its result before the next starts, so a retried task resumes instead of repeating,
//! existing branches are never force-pushed, and pull requests are never merged.

use bcp_core::domain::{JobStatus, PublishStage};
use bcp_core::redact::Redactor;
use bcp_core::steps::StageTracker;
use bcp_core::{validate, AppError, ErrorCode};
use bcp_github::{GitHubError, TreeFile};
use bcp_store::{actions, JobRow, PublishProgress, TaskRow};
use serde_json::json;

use crate::logger::JobLogger;
use crate::{guard, retry_delay, user_safe, Outcome, RunCtl, Verdict, Worker};

pub const JOB_TRAILER: &str = "Bounded-Context-Job";

pub fn branch_name(repo: &str, short_id: &str) -> String {
    format!("bounded-context/{}-{short_id}", validate::slugify(repo, 40))
}

fn svc(e: GitHubError) -> AppError {
    e.to_service_error()
}

impl Worker {
    pub(crate) async fn publish(&self, task: &TaskRow, ctl: &RunCtl) -> Verdict {
        let store = &self.store;
        let job = match store.get_job(task.job_id).await {
            Ok(Some(j)) => j,
            Ok(None) => return Verdict::Skipped,
            Err(e) => return Verdict::Retry { delay: retry_delay(task.attempts, None), error: e.to_string() },
        };
        if job.status() != JobStatus::Succeeded {
            return Verdict::Skipped;
        }
        let already = store.count_events(job.id).await.unwrap_or(0);
        let logger = JobLogger::new(store.clone(), job.id, already, self.cfg.max_log_lines + 200, Redactor::new());
        logger.set_stage("publishing");
        logger.info(&format!("Publishing (attempt {}/{})", task.attempts, task.max_attempts)).await;
        let mut steps = StageTracker::default();

        let result = self.publish_inner(&job, ctl, &mut steps, &logger).await.map_err(|o| match o {
            Outcome::Failed(e) => Outcome::Failed(user_safe(e, job.id)),
            other => other,
        });
        match result {
            Ok(()) => {
                metrics::counter!("bcp_publish_total", "outcome" => "succeeded").increment(1);
                Verdict::Succeeded
            }
            Err(Outcome::Failed(err)) if err.retryable && task.attempts_left() => {
                let delay = retry_delay(task.attempts, err.retry_after);
                logger
                    .warn(&format!(
                        "Publishing attempt {} failed: {} Retrying in {}s.",
                        task.attempts,
                        err.message,
                        delay.as_secs()
                    ))
                    .await;
                let _ = store.requeue_publish(job.id).await;
                Verdict::Retry { delay, error: format!("{}: {}", err.code, err.message) }
            }
            Err(Outcome::Failed(err)) => {
                steps.fail();
                logger.error(&format!("Publishing failed: {}", err.message)).await;
                if let Err(e) = store.fail_publish(job.id, &err, &steps.to_json()).await {
                    tracing::error!(error = %e, "failed to record publish failure");
                }
                metrics::counter!("bcp_publish_total", "outcome" => "failed", "code" => err.code.as_str()).increment(1);
                self.audit(&job, actions::PUBLISH_FAILED, json!({ "code": err.code })).await;
                Verdict::Failed(err)
            }
            Err(Outcome::Released) | Err(Outcome::Cancelled) => {
                // Publishing cannot be cancelled by users; an interruption means shutdown. Resume later.
                let _ = store.requeue_publish(job.id).await;
                Verdict::Release
            }
            Err(Outcome::LeaseLost) => Verdict::Abandon,
        }
    }

    async fn publish_stage(
        &self,
        job: &JobRow,
        steps: &mut StageTracker,
        log: &JobLogger,
        stage: PublishStage,
    ) -> Result<(), Outcome> {
        steps.start(stage.as_str());
        self.store.set_publish_stage(job.id, stage, &steps.to_json()).await.map_err(|e| Outcome::Failed(e.into()))?;
        log.info(&format!("{}…", stage.label())).await;
        Ok(())
    }

    async fn publish_inner(
        &self,
        job: &JobRow,
        ctl: &RunCtl,
        steps: &mut StageTracker,
        log: &JobLogger,
    ) -> Result<(), Outcome> {
        let gh = &self.github;
        let store = &self.store;
        let (owner, repo) = (job.owner.as_str(), job.repo.as_str());

        // ---- checking_base -----------------------------------------------------------------------------
        self.publish_stage(job, steps, log, PublishStage::CheckingBase).await?;
        let requester = job.publish_requested_by.unwrap_or(job.user_id);
        let user_token = guard(ctl, self.tokens.access_token(requester)).await?;
        let gh_repo =
            guard(ctl, async { gh.get_repository(&user_token, owner, repo).await.map_err(|e| e.to_user_error()) })
                .await?;
        if !gh_repo.permissions().push {
            return Err(Outcome::Failed(
                AppError::new(
                    ErrorCode::RepositoryAccessDenied,
                    "You need write access to this repository to open a pull request.",
                )
                .retryable(false),
            ));
        }
        let requester_login = match store.get_user(requester).await {
            Ok(Some(u)) => (u.login, u.github_id),
            _ => (job.requested_by.clone(), 0),
        };
        let files: Vec<TreeFile> = store
            .list_artifacts(job.id)
            .await
            .map_err(|e| Outcome::Failed(e.into()))?
            .into_iter()
            .filter(|a| a.allowed && validate::relative_path(&a.path).is_ok() && self.bundle.matches_output(&a.path))
            .filter_map(|a| a.new_content.map(|content| TreeFile { path: a.path, content }))
            .collect();
        if files.is_empty() {
            return Err(Outcome::Failed(
                AppError::conflict("There is nothing to publish for this job.").retryable(false),
            ));
        }
        let base_sha =
            job.base_sha.clone().ok_or_else(|| Outcome::Failed(AppError::internal("job has no analysed commit")))?;
        let installation = match job.installation_id {
            Some(i) => i,
            None => {
                guard(ctl, async { gh.get_repository_installation(owner, repo).await.map(|i| i.id).map_err(svc) })
                    .await?
            }
        };
        let token = guard(ctl, async {
            gh.create_installation_token(installation, repo, &[("contents", "write"), ("pull_requests", "write")])
                .await
                .map_err(svc)
        })
        .await?
        .token;

        // Always re-checked, even when a commit exists from an earlier attempt: the base may have moved since.
        {
            let head =
                guard(ctl, async { gh.get_branch_head(&token, owner, repo, &job.base_branch).await.map_err(svc) })
                    .await?;
            let Some(head) = head else {
                return Err(Outcome::Failed(
                    AppError::new(
                        ErrorCode::BaseBranchChanged,
                        format!("The branch '{}' no longer exists.", job.base_branch),
                    )
                    .retryable(false),
                ));
            };
            if head != base_sha {
                for f in &files {
                    let (at_base, at_head) = guard(ctl, async {
                        let a = gh.get_file_sha(&token, owner, repo, &f.path, &base_sha).await.map_err(svc)?;
                        let b = gh.get_file_sha(&token, owner, repo, &f.path, &head).await.map_err(svc)?;
                        Ok((a, b))
                    })
                    .await?;
                    if at_base != at_head {
                        return Err(Outcome::Failed(
                            AppError::new(
                                ErrorCode::BaseBranchChanged,
                                format!(
                                    "{} was changed on '{}' after the documentation was generated. Generate again so their changes are not overwritten.",
                                    f.path, job.base_branch
                                ),
                            )
                            .retryable(false),
                        ));
                    }
                }
                log.info(&format!(
                    "'{}' moved to {} since the analysis, but the output files are untouched; the pull request is based on the analysed commit {}",
                    job.base_branch,
                    &head[..head.len().min(12)],
                    &base_sha[..base_sha.len().min(12)]
                ))
                .await;
            } else {
                log.info(&format!("'{}' is still at the analysed commit", job.base_branch)).await;
            }
        }

        // ---- creating_commit ---------------------------------------------------------------------------
        self.publish_stage(job, steps, log, PublishStage::CreatingCommit).await?;
        let mut commit_sha = match &job.publish_commit_sha {
            Some(sha) => {
                log.info(&format!("Reusing commit {} from a previous attempt", &sha[..sha.len().min(12)])).await;
                sha.clone()
            }
            None => {
                let base_commit =
                    guard(ctl, async { gh.get_git_commit(&token, owner, repo, &base_sha).await.map_err(svc) }).await?;
                let tree = guard(ctl, async {
                    gh.create_tree(&token, owner, repo, &base_commit.tree.sha, &files).await.map_err(svc)
                })
                .await?;
                let (login, github_id) = &requester_login;
                let message = format!(
                    "docs: add bounded-context documentation\n\n\
                     Generated by the Bounded Context Portal from {} on {}\n\
                     with agent {} (bundle {}) and model {}.\n\n\
                     {JOB_TRAILER}: {}\n\
                     Co-authored-by: {login} <{github_id}+{login}@users.noreply.github.com>\n",
                    &base_sha[..base_sha.len().min(12)],
                    job.base_branch,
                    job.agent_id,
                    job.agent_version,
                    job.model,
                    job.id,
                );
                let commit = guard(ctl, async {
                    gh.create_commit(&token, owner, repo, &message, &tree, &[&base_sha]).await.map_err(svc)
                })
                .await?;
                store
                    .save_publish_progress(
                        job.id,
                        &PublishProgress { commit_sha: Some(commit.sha.clone()), ..Default::default() },
                    )
                    .await
                    .map_err(|e| Outcome::Failed(e.into()))?;
                self.audit(job, actions::COMMIT_CREATED, json!({ "commit_sha": commit.sha })).await;
                log.info(&format!("Created commit {}", &commit.sha[..commit.sha.len().min(12)])).await;
                commit.sha
            }
        };

        // ---- creating_branch ---------------------------------------------------------------------------
        self.publish_stage(job, steps, log, PublishStage::CreatingBranch).await?;
        let branch = job.publish_branch.clone().unwrap_or_else(|| branch_name(repo, &job.short_id()));
        store
            .save_publish_progress(job.id, &PublishProgress { branch: Some(branch.clone()), ..Default::default() })
            .await
            .map_err(|e| Outcome::Failed(e.into()))?;
        let mut existing =
            guard(ctl, async { gh.get_branch_head(&token, owner, repo, &branch).await.map_err(svc) }).await?;
        if existing.is_none() {
            match gh.create_branch(&token, owner, repo, &branch, &commit_sha).await {
                Ok(()) => {
                    self.audit(job, actions::BRANCH_CREATED, json!({ "branch": branch, "commit_sha": commit_sha }))
                        .await;
                    log.info(&format!("Created branch {branch}")).await;
                }
                // Lost a race with a previous attempt: inspect what is there now.
                Err(GitHubError::Unprocessable { .. }) => {
                    existing =
                        guard(ctl, async { gh.get_branch_head(&token, owner, repo, &branch).await.map_err(svc) })
                            .await?;
                }
                Err(e) => return Err(Outcome::Failed(svc(e))),
            }
        }
        if let Some(head) = existing {
            if head == commit_sha {
                log.info(&format!("Branch {branch} already points at the commit")).await;
            } else {
                let theirs =
                    guard(ctl, async { gh.get_git_commit(&token, owner, repo, &head).await.map_err(svc) }).await?;
                if theirs.message.contains(&format!("{JOB_TRAILER}: {}", job.id)) {
                    log.info(&format!(
                        "Branch {branch} holds a commit from a previous attempt of this job; reusing it"
                    ))
                    .await;
                    commit_sha = head;
                    store
                        .save_publish_progress(
                            job.id,
                            &PublishProgress { commit_sha: Some(commit_sha.clone()), ..Default::default() },
                        )
                        .await
                        .map_err(|e| Outcome::Failed(e.into()))?;
                } else {
                    return Err(Outcome::Failed(
                        AppError::new(
                            ErrorCode::BranchConflict,
                            format!("A branch named '{branch}' already exists with other commits. The portal never overwrites branches; delete it and try again."),
                        )
                        .retryable(false),
                    ));
                }
            }
        }

        // ---- creating_pull_request ---------------------------------------------------------------------
        self.publish_stage(job, steps, log, PublishStage::CreatingPullRequest).await?;
        let pr = match guard(ctl, async {
            gh.find_open_pull(&token, owner, repo, &branch, &job.base_branch).await.map_err(svc)
        })
        .await?
        {
            Some(pr) => {
                log.info(&format!("Reusing open pull request #{}", pr.number)).await;
                pr
            }
            None => {
                let title = format!("docs: bounded-context documentation for {repo}");
                let body = self.pr_body(job, &base_sha, &requester_login.0, &files);
                match gh.create_pull(&token, owner, repo, &title, &body, &branch, &job.base_branch).await {
                    Ok(pr) => {
                        self.audit(
                            job,
                            actions::PULL_REQUEST_CREATED,
                            json!({ "pr_number": pr.number, "pr_url": pr.html_url }),
                        )
                        .await;
                        log.info(&format!("Opened pull request #{}", pr.number)).await;
                        pr
                    }
                    Err(GitHubError::Unprocessable { message }) => {
                        match guard(ctl, async {
                            gh.find_open_pull(&token, owner, repo, &branch, &job.base_branch).await.map_err(svc)
                        })
                        .await?
                        {
                            Some(pr) => pr,
                            None => {
                                return Err(Outcome::Failed(
                                    AppError::new(
                                        ErrorCode::Conflict,
                                        format!("GitHub refused to open the pull request: {message}"),
                                    )
                                    .retryable(false),
                                ))
                            }
                        }
                    }
                    Err(e) => return Err(Outcome::Failed(svc(e))),
                }
            }
        };
        store
            .save_publish_progress(
                job.id,
                &PublishProgress {
                    pr_number: Some(pr.number as i32),
                    pr_url: Some(pr.html_url.clone()),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| Outcome::Failed(e.into()))?;
        steps.complete();
        store.complete_publish(job.id, &steps.to_json()).await.map_err(|e| Outcome::Failed(e.into()))?;
        log.info(&format!("Pull request ready: {}", pr.html_url)).await;
        Ok(())
    }

    fn pr_body(&self, job: &JobRow, base_sha: &str, login: &str, files: &[TreeFile]) -> String {
        let portal = self.cfg.common.public_url.as_str().trim_end_matches('/');
        let placeholder =
            if job.agent_placeholder { " — ⚠️ placeholder agent, not the production agent" } else { "" };
        let file_list: String = files.iter().map(|f| format!("- `{}`\n", f.path)).collect();
        format!(
            "## Bounded-context documentation\n\n\
             Generated by the **Bounded Context Portal**, requested by {login}.\n\n\
             | | |\n|---|---|\n\
             | Base branch | `{}` |\n\
             | Analysed commit | `{base_sha}` |\n\
             | Agent | `{}` (bundle `{}`){placeholder} |\n\
             | Model | `{}` |\n\
             | Portal job | {portal}/jobs/{} |\n\n\
             ### Files\n\n{file_list}\n\
             > Generated by an AI agent. Review the content before merging. The portal never merges pull requests.\n",
            job.base_branch, job.agent_id, job.agent_version, job.model, job.id,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names_are_valid_refs() {
        let b = branch_name("My.Shop_Repo", "1a2b3c4d");
        assert_eq!(b, "bounded-context/my-shop-repo-1a2b3c4d");
        assert!(validate::branch(&b).is_ok());
        assert!(validate::branch(&branch_name("...", "deadbeef")).is_ok());
    }
}
