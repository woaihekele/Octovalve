mod cli;
mod config;
mod state;

use crate::cli::Args;
use crate::config::load_console_config;
use crate::state::{build_console_state, TargetInfo};
use anyhow::Context;
use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use clap::Parser;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;

#[derive(Clone)]
struct AppState {
    state: Arc<RwLock<crate::state::ConsoleState>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    init_tracing(args.log_to_stderr)?;

    let config = load_console_config(&args.config)
        .with_context(|| format!("failed to load config {}", args.config.display()))?;
    let state = build_console_state(config)?;

    let app_state = AppState {
        state: Arc::new(RwLock::new(state)),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/targets", get(list_targets))
        .with_state(app_state);

    let listener = TcpListener::bind(&args.listen_addr)
        .await
        .with_context(|| format!("failed to bind {}", args.listen_addr))?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

async fn list_targets(State(state): State<AppState>) -> Json<Vec<TargetInfo>> {
    let state = state.state.read().await;
    Json(state.list_targets())
}

fn init_tracing(log_to_stderr: bool) -> anyhow::Result<()> {
    let builder = tracing_subscriber::fmt().with_env_filter(
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
    );
    if log_to_stderr {
        builder.with_writer(std::io::stderr).init();
    } else {
        builder.init();
    }
    Ok(())
}
