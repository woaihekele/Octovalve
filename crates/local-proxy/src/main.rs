mod cli;
mod config;
mod mcp;
mod state;
mod tunnel;
mod tunnel_client;

use clap::Parser;
use cli::Args;
use mcp::ProxyHandler;
use rust_mcp_sdk::mcp_server::server_runtime;
use rust_mcp_sdk::schema::{
    Implementation, InitializeResult, ServerCapabilities, ServerCapabilitiesTools,
    LATEST_PROTOCOL_VERSION,
};
use rust_mcp_sdk::{McpServer, StdioTransport, TransportOptions};
use state::build_proxy_state;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::prelude::*;
use tunnel::{shutdown_tunnels, spawn_shutdown_handler};
use tunnel_client::TunnelClient;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const TUNNEL_DAEMON_BOOT_RETRIES: usize = 10;
const TUNNEL_DAEMON_BOOT_DELAY: Duration = Duration::from_millis(200);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let args = Args::parse();
    let (state, defaults) = build_proxy_state(&args)?;
    let state = Arc::new(RwLock::new(state));
    let shutdown = CancellationToken::new();
    let tunnel_client = TunnelClient::new(args.tunnel_daemon_addr.clone(), args.client_id.clone());
    ensure_tunnel_daemon(&tunnel_client, &args.tunnel_daemon_addr, &args.config).await?;
    spawn_heartbeat_task(tunnel_client.clone(), shutdown.clone());
    {
        let mut guard = state.write().await;
        guard.set_tunnel_client(Some(tunnel_client.clone()));
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
    let handler = ProxyHandler::new(Arc::clone(&state), args.client_id, defaults, tunnel_client);
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
    config: &Path,
) -> anyhow::Result<()> {
    if client.list_forwards().await.is_ok() {
        return Ok(());
    }
    spawn_tunnel_daemon(addr, config)?;
    for _ in 0..TUNNEL_DAEMON_BOOT_RETRIES {
        if client.list_forwards().await.is_ok() {
            return Ok(());
        }
        sleep(TUNNEL_DAEMON_BOOT_DELAY).await;
    }
    anyhow::bail!("tunnel-daemon not available at {addr}");
}

fn spawn_tunnel_daemon(addr: &str, config: &Path) -> anyhow::Result<()> {
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
