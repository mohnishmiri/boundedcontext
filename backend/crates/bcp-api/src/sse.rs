//! `GET /api/bounded-context/jobs/{id}/events` — Server-Sent Events with job snapshots and log lines.
//!
//! Wake-ups come from PostgreSQL `LISTEN bcp_jobs` (fanned out per pod through a broadcast channel), with a
//! slow poll as a safety net. The stream ends once the job and its publish sub-state are settled; clients
//! close their `EventSource` on that final snapshot.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::{header, HeaderValue};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use bcp_core::AppError;
use futures::Stream;
use tokio::sync::broadcast::error::RecvError;
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::error::ApiResult;
use crate::jobs::job_json;
use crate::state::AppState;

const POLL_EVERY: Duration = Duration::from_secs(5);
const PING_EVERY: Duration = Duration::from_secs(15);
const LOG_BATCH: i64 = 500;

pub async fn events(State(state): State<AppState>, user: CurrentUser, Path(id): Path<String>) -> ApiResult<Response> {
    let id = Uuid::parse_str(&id).map_err(|_| AppError::not_found("Job"))?;
    let job = state.store.get_user_job(user.user.id, id).await?;
    // Subscribe before taking the snapshot so no change between the two is missed.
    let rx = state.job_events.subscribe();
    let start_seq = state.store.tail_events(id, 1).await?.last().map(|e| e.seq).unwrap_or(0);
    let first = job_json(&state, &job).await?;
    metrics::gauge!("bcp_sse_streams").increment(1.0);
    let stream = event_stream(state, id, rx, first, job.updated_at, start_seq, job.is_settled());
    let mut resp = Sse::new(stream).into_response();
    let h = resp.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    h.insert("x-accel-buffering", HeaderValue::from_static("no"));
    Ok(resp)
}

struct StreamGuard;

impl Drop for StreamGuard {
    fn drop(&mut self) {
        metrics::gauge!("bcp_sse_streams").decrement(1.0);
    }
}

fn event_stream(
    state: AppState,
    id: Uuid,
    mut rx: tokio::sync::broadcast::Receiver<Uuid>,
    first: serde_json::Value,
    mut last_updated: chrono::DateTime<chrono::Utc>,
    mut last_seq: i64,
    settled: bool,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        let _guard = StreamGuard;
        yield Ok(Event::default().event("job").json_data(&first).unwrap_or_default());
        if settled {
            return;
        }
        let mut poll = tokio::time::interval(POLL_EVERY);
        let mut ping = tokio::time::interval(PING_EVERY);
        poll.reset();
        ping.reset();
        loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Ok(changed) if changed != id => continue,
                    Ok(_) | Err(RecvError::Lagged(_)) => {
                        // Coalesce bursts (many log lines) into one refresh.
                        while rx.try_recv().is_ok() {}
                    }
                    Err(RecvError::Closed) => break,
                },
                _ = poll.tick() => {}
                _ = ping.tick() => {
                    yield Ok(Event::default().event("ping").data("{}"));
                    continue;
                }
            }
            loop {
                match state.store.list_events(id, last_seq, LOG_BATCH).await {
                    Ok(batch) => {
                        let full = batch.len() as i64 == LOG_BATCH;
                        for e in batch {
                            last_seq = e.seq;
                            yield Ok(Event::default().event("log").json_data(&e).unwrap_or_default());
                        }
                        if !full { break; }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "SSE log refresh failed");
                        break;
                    }
                }
            }
            match state.store.get_job(id).await {
                Ok(Some(job)) => {
                    if job.updated_at != last_updated {
                        last_updated = job.updated_at;
                        if let Ok(v) = job_json(&state, &job).await {
                            yield Ok(Event::default().event("job").json_data(&v).unwrap_or_default());
                        }
                    }
                    if job.is_settled() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(e) => tracing::warn!(error = %e, "SSE job refresh failed"),
            }
        }
    }
}

/// Forwards `NOTIFY bcp_jobs` payloads (job ids) to SSE subscribers, reconnecting with backoff.
pub async fn run_notifier(state: AppState, shutdown: tokio_util::sync::CancellationToken) {
    let mut backoff = Duration::from_millis(500);
    loop {
        if shutdown.is_cancelled() {
            return;
        }
        match state.store.listener(&[bcp_store::JOBS_CHANNEL]).await {
            Ok(mut listener) => {
                backoff = Duration::from_millis(500);
                loop {
                    tokio::select! {
                        _ = shutdown.cancelled() => return,
                        n = listener.recv() => match n {
                            Ok(n) => {
                                if let Ok(id) = Uuid::parse_str(n.payload()) {
                                    let _ = state.job_events.send(id);
                                }
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, "job notification listener lost; reconnecting");
                                break;
                            }
                        }
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "could not LISTEN for job notifications"),
        }
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}
