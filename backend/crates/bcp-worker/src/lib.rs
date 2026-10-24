//! The worker: claims tasks from the PostgreSQL queue and runs the generate and publish pipelines.
//!
//! Each running task holds a heartbeat lease. User cancellation, lease loss and shutdown all cancel the same
//! token; the pipeline then reports why it stopped so the task is finished, released for another worker
//! (shutdown — the attempt is not consumed) or abandoned (lease lost — someone else owns it now).

pub mod config;
mod generate;
mod logger;
pub mod publish;

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;
use bcp_agent::{AgentAdapter, AgentBundle, CopilotCliAdapter};
use bcp_core::domain::TaskKind;
use bcp_core::{AppError, ErrorCode};
use bcp_github::GitHubClient;
use bcp_services::generators::GeneratorResolver;
use bcp_services::TokenService;
use bcp_store::{Store, TaskRow, TASKS_CHANNEL};
use rand::Rng;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

pub use config::{Mode, WorkerConfig};

/// Why a pipeline stopped early.
#[derive(Debug)]
pub(crate) enum Outcome {
    Failed(AppError),
    Cancelled,
    Released,
    LeaseLost,
}

/// What to do with the task row.
#[derive(Debug)]
pub enum Verdict {
    Succeeded,
    Failed(AppError),
    Retry {
        delay: Duration,
        error: String,
    },
    Cancelled,
    /// Hand back without consuming the attempt (shutdown).
    Release,
    /// Lease lost: another worker owns the task now; touch nothing.
    Abandon,
    /// The job is no longer runnable (cancelled while queued, deleted).
    Skipped,
}

#[derive(Clone)]
pub(crate) struct RunCtl {
    pub run: CancellationToken,
    pub user_cancel: CancellationToken,
    pub lease_lost: Arc<AtomicBool>,
}

impl RunCtl {
    fn new(hard_stop: &CancellationToken) -> Self {
        let run = CancellationToken::new();
        let ctl = Self {
            run: run.clone(),
            user_cancel: CancellationToken::new(),
            lease_lost: Arc::new(AtomicBool::new(false)),
        };
        let (hs, r) = (hard_stop.clone(), run);
        tokio::spawn(async move {
            tokio::select! {
                _ = hs.cancelled() => r.cancel(),
                _ = r.cancelled() => {}
            }
        });
        ctl
    }

    pub fn stop_reason(&self) -> Outcome {
        if self.lease_lost.load(Ordering::SeqCst) {
            Outcome::LeaseLost
        } else if self.user_cancel.is_cancelled() {
            Outcome::Cancelled
        } else {
            Outcome::Released
        }
    }
}

/// Awaits `fut` unless the run is cancelled first.
pub(crate) async fn guard<T>(ctl: &RunCtl, fut: impl Future<Output = Result<T, AppError>>) -> Result<T, Outcome> {
    tokio::select! {
        r = fut => r.map_err(Outcome::Failed),
        _ = ctl.run.cancelled() => Err(ctl.stop_reason()),
    }
}

/// Internal errors carry diagnostic detail (paths, OS errors) that belongs in logs, not in the job shown to
/// users. Logs the detail and returns a generic, user-safe error with the same code and retry flag.
pub(crate) fn user_safe(err: AppError, job_id: uuid::Uuid) -> AppError {
    if err.code != ErrorCode::Internal {
        return err;
    }
    tracing::error!(%job_id, detail = %err.message, "internal error");
    AppError::new(
        ErrorCode::Internal,
        "An internal error occurred on the worker. The platform team can find details in the worker logs.",
    )
    .retryable(err.retryable)
}

/// Exponential backoff with jitter (20 s, 40 s, 80 s … capped at 10 min) unless GitHub said how long to wait.
pub(crate) fn retry_delay(attempt: i32, retry_after: Option<u64>) -> Duration {
    if let Some(s) = retry_after {
        return Duration::from_secs(s.clamp(1, 3600));
    }
    let base = 20u64.saturating_mul(1 << attempt.clamp(1, 10).saturating_sub(1)).min(600);
    Duration::from_secs(base + rand::thread_rng().gen_range(0..=base / 5))
}

/// Builds the agent runtime for a bundle. Each job runs the generator it was created with, so the worker
/// needs an adapter per bundle rather than one global adapter.
pub type AdapterFactory = Arc<dyn Fn(Arc<AgentBundle>) -> Arc<dyn AgentAdapter> + Send + Sync>;

pub struct Worker {
    pub(crate) cfg: WorkerConfig,
    pub(crate) store: Store,
    pub(crate) github: GitHubClient,
    pub(crate) tokens: TokenService,
    /// The built-in generator (the bundle baked into the image); also used for the runtime preflight.
    pub(crate) bundle: Arc<AgentBundle>,
    pub(crate) adapter: Arc<dyn AgentAdapter>,
    pub(crate) adapters: AdapterFactory,
    pub(crate) resolver: GeneratorResolver,
    runtime_ready: AtomicBool,
}

