mod cli;
mod config;
mod control;
mod events;
mod local_exec;
mod runtime;
mod state;
mod terminal;

use crate::cli::Args;
use crate::config::load_console_config;
use crate::control::ServiceSnapshot;
use crate::events::ConsoleEvent;
use crate::local_exec::{spawn_local_exec, PolicyConfig};
use crate::state::{build_console_state, ConsoleState, ControlCommand, TargetInfo};
use crate::terminal::terminal_ws_handler;
use anyhow::Context;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::Path;
use axum::extract::State;
use axum::http::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::get;
use axum::routing::post;
use axum::{Json, Router};
use clap::Parser;
use serde::Deserialize;
use std::sync::Arc;
use system_utils::path::expand_tilde;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::info;

#[derive(Clone)]
struct AppState {
    state: Arc<RwLock<crate::state::ConsoleState>>,
    event_tx: broadcast::Sender<ConsoleEvent>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    init_tracing(args.log_to_stderr)?;

    info!(
        listen_addr = %args.listen_addr,
        config = %args.config.display(),
        command_listen_addr = %args.command_listen_addr,
        "console starting"
    );
    let config = load_console_config(&args.config)
        .with_context(|| format!("failed to load config {}", args.config.display()))?;
    let state = build_console_state(config)?;
    let local_audit_dir = expand_tilde(&args.local_audit_dir);
    let shutdown = CancellationToken::new();
    let shared_state = Arc::new(RwLock::new(state));
    let (event_tx, _) = broadcast::channel(512);
    let app_state = AppState {
        state: Arc::clone(&shared_state),
        event_tx: event_tx.clone(),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/targets", get(list_targets))
        .route("/targets/:name/snapshot", get(get_snapshot))
        .route("/targets/:name/approve", post(approve_command))
        .route("/targets/:name/deny", post(deny_command))
        .route("/targets/:name/cancel", post(cancel_command))
        .route("/targets/:name/terminal", get(terminal_ws_handler))
        .route("/ws", get(ws_handler))
        .with_state(app_state)
        .layer(middleware::from_fn(log_http_request));
    let policy = PolicyConfig::load(&args.broker_config)
        .with_context(|| format!("failed to load policy {}", args.broker_config.display()))?;
    let listen_addr = args
        .command_listen_addr
        .parse()
        .with_context(|| format!("invalid command_listen_addr {}", args.command_listen_addr))?;
    spawn_local_exec(
        listen_addr,
        policy,
        local_audit_dir,
        Arc::clone(&shared_state),
        event_tx.clone(),
    )
    .await
    .context("failed to start local exec server")?;

    let listener = TcpListener::bind(&args.listen_addr)
        .await
        .with_context(|| format!("failed to bind {}", args.listen_addr))?;
    info!(addr = %args.listen_addr, "console listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(wait_for_shutdown(shutdown.clone()))
        .await?;
    info!("console shutting down");
    shutdown.cancel();
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

async fn log_http_request(req: Request<Body>, next: Next) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let host = req
        .headers()
        .get("host")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("-")
        .to_string();
    let response = next.run(req).await;
    let status = response.status();
    tracing::info!(
        method = %method,
        uri = %uri,
        host = %host,
        status = %status,
        "http request"
    );
    response
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
        Some(snapshot) => {
            let queue_len = snapshot.queue.len();
            let history_len = snapshot.history.len();
            let last_id = snapshot
                .last_result
                .as_ref()
                .map(|result| result.id.clone());
            tracing::info!(
                target = %name,
                queue_len = queue_len,
                history_len = history_len,
                last_result_id = ?last_id,
                "snapshot served"
            );
            Ok(Json(snapshot))
        }
        None => {
            if let Some(target) = state.target_info(&name) {
                tracing::info!(
                    event = "snapshot.miss",
                    target = %name,
                    status = ?target.status,
                    pending_count = target.pending_count,
                    last_seen = ?target.last_seen,
                    last_error = ?target.last_error,
                    "snapshot not ready"
                );
            } else {
                tracing::warn!(
                    event = "snapshot.miss",
                    target = %name,
                    "snapshot not ready"
                );
            }
            Err(StatusCode::NOT_FOUND)
        }
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

async fn cancel_command(
    Path(name): Path<String>,
    State(state): State<AppState>,
    Json(payload): Json<CommandPayload>,
) -> Result<Json<ActionResponse>, StatusCode> {
    let sender = state.state.read().await.command_sender(&name);
    let Some(sender) = sender else {
        return Err(StatusCode::NOT_FOUND);
    };
    sender
        .send(ControlCommand::Cancel(payload.id))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(ActionResponse {
        message: "cancel queued".to_string(),
    }))
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: AppState) {
    let snapshot = {
        let state = state.state.read().await;
        state.list_targets()
    };
    if send_ws_event(
        &mut socket,
        ConsoleEvent::TargetsSnapshot { targets: snapshot },
    )
    .await
    .is_err()
    {
        return;
    }

    let mut rx = state.event_tx.subscribe();
    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Ok(event) => {
                        if send_ws_event(&mut socket, event).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }
}

async fn send_ws_event(socket: &mut WebSocket, event: ConsoleEvent) -> Result<(), axum::Error> {
    let payload = match serde_json::to_string(&event) {
        Ok(payload) => payload,
        Err(err) => {
            tracing::warn!(error = %err, "failed to serialize websocket event");
            return Ok(());
        }
    };
    socket.send(Message::Text(payload)).await
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
    info!("shutdown signal received");
    shutdown.cancel();
}
