//! Runs the agent bundle without the web application — for validating a bundle and comparing its output with
//! what the agent produces in VS Code.
//!
//!   bcp-agent-run validate --bundle agent-bundle
//!   bcp-agent-run run --bundle agent-bundle --workspace ../shop --model auto --owner acme --repo shop --branch main

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use bcp_agent::workspace::{capture_base_outputs, collect_outputs, snapshot};
use bcp_agent::{AgentAdapter, AgentBundle, AgentRunRequest, CopilotCliAdapter, LogLevel, ProgressSink, Secret};
use clap::{Parser, Subcommand};
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(name = "bcp-agent-run", about = "Validate or run the bounded-context agent bundle locally")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum Cmd {
    /// Validate bundle.yaml and the agent file.
    Validate {
        #[arg(long, default_value = "agent-bundle")]
        bundle: PathBuf,
    },
    /// Run the agent against a local directory (modified in place — use a scratch checkout).
    Run {
        #[arg(long, default_value = "agent-bundle")]
        bundle: PathBuf,
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long, default_value = "auto")]
        model: String,
        #[arg(long)]
        owner: String,
        #[arg(long)]
        repo: String,
        #[arg(long, default_value = "main")]
        branch: String,
        #[arg(long, default_value = "HEAD")]
        sha: String,
        /// Agent input as name=value (repeatable).
        #[arg(long = "input", value_parser = parse_kv)]
        inputs: Vec<(String, String)>,
        #[arg(long, env = "BCP_COPILOT_BIN", default_value = "copilot")]
        copilot_bin: PathBuf,
        /// Override the bundle's timeout.
        #[arg(long)]
        timeout_secs: Option<u64>,
        /// Print the command line instead of running it.
        #[arg(long)]
        dry_run: bool,
    },
}

fn parse_kv(s: &str) -> Result<(String, String), String> {
    s.split_once('=').map(|(k, v)| (k.to_owned(), v.to_owned())).ok_or_else(|| "expected name=value".to_owned())
}

struct StderrSink;

#[async_trait]
impl ProgressSink for StderrSink {
    async fn log(&self, level: LogLevel, message: &str) {
        eprintln!("[{}] {message}", level.as_str());
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    match Cli::parse().command {
        Cmd::Validate { bundle } => {
            let b = AgentBundle::load(&bundle)?;
            println!("agent:        {} ({})", b.id(), b.display_name());
            println!("version:      {}", b.version);
            println!("placeholder:  {}", b.manifest.placeholder);
            println!("output paths: {}", b.output_paths().join(", "));
            println!("primary:      {}", b.primary_output().unwrap_or("(first matching file)"));
            println!("inputs:       {}", b.inputs().iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(", "));
            println!("model lock:   {}", b.locked_model().unwrap_or("(none — portal selection applies)"));
            if b.manifest.placeholder {
                eprintln!("warning: this is the placeholder agent; install the real agent (agent-bundle/README.md)");
            }
            Ok(())
        }
        Cmd::Run { bundle, workspace, model, owner, repo, branch, sha, inputs, copilot_bin, timeout_secs, dry_run } => {
            for (field, check) in [
                ("owner", bcp_core::validate::owner(&owner)),
                ("repo", bcp_core::validate::repo(&repo)),
                ("branch", bcp_core::validate::branch(&branch)),
                ("model", bcp_core::validate::model_id(&model)),
            ] {
                if let Err(e) = check {
                    bail!("--{field}: {e}");
                }
            }
            let b = Arc::new(AgentBundle::load(&bundle)?);
            let inputs = b
                .validate_inputs(&inputs.into_iter().collect::<BTreeMap<_, _>>())
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let workspace = workspace.canonicalize().context("--workspace")?;
            let scratch = tempfile::tempdir()?;
            let token = std::env::var("COPILOT_GITHUB_TOKEN")
                .or_else(|_| std::env::var("GH_TOKEN"))
                .context("set COPILOT_GITHUB_TOKEN (fine-grained PAT with Copilot Requests, or `gh auth token`)")?;
            let req = AgentRunRequest {
                workspace: workspace.clone(),
                scratch_dir: scratch.path().to_owned(),
                model: Some(model),
                inputs,
                owner,
                repo,
                branch,
                sha,
                copilot_token: Secret::new(token),
                timeout: Duration::from_secs(timeout_secs.unwrap_or(b.manifest.runtime.timeout_secs)),
            };
            let adapter = CopilotCliAdapter::new(b.clone(), copilot_bin);
            if dry_run {
                let cl = adapter.command_line(&req);
                println!(
                    "{} {}",
                    cl.program.display(),
                    cl.args.iter().map(|a| format!("{a:?}")).collect::<Vec<_>>().join(" ")
                );
                return Ok(());
            }
            let before = snapshot(&workspace)?;
            let base = capture_base_outputs(&b, &workspace, &before);
            let cancel = CancellationToken::new();
            let c2 = cancel.clone();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                c2.cancel();
            });
            let outcome =
                adapter.run(req, &StderrSink, cancel).await.map_err(|e| anyhow::anyhow!("{}", e.to_app_error()))?;
            let after = snapshot(&workspace)?;
            let report = collect_outputs(&b, &workspace, &before, &base, &after).map_err(|e| anyhow::anyhow!("{e}"))?;
            println!(
                "finished in {:.1}s (Copilot CLI {})",
                outcome.duration.as_secs_f64(),
                outcome.cli_version.unwrap_or_default()
            );
            println!("primary output: {}", report.primary);
            for f in &report.files {
                println!("  {} {} ({} bytes)", f.change_type, f.path, f.size_bytes);
            }
            for u in &report.unexpected {
                println!("  UNEXPECTED {} {} (would not be committed)", u.change, u.path);
            }
            if let Some(t) = outcome.transcript_path {
                let keep = std::env::temp_dir().join("bcp-agent-run-transcript.md");
                std::fs::copy(&t, &keep)?;
                println!("transcript: {}", keep.display());
            }
            Ok(())
        }
    }
}