impl Worker {
    pub fn new(
        cfg: WorkerConfig,
        store: Store,
        github: GitHubClient,
        bundle: Arc<AgentBundle>,
        adapters: AdapterFactory,
    ) -> Self {
        let tokens = TokenService::new(store.clone(), github.clone(), cfg.common.cipher.clone());
        let adapter = adapters(bundle.clone());
        let resolver = GeneratorResolver::new(github.clone(), cfg.work_dir.clone());
        Self { cfg, store, github, tokens, bundle, adapter, adapters, resolver, runtime_ready: AtomicBool::new(false) }
    }

    /// Builds a worker from configuration with the Copilot CLI adapter.
    pub async fn from_config(cfg: WorkerConfig) -> anyhow::Result<Arc<Self>> {
        let store = Store::connect(
            &cfg.common.database_url,
            cfg.common.database_max_connections.max(cfg.concurrency as u32 + 3),
        )
        .await
        .context("connecting to PostgreSQL")?;
        let github = GitHubClient::new(&cfg.common.github)?;
        let bundle = Arc::new(AgentBundle::load(&cfg.common.agent_bundle_dir).context("loading agent bundle")?);
        if bundle.manifest.placeholder {
            tracing::warn!("the agent bundle is a PLACEHOLDER — install the real agent (agent-bundle/README.md)");
        }
        let bin = cfg.copilot_bin.clone();
        let adapters: AdapterFactory = Arc::new(move |b| Arc::new(CopilotCliAdapter::new(b, bin.clone())));
        Ok(Arc::new(Self::new(cfg, store, github, bundle, adapters)))
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Checks the agent runtime; readiness stays false until it succeeds.
    pub async fn preflight(&self) -> bool {
        match self.adapter.preflight().await {
            Ok(version) => {
                if !self.runtime_ready.swap(true, Ordering::SeqCst) {
                    tracing::info!(%version, "agent runtime available");
                }
                true
            }
            Err(e) => {
                self.runtime_ready.store(false, Ordering::SeqCst);
                tracing::error!(error = %e.to_app_error(), "agent runtime preflight failed");
                false
            }
        }
    }

    /// Long-running mode: claim and process tasks until `shutdown`, then drain gracefully.
    pub async fn run_loop(self: Arc<Self>, shutdown: CancellationToken) -> anyhow::Result<()> {
        // Wait for the API's migrations (or the Helm hook) before touching the queue.
        loop {
            match self.store.ready().await {
                Ok(true) => break,
                Ok(false) => tracing::info!("waiting for database migrations"),
                Err(e) => tracing::warn!(error = %e, "database not reachable yet"),
            }
            tokio::select! {
                _ = shutdown.cancelled() => return Ok(()),
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            }
        }
        let hard_stop = CancellationToken::new();
        let sem = Arc::new(Semaphore::new(self.cfg.concurrency));
        let mut running: JoinSet<()> = JoinSet::new();
        tokio::spawn(self.clone().reaper(shutdown.clone()));
        tokio::spawn(self.clone().preflight_loop(shutdown.clone()));
        let mut listener = match self.store.listener(&[TASKS_CHANNEL]).await {
            Ok(l) => Some(l),
            Err(e) => {
                tracing::warn!(error = %e, "LISTEN unavailable; polling only");
                None
            }
        };
        tracing::info!(worker = %self.cfg.worker_id, concurrency = self.cfg.concurrency, "worker started");

        loop {
            while running.try_join_next().is_some() {}
            let permit = tokio::select! {
                _ = shutdown.cancelled() => break,
                p = sem.clone().acquire_owned() => p.expect("semaphore open"),
            };
            if !self.runtime_ready.load(Ordering::SeqCst) {
                drop(permit);
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_secs(5)) => continue,
                }
            }
            match self.claim().await {
                Ok(Some(task)) => {
                    let me = self.clone();
                    let hs = hard_stop.clone();
                    running.spawn(async move {
                        me.process(task, hs).await;
                        drop(permit);
                    });
                }
                Ok(None) => {
                    drop(permit);
                    let wait = async {
                        match listener.as_mut() {
                            Some(l) => {
                                let _ = tokio::time::timeout(Duration::from_secs(5), l.recv()).await;
                            }
                            None => tokio::time::sleep(Duration::from_secs(3)).await,
                        }
                    };
                    tokio::select! {
                        _ = shutdown.cancelled() => break,
                        _ = wait => {}
                    }
                }
                Err(e) => {
                    drop(permit);
                    tracing::warn!(error = %e, "claiming a task failed");
                    tokio::select! {
                        _ = shutdown.cancelled() => break,
                        _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                    }
                }
            }
        }

