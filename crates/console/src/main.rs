mod cli;
mod config;
mod control;
mod runtime;
mod state;
mod tunnel;

use crate::cli::Args;
use crate::config::load_console_config;
use crate::control::ServiceSnapshot;
use crate::runtime::spawn_target_workers;
use crate::state::{build_console_state, ControlCommand, TargetInfo};
use anyhow::Context;
use axum::extract::Path;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::routing::post;
use axum::{Json, Router};
use clap::Parser;
use serde::Deserialize;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

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

    let shutdown = CancellationToken::new();
    let shared_state = Arc::new(RwLock::new(state));
    let app_state = AppState {
        state: Arc::clone(&shared_state),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/targets", get(list_targets))
        .route("/targets/:name/snapshot", get(get_snapshot))
        .route("/targets/:name/approve", post(approve_command))
        .route("/targets/:name/deny", post(deny_command))
        .with_state(app_state);

    spawn_target_workers(Arc::clone(&shared_state), shutdown.clone());

    let listener = TcpListener::bind(&args.listen_addr)
        .await
        .with_context(|| format!("failed to bind {}", args.listen_addr))?;
    axum::serve(listener, app)
        .with_graceful_shutdown(wait_for_shutdown(shutdown.clone()))
        .await?;
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

async fn list_targets(State(state): State<AppState>) -> Json<Vec<TargetInfo>> {
    let state = state.state.read().await;
    Json(state.list_targets())
}

async fn get_snapshot(
    Path(name): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<ServiceSnapshot>, StatusCode> {
    let state = state.state.read().await;
    match state.snapshot(&name) {
        Some(snapshot) => Ok(Json(snapshot)),
        None => Err(StatusCode::NOT_FOUND),
    }
}

#[derive(Deserialize)]
struct CommandPayload {
    id: String,
}

#[derive(serde::Serialize)]
struct ActionResponse {
    message: String,
}

async fn approve_command(
    Path(name): Path<String>,
    State(state): State<AppState>,
    Json(payload): Json<CommandPayload>,
) -> Result<Json<ActionResponse>, StatusCode> {
    let sender = state.state.read().await.command_sender(&name);
    let Some(sender) = sender else {
        return Err(StatusCode::NOT_FOUND);
    };
    sender
        .send(ControlCommand::Approve(payload.id))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(ActionResponse {
        message: "approve queued".to_string(),
    }))
}

async fn deny_command(
    Path(name): Path<String>,
    State(state): State<AppState>,
    Json(payload): Json<CommandPayload>,
) -> Result<Json<ActionResponse>, StatusCode> {
    let sender = state.state.read().await.command_sender(&name);
    let Some(sender) = sender else {
        return Err(StatusCode::NOT_FOUND);
    };
    sender
        .send(ControlCommand::Deny(payload.id))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(ActionResponse {
        message: "deny queued".to_string(),
    }))
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

async fn wait_for_shutdown(shutdown: CancellationToken) {
    let _ = tokio::signal::ctrl_c().await;
    shutdown.cancel();
}
