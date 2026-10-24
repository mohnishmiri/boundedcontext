//! Standalone mock GitHub for local development (docker compose) and the e2e suite. Never deploy it.

use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let addr: SocketAddr = std::env::var("MOCK_GITHUB_LISTEN").unwrap_or_else(|_| "0.0.0.0:9000".into()).parse()?;
    let public =
        std::env::var("MOCK_GITHUB_PUBLIC_URL").unwrap_or_else(|_| format!("http://localhost:{}", addr.port()));
    let mock = mock_github::MockGitHub::start_on(addr, Some(public.clone())).await;
    tracing::warn!(listen = %mock.url, public = %public, "mock GitHub running — local development only");
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
    Ok(())
}
