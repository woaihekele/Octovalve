mod cli;
mod config;
mod mcp;
mod state;
mod tunnel;

use clap::Parser;
use cli::Args;
use mcp::ProxyHandler;
use rust_mcp_sdk::mcp_server::server_runtime;
use rust_mcp_sdk::schema::{
    Implementation, InitializeResult, ServerCapabilities, ServerCapabilitiesTools,
    LATEST_PROTOCOL_VERSION,
};
use rust_mcp_sdk::{McpServer, StdioTransport, TransportOptions};
use state::{build_proxy_state, ProxyState};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::prelude::*;
use tunnel::{shutdown_tunnels, spawn_shutdown_handler};
use tunnel_manager::TunnelManager;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let args = Args::parse();
    let (state, defaults) = build_proxy_state(&args)?;
    let tunnel_targets = state.tunnel_targets();
    let state = Arc::new(RwLock::new(state));
    let shutdown = CancellationToken::new();
    let tunnel_manager = if tunnel_targets.is_empty() {
        None
    } else {
        Some(Arc::new(TunnelManager::new(
            tunnel_targets,
            expand_tilde(&args.tunnel_control_dir),
        )?))
    };
    {
        let mut guard = state.write().await;
        guard.set_tunnel_manager(tunnel_manager.clone());
    }
    if let Some(manager) = tunnel_manager.clone() {
        let state = Arc::clone(&state);
        let client_id = args.client_id.clone();
        tokio::spawn(async move {
            if let Err(err) = precheck_tunnels(state, manager, &client_id).await {
                tracing::warn!(
                    event = "tunnel.precheck.task_failed",
                    error = %err,
                    "tunnel precheck task failed"
                );
            }
        });
    }
    spawn_shutdown_handler(Arc::clone(&state), shutdown.clone());

    let server_details = InitializeResult {
        server_info: Implementation {
            name: "octovalve_proxy".to_string(),
            version: "0.1.0".to_string(),
            title: Some("Octovalve Proxy".to_string()),
        },
        capabilities: ServerCapabilities {
            tools: Some(ServerCapabilitiesTools { list_changed: None }),
            ..Default::default()
        },
        meta: None,
        instructions: Some(
            "Use run_command to execute commands on a target after approval. target is required. Use list_targets to see available targets."
                .to_string(),
        ),
        protocol_version: LATEST_PROTOCOL_VERSION.to_string(),
    };

    let transport = StdioTransport::new(TransportOptions::default())
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let handler = ProxyHandler::new(Arc::clone(&state), args.client_id, defaults, tunnel_manager);
    let server = server_runtime::create_server(server_details, transport, handler);
    let result = server.start().await;
    shutdown.cancel();
    shutdown_tunnels(Arc::clone(&state)).await;
    result.map_err(|err| anyhow::anyhow!(err.to_string()))?;
    Ok(())
}

fn init_tracing() {
    let layer = tracing_subscriber::fmt::layer()
        .with_writer(io::stderr)
        .with_target(false);
    tracing_subscriber::registry().with(layer).init();
}

async fn precheck_tunnels(
    state: Arc<RwLock<ProxyState>>,
    manager: Arc<TunnelManager>,
    client_id: &str,
) -> anyhow::Result<()> {
    let target_names = {
        let state = state.read().await;
        state.target_names()
    };
    for name in target_names {
        let forward = {
            let guard = state.read().await;
            match guard.forward_spec(&name) {
                Ok(value) => value,
                Err(err) => {
                    tracing::warn!(
                        event = "tunnel.precheck.forward_spec_failed",
                        target = %name,
                        error = %err,
                        "failed to resolve forward spec"
                    );
                    drop(guard);
                    let mut write_guard = state.write().await;
                    write_guard.note_failure(&name, &err.to_string());
                    continue;
                }
            }
        };
        if let Some(forward) = forward {
            tracing::info!(target = %name, "prechecking ssh tunnel forward");
            match manager.ensure_forward(client_id, &forward).await {
                Ok(_) => {
                    let mut write_guard = state.write().await;
                    write_guard.note_tunnel_ready(&name);
                }
                Err(err) => {
                    tracing::warn!(
                        event = "tunnel.precheck.failed",
                        target = %name,
                        error = %err,
                        "tunnel precheck failed; continuing"
                    );
                    let mut write_guard = state.write().await;
                    write_guard.note_failure(&name, &err.to_string());
                }
            }
        } else {
            tracing::info!(target = %name, "prechecking local target");
            let mut write_guard = state.write().await;
            write_guard.note_tunnel_ready(&name);
        }
    }
    Ok(())
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