        if !running.is_empty() {
            tracing::info!(
                tasks = running.len(),
                grace_secs = self.cfg.shutdown_grace.as_secs(),
                "draining running tasks"
            );
            if tokio::time::timeout(self.cfg.shutdown_grace, async { while running.join_next().await.is_some() {} })
                .await
                .is_err()
            {
                tracing::warn!("grace period over; releasing remaining tasks to the queue");
                hard_stop.cancel();
                let _ = tokio::time::timeout(Duration::from_secs(20), async {
                    while running.join_next().await.is_some() {}
                })
                .await;
            }
        }
        tracing::info!("worker stopped");
        Ok(())
    }

    /// One-shot mode (KEDA ScaledJob): wait up to `idle_exit` for a task, run it, return.
    pub async fn run_once(self: Arc<Self>, shutdown: CancellationToken) -> anyhow::Result<()> {
        if !self.preflight().await {
            anyhow::bail!("agent runtime unavailable");
        }
        let _ = self.reap_once().await;
        let deadline = tokio::time::Instant::now() + self.cfg.idle_exit;
        let task = loop {
            if let Some(t) = self.claim().await? {
                break Some(t);
            }
            if tokio::time::Instant::now() >= deadline || shutdown.is_cancelled() {
                break None;
            }
            tokio::select! {
                _ = shutdown.cancelled() => break None,
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            }
        };
        let Some(task) = task else {
            tracing::info!("no task available; exiting");
            return Ok(());
        };
        let hard_stop = CancellationToken::new();
        let me = self.clone();
        let hs = hard_stop.clone();
        let mut handle = tokio::spawn(async move { me.process(task, hs).await });
        tokio::select! {
            _ = &mut handle => {}
            _ = shutdown.cancelled() => {
                // Kubernetes is stopping the pod: release the task promptly for another Job.
                hard_stop.cancel();
                let _ = tokio::time::timeout(Duration::from_secs(30), handle).await;
            }
        }
        Ok(())
    }

    async fn claim(&self) -> Result<Option<TaskRow>, bcp_store::StoreError> {
        let task = self
            .store
            .claim_task(
                &self.cfg.worker_id,
                self.cfg.lease_secs,
                self.cfg.generate_max_attempts,
                self.cfg.publish_max_attempts,
            )
            .await?;
        if let Some(t) = &task {
            tracing::info!(task_id = %t.id, job_id = %t.job_id, kind = %t.kind, attempt = t.attempts, "claimed task");
            metrics::counter!("bcp_tasks_claimed_total", "kind" => t.kind.clone()).increment(1);
        }
        Ok(task)
    }

    /// Runs one claimed task to a verdict and records it on the task row.
    pub async fn process(self: Arc<Self>, task: TaskRow, hard_stop: CancellationToken) {
        let ctl = RunCtl::new(&hard_stop);
        let watcher = tokio::spawn(self.clone().watch(task.clone(), ctl.clone()));
        metrics::gauge!("bcp_tasks_running").increment(1.0);
        let verdict = match task.kind() {
            Some(TaskKind::Generate) => self.generate(&task, &ctl).await,
            Some(TaskKind::Publish) => self.publish(&task, &ctl).await,
            None => Verdict::Failed(AppError::internal(format!("unknown task kind {}", task.kind))),
        };
        metrics::gauge!("bcp_tasks_running").decrement(1.0);
        watcher.abort();
        ctl.run.cancel(); // stops the hard-stop forwarder
        let wid = &self.cfg.worker_id;
        let res = match &verdict {
            Verdict::Succeeded => self.store.finish_task(task.id, wid, "succeeded", None).await,
            Verdict::Failed(e) => {
                self.store.finish_task(task.id, wid, "failed", Some(&format!("{}: {}", e.code, e.message))).await
            }
            Verdict::Retry { delay, error } => {
                self.store.reschedule_task(task.id, wid, delay.as_secs_f64(), error).await
            }
            Verdict::Cancelled | Verdict::Skipped => self.store.finish_task(task.id, wid, "cancelled", None).await,
            Verdict::Release => self.store.release_task(task.id, wid).await,
            Verdict::Abandon => Ok(()),
        };
        if let Err(e) = res {
            tracing::error!(task_id = %task.id, error = %e, "failed to record task outcome; the reaper will recover it");
        }
        tracing::info!(task_id = %task.id, job_id = %task.job_id, verdict = ?verdict, "task finished");
    }

    /// Heartbeats the lease and watches for user cancellation while a task runs.
    async fn watch(self: Arc<Self>, task: TaskRow, ctl: RunCtl) {
        let heartbeat_every = Duration::from_secs((self.cfg.lease_secs / 3).max(5) as u64);
        let mut next_heartbeat = tokio::time::Instant::now() + heartbeat_every;
        let generate = task.kind() == Some(TaskKind::Generate);
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if generate
                && matches!(self.store.is_cancel_requested(task.job_id).await, Ok(true))
                && !ctl.user_cancel.is_cancelled()
            {
                tracing::info!(job_id = %task.job_id, "cancellation requested");
                ctl.user_cancel.cancel();
                ctl.run.cancel();
            }
            if tokio::time::Instant::now() >= next_heartbeat {
                next_heartbeat += heartbeat_every;
                match self.store.heartbeat_task(task.id, &self.cfg.worker_id, self.cfg.lease_secs).await {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::warn!(task_id = %task.id, "lease lost; stopping work on this task");
                        ctl.lease_lost.store(true, Ordering::SeqCst);
                        ctl.run.cancel();
                        return;
                    }
                    Err(e) => tracing::warn!(task_id = %task.id, error = %e, "heartbeat failed"),
                }
            }
        }
    }

    async fn reap_once(&self) -> Result<(), bcp_store::StoreError> {
        for t in self.store.reap_expired_tasks().await? {
            metrics::counter!("bcp_tasks_reaped_total", "requeued" => t.requeued.to_string()).increment(1);
            tracing::warn!(task_id = %t.id, job_id = %t.job_id, requeued = t.requeued, "recovered task from an unresponsive worker");
            let kind = TaskKind::parse(&t.kind);
            if t.requeued {
                match kind {
                    Some(TaskKind::Generate) => self.store.requeue_job(t.job_id).await?,
                    _ => self.store.requeue_publish(t.job_id).await?,
                }
                let _ = self
                    .store
                    .append_event(
                        t.job_id,
                        "warn",
                        "queued",
                        "The worker running this job stopped responding; it was returned to the queue.",
                    )
                    .await;
            } else {
                let err = AppError::new(
                    ErrorCode::AgentFailed,
                    "The worker running this job stopped responding too many times.",
                )
                .retryable(false);
                match kind {
                    Some(TaskKind::Generate) => {
                        let job = self.store.get_job(t.job_id).await?;
                        let times = job.map(|j| j.stage_times).unwrap_or_default();
                        self.store
                            .finish_job_with_error(t.job_id, bcp_core::domain::JobStatus::Failed, &err, &times)
                            .await?
                    }
                    _ => {
                        self.store
                            .fail_publish(
                                t.job_id,
                                &AppError::new(ErrorCode::Internal, "Publishing was interrupted too many times.")
                                    .retryable(true),
                                &serde_json::json!({}),
                            )
                            .await?
                    }
                }
            }
        }
        Ok(())
    }

    async fn reaper(self: Arc<Self>, shutdown: CancellationToken) {
        let mut tick = tokio::time::interval(Duration::from_secs(15));
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = tick.tick() => {
                    if let Err(e) = self.reap_once().await {
                        tracing::warn!(error = %e, "reaper failed");
                    }
                    if let Ok(n) = self.store.pending_task_count().await {
                        metrics::gauge!("bcp_tasks_pending").set(n as f64);
                    }
                }
            }
        }
    }

    async fn preflight_loop(self: Arc<Self>, shutdown: CancellationToken) {
        loop {
            let ok = self.preflight().await;
            let wait = if ok { Duration::from_secs(300) } else { Duration::from_secs(30) };
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = tokio::time::sleep(wait) => {}
            }
        }
    }

    pub fn health_router(self: Arc<Self>) -> Router {
        async fn healthz() -> &'static str {
            "ok"
        }
        async fn readyz(State(w): State<Arc<Worker>>) -> (StatusCode, &'static str) {
            if !w.runtime_ready.load(Ordering::SeqCst) {
                return (StatusCode::SERVICE_UNAVAILABLE, "agent runtime unavailable");
            }
            match w.store.ready().await {
                Ok(true) => (StatusCode::OK, "ready"),
                Ok(false) => (StatusCode::SERVICE_UNAVAILABLE, "migrations pending"),
                Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "database unavailable"),
            }
        }
        Router::new().route("/healthz", get(healthz)).route("/readyz", get(readyz)).with_state(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_honours_retry_after() {
        assert!(retry_delay(1, None) >= Duration::from_secs(20) && retry_delay(1, None) <= Duration::from_secs(24));
        assert!(retry_delay(3, None) >= Duration::from_secs(80));
        assert!(retry_delay(30, None) <= Duration::from_secs(720));
        assert_eq!(retry_delay(1, Some(7)), Duration::from_secs(7));
    }
}
