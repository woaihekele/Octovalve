mod bootstrap;
mod cli;
mod config;
mod control;
mod events;
mod runtime;
mod state;
mod terminal;
mod tunnel;

use crate::bootstrap::{bootstrap_remote_broker, stop_remote_broker, BootstrapConfig};
use crate::cli::Args;
use crate::config::load_console_config;
use crate::control::ServiceSnapshot;
use crate::events::ConsoleEvent;
use crate::runtime::spawn_target_workers;
use crate::state::{build_console_state, parse_ssh_host, ConsoleState, ControlCommand, TargetInfo};
use crate::terminal::terminal_ws_handler;
use crate::tunnel::TargetRuntime;
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
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use tunnel_manager::{TunnelManager, TunnelTargetSpec};
use tunnel_protocol::{ForwardPurpose, ForwardSpec};

#[derive(Clone)]
struct AppState {
    state: Arc<RwLock<crate::state::ConsoleState>>,
    event_tx: broadcast::Sender<ConsoleEvent>,
    bootstrap: BootstrapConfig,
}

const CONSOLE_TUNNEL_CLIENT_ID: &str = "console";

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
    let state = build_console_state(config)?;
    let tunnel_targets = build_tunnel_targets(&state);

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
        bootstrap: bootstrap.clone(),
    };
    spawn_target_ip_resolution(Arc::clone(&shared_state), event_tx.clone());

    let app = Router::new()
        .route("/health", get(health))
        .route("/targets", get(list_targets))
        .route("/targets/:name/snapshot", get(get_snapshot))
        .route("/targets/reload-brokers", post(reload_remote_brokers))
        .route("/targets/:name/approve", post(approve_command))
        .route("/targets/:name/deny", post(deny_command))
        .route("/targets/:name/terminal", get(terminal_ws_handler))
        .route("/ws", get(ws_handler))
        .with_state(app_state)
        .layer(middleware::from_fn(log_http_request));

    let tunnel_manager = if tunnel_targets.is_empty() {
        None
    } else {
        let control_dir = expand_tilde(&args.tunnel_control_dir);
        Some(Arc::new(TunnelManager::new(tunnel_targets, control_dir)?))
    };
    let tunnel_manager_handle = tunnel_manager.clone();
    let worker_handles = spawn_target_workers(
        Arc::clone(&shared_state),
        bootstrap,
        shutdown.clone(),
        event_tx,
        tunnel_manager,
        CONSOLE_TUNNEL_CLIENT_ID.to_string(),
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
    if let Some(manager) = tunnel_manager_handle {
        manager.shutdown().await;
    }
    info!("console workers stopped");
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

async fn reload_remote_brokers(State(state): State<AppState>) -> Result<Json<ActionResponse>, StatusCode> {
    let targets = {
        let state = state.state.read().await;
        state.target_specs()
    };
    if targets.is_empty() {
        return Ok(Json(ActionResponse {
            message: "no targets to reload".to_string(),
        }));
    }
    let bootstrap = state.bootstrap.clone();
    let mut failures = Vec::new();
    for spec in targets {
        let runtime = runtime_from_spec(spec);
        if let Err(err) = stop_remote_broker(&runtime, &bootstrap).await {
            failures.push(format!("{} stop failed: {err}", runtime.name));
        }
        if let Err(err) = bootstrap_remote_broker(&runtime, &bootstrap).await {
            failures.push(format!("{} start failed: {err}", runtime.name));
        }
    }
    if failures.is_empty() {
        Ok(Json(ActionResponse {
            message: "remote brokers reloaded".to_string(),
        }))
    } else {
        warn!(event = "remote_broker.reload_failed", errors = ?failures, "remote broker reload failed");
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}

fn runtime_from_spec(spec: crate::state::TargetSpec) -> TargetRuntime {
    TargetRuntime {
        name: spec.name,
        ssh: spec.ssh,
        ssh_args: spec.ssh_args,
        ssh_password: spec.ssh_password,
        control_remote_addr: spec.control_remote_addr,
        control_local_bind: spec.control_local_bind,
        control_local_port: spec.control_local_port,
        control_local_addr: spec.control_local_addr,
    }
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

fn build_tunnel_targets(state: &ConsoleState) -> Vec<TunnelTargetSpec> {
    state
        .target_specs()
        .into_iter()
        .filter_map(|target| {
            let ssh = target.ssh.clone()?;
            let Some(local_bind) = target.control_local_bind.clone() else {
                warn!(target = %target.name, "missing control_local_bind; skipping tunnel target");
                return None;
            };
            let Some(local_port) = target.control_local_port else {
                warn!(target = %target.name, "missing control_local_port; skipping tunnel target");
                return None;
            };
            let allowed_forwards = vec![ForwardSpec {
                target: target.name.clone(),
                purpose: ForwardPurpose::Control,
                local_bind,
                local_port,
                remote_addr: target.control_remote_addr.clone(),
            }];
            Some(TunnelTargetSpec {
                name: target.name,
                ssh,
                ssh_args: target.ssh_args,
                ssh_password: target.ssh_password,
                allowed_forwards,
            })
        })
        .collect()
}

fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home);
        }
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn spawn_target_ip_resolution(
    state: Arc<RwLock<ConsoleState>>,
    event_tx: broadcast::Sender<ConsoleEvent>,
) {
    tokio::spawn(async move {
        let targets = {
            let guard = state.read().await;
            guard.target_specs()
        };
        for target in targets {
            if target.ip.is_some() {
                continue;
            }
            let mut candidates = Vec::new();
            if let Some(hostname) = target.hostname.as_deref().map(str::trim) {
                if !hostname.is_empty() {
                    candidates.push(hostname.to_string());
                }
            }
            if let Some(ssh) = target.ssh.as_deref().and_then(parse_ssh_host) {
                let ssh = ssh.trim();
                if !ssh.is_empty() && !candidates.iter().any(|host| host == ssh) {
                    candidates.push(ssh.to_string());
                }
            }
            let mut resolved = None;
            for host in candidates {
                if let Some(ip) = resolve_host_ip(&host).await {
                    resolved = Some(ip);
                    break;
                }
            }
            let Some(ip) = resolved else {
                continue;
            };
            let updated = {
                let mut guard = state.write().await;
                guard.update_target_ip(&target.name, ip)
            };
            if let Some(target) = updated {
                let _ = event_tx.send(ConsoleEvent::TargetUpdated { target });
            }
        }
    });
}

async fn resolve_host_ip(host: &str) -> Option<String> {
    let trimmed = host.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.parse::<IpAddr>().is_ok() {
        return Some(trimmed.to_string());
    }
    let addrs = tokio::net::lookup_host((trimmed, 0)).await.ok()?;
    let mut fallback = None;
    for addr in addrs {
        let ip = addr.ip();
        if matches!(ip, IpAddr::V4(_)) {
            return Some(ip.to_string());
        }
        if fallback.is_none() {
            fallback = Some(ip.to_string());
        }
    }
    fallback
}
