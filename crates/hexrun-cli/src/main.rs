use std::net::SocketAddr;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "hexrun", about = "NPU-first local LLM runtime for Snapdragon X Elite", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Download a model from the registry
    Pull { model: String },
    /// List locally cached models
    List,
    /// Remove a locally cached model
    Rm { model: String },
    /// Show model manifest and runtime stats
    Show {
        model: String,
        #[arg(long)]
        profile: bool,
    },
    /// Run a one-shot generation or interactive REPL
    Run {
        model: String,
        #[arg(trailing_var_arg = true)]
        prompt: Vec<String>,
    },
    /// List in-flight sessions on a running `hexrun serve`
    Ps,
    /// Start the OpenAI- and Ollama-compatible HTTP server
    Serve {
        #[arg(long, default_value = "127.0.0.1:11435")]
        bind: SocketAddr,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Pull { model } => {
            println!("pull: {model} (Phase 3 — not yet implemented)");
        }
        Cmd::List => {
            for name in hexrun_registry::list_local().await? {
                println!("{name}");
            }
        }
        Cmd::Rm { model } => {
            println!("rm: {model} (Phase 3 — not yet implemented)");
        }
        Cmd::Show { model, profile } => {
            println!("show: {model} (profile={profile}) (Phase 3 — not yet implemented)");
        }
        Cmd::Run { model, prompt } => {
            let prompt = prompt.join(" ");
            println!("run: {model} -> {prompt:?} (Phase 2 — not yet implemented)");
        }
        Cmd::Ps => {
            println!("ps: (Phase 4 — not yet implemented)");
        }
        Cmd::Serve { bind } => {
            let state = hexrun_server::ServerState { engine: None };
            hexrun_server::serve(bind, state).await?;
        }
    }
    Ok(())
}
