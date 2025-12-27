mod bootstrap;
mod cli;
mod config;
mod control;
mod events;
mod runtime;
mod state;
mod tunnel;
mod tunnel_client;

use crate::bootstrap::BootstrapConfig;
use crate::cli::Args;
use crate::config::load_console_config;
use crate::control::ServiceSnapshot;
use crate::events::ConsoleEvent;
use crate::runtime::spawn_target_workers;
use crate::state::{build_console_state, ControlCommand, TargetInfo};
use crate::tunnel_client::TunnelClient;
use anyhow::Context;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::Path;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::routing::post;
use axum::{Json, Router};
use clap::Parser;
use serde::Deserialize;
use std::path::{Path as FsPath, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio::sync::broadcast;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

#[derive(Clone)]
struct AppState {
    state: Arc<RwLock<crate::state::ConsoleState>>,
    event_tx: broadcast::Sender<ConsoleEvent>,
}

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const TUNNEL_DAEMON_BOOT_RETRIES: usize = 10;
const TUNNEL_DAEMON_BOOT_DELAY: Duration = Duration::from_millis(200);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    init_tracing(args.log_to_stderr)?;

    info!(
        listen_addr = %args.listen_addr,
        config = %args.config.display(),
        broker_bin = %args.broker_bin.display(),
        broker_bin_linux_x86_64 = ?args.broker_bin_linux_x86_64,
        "console starting"
    );
    let config = load_console_config(&args.config)
        .with_context(|| format!("failed to load config {}", args.config.display()))?;
    let needs_tunnel_daemon = config.targets.iter().any(|target| target.ssh.is_some());
    let state = build_console_state(config)?;

    let shutdown = CancellationToken::new();
    let bootstrap = BootstrapConfig {
        local_bin: args.broker_bin.clone(),
        local_bin_linux_x86_64: args.broker_bin_linux_x86_64.clone(),
        local_config: args.broker_config.clone(),
        remote_dir: args.remote_dir.clone(),
        remote_listen_addr: args.remote_listen_addr.clone(),
        remote_control_addr: args.remote_control_addr.clone(),
        remote_audit_dir: args.remote_audit_dir.clone(),
    };
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
        .route("/ws", get(ws_handler))
        .with_state(app_state);

    let tunnel_client = TunnelClient::new(args.tunnel_daemon_addr.clone(), args.tunnel_client_id);
    if needs_tunnel_daemon {
        let daemon_client = tunnel_client.clone();
        let addr = args.tunnel_daemon_addr.clone();
        let config_path = args.config.clone();
        tokio::spawn(async move {
            if let Err(err) = ensure_tunnel_daemon(&daemon_client, &addr, &config_path).await {
                warn!(error = %err, "tunnel-daemon not ready, continuing without tunnels");
            }
        });
        spawn_heartbeat_task(tunnel_client.clone(), shutdown.clone());
    }
    let worker_handles = spawn_target_workers(
        Arc::clone(&shared_state),
        bootstrap,
        shutdown.clone(),
        event_tx,
        tunnel_client,
    )
    .await;

    let listener = TcpListener::bind(&args.listen_addr)
        .await
        .with_context(|| format!("failed to bind {}", args.listen_addr))?;
    info!(addr = %args.listen_addr, "console listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(wait_for_shutdown(shutdown.clone()))
        .await?;
    info!("console shutting down, waiting for workers");
    shutdown.cancel();
    for handle in worker_handles {
        let _ = handle.await;
    }
    info!("console workers stopped");
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

fn spawn_heartbeat_task(client: TunnelClient, shutdown: CancellationToken) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = interval.tick() => {
                    let _ = client.heartbeat().await;
                }
            }
        }
    });
}

async fn ensure_tunnel_daemon(
    client: &TunnelClient,
    addr: &str,
    config: &FsPath,
) -> anyhow::Result<()> {
    if client.list_forwards().await.is_ok() {
        let _ = client.heartbeat().await;
        return Ok(());
    }
    spawn_tunnel_daemon(addr, config)?;
    for _ in 0..TUNNEL_DAEMON_BOOT_RETRIES {
        if client.list_forwards().await.is_ok() {
            let _ = client.heartbeat().await;
            return Ok(());
        }
        sleep(TUNNEL_DAEMON_BOOT_DELAY).await;
    }
    anyhow::bail!("tunnel-daemon not available at {addr}");
}

fn spawn_tunnel_daemon(addr: &str, config: &FsPath) -> anyhow::Result<()> {
    let bin = resolve_tunnel_daemon_bin();
    let mut cmd = Command::new(bin);
    cmd.arg("--config")
        .arg(config)
        .arg("--listen-addr")
        .arg(addr)
        .arg("--log-to-stderr")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    cmd.spawn().map_err(|err| {
        anyhow::anyhow!(
            "failed to spawn tunnel-daemon: {} (set OCTOVALVE_TUNNEL_DAEMON_BIN to override)",
            err
        )
    })?;
    Ok(())
}

fn resolve_tunnel_daemon_bin() -> PathBuf {
    if let Ok(path) = std::env::var("OCTOVALVE_TUNNEL_DAEMON_BIN") {
        return PathBuf::from(path);
    }
    if let Ok(exe) = std::env::current_exe() {
        let candidate = exe.with_file_name("tunnel-daemon");
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from("tunnel-daemon")
}
