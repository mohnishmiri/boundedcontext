use clap::{Parser, Subcommand};
use tokio_util::sync::CancellationToken;

use bcp_worker::{Mode, Worker, WorkerConfig};

#[derive(Parser)]
#[command(name = "bcp-worker", version, about = "Bounded Context Portal worker")]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Process tasks until stopped (Deployment).
    Run,
    /// Process at most one task, then exit (KEDA ScaledJob).
    Once,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let _telemetry = bcp_core::telemetry::init("bcp-worker", bcp_core::config::log_format())?;
    let mut cfg = WorkerConfig::from_env()?;
    match cli.command {
        Some(Cmd::Run) => cfg.mode = Mode::Loop,
        Some(Cmd::Once) => cfg.mode = Mode::Once,
        None => {}
    }
    let mode = cfg.mode;
    let (listen, metrics_listen) = (cfg.listen, cfg.metrics_listen);
    let worker = Worker::from_config(cfg).await?;

    let shutdown = CancellationToken::new();
    let s = shutdown.clone();
    tokio::spawn(async move {
        let mut term =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
        tracing::info!("shutdown signal received");
        s.cancel();
    });

    match mode {
        Mode::Loop => {
            bcp_core::telemetry::init_metrics(metrics_listen)?;
            let listener = tokio::net::TcpListener::bind(listen).await?;
            let app = worker.clone().health_router();
            tokio::spawn(async move {
                if let Err(e) = axum::serve(listener, app).await {
                    tracing::error!(error = %e, "health server stopped");
                }
            });
            worker.run_loop(shutdown).await
        }
        Mode::Once => worker.run_once(shutdown).await,
    }
}
