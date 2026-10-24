use std::collections::HashMap;
use std::ops::Deref;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bcp_agent::{AgentAdapter, AgentBundle, AgentDescriptor, CopilotCliAdapter};
use bcp_core::models::ModelCatalog;
use bcp_github::GitHubClient;
use bcp_services::TokenService;
use bcp_store::Store;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::config::ApiConfig;
use crate::middleware::RateLimiter;

#[derive(Clone)]
pub struct AppState(Arc<Inner>);

pub struct Inner {
    pub cfg: ApiConfig,
    pub store: Store,
    pub github: GitHubClient,
    pub tokens: TokenService,
    pub bundle: Arc<AgentBundle>,
    pub agent: AgentDescriptor,
    pub catalog: ModelCatalog,
    /// Job ids whose row or log changed (fed by PostgreSQL LISTEN).
    pub job_events: broadcast::Sender<Uuid>,
    pub cache: TtlCache,
    pub limiter: RateLimiter,
}

impl Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

impl AppState {
    pub fn new(
        cfg: ApiConfig,
        store: Store,
        github: GitHubClient,
        bundle: Arc<AgentBundle>,
        catalog: ModelCatalog,
    ) -> Self {
        let tokens = TokenService::new(store.clone(), github.clone(), cfg.common.cipher.clone());
        // The API never runs the agent; the adapter is only used to describe it.
        let agent = CopilotCliAdapter::new(bundle.clone(), "copilot").describe();
        let (job_events, _) = broadcast::channel(1024);
        let limiter = RateLimiter::new(cfg.rate_limit_per_minute);
        Self(Arc::new(Inner {
            cfg,
            store,
            github,
            tokens,
            bundle,
            agent,
            catalog,
            job_events,
            cache: TtlCache::default(),
            limiter,
        }))
    }
}

type CacheEntry = (Instant, Arc<dyn std::any::Any + Send + Sync>);

/// Small per-pod cache for GitHub listings (repositories, branches, installations), keyed per user so
/// nobody sees another user's view. Entries expire quickly; correctness never depends on them.
#[derive(Default)]
pub struct TtlCache {
    entries: Mutex<HashMap<String, CacheEntry>>,
}

impl TtlCache {
    pub fn get<T: Send + Sync + 'static>(&self, key: &str, ttl: Duration) -> Option<Arc<T>> {
        let map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let (at, value) = map.get(key)?;
        if at.elapsed() > ttl {
            return None;
        }
        value.clone().downcast::<T>().ok()
    }

    pub fn put<T: Send + Sync + 'static>(&self, key: String, value: Arc<T>) {
        let mut map = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > 10_000 {
            map.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(300));
        }
        map.insert(key, (Instant::now(), value));
    }
}
