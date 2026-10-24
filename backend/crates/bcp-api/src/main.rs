use anyhow::Context;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "bcp-api", version, about = "Bounded Context Portal API")]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Serve the HTTP API (default).
    Serve,
    /// Apply database migrations and exit (Helm pre-install/pre-upgrade hook).
    Migrate,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let _telemetry = bcp_core::telemetry::init("bcp-api", bcp_core::config::log_format())?;
    match cli.command.unwrap_or(Cmd::Serve) {
        Cmd::Serve => bcp_api::serve(bcp_api::ApiConfig::from_env()?).await,
        Cmd::Migrate => {
            // Only the database URL is needed here; the hook pod has no other secrets mounted.
            let url = bcp_core::config::env_secret_req("BCP_DATABASE_URL")?;
            let store = bcp_store::Store::connect(&url, 2).await.context("connecting to PostgreSQL")?;
            store.migrate().await.context("applying migrations")?;
            tracing::info!("migrations applied");
            Ok(())
        }
    }
}
