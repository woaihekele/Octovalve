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
use state::build_proxy_state;
use std::io;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::prelude::*;
use tunnel::{shutdown_tunnels, spawn_shutdown_handler, spawn_tunnel_manager};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let args = Args::parse();
    let (state, defaults) = build_proxy_state(&args)?;
    let state = Arc::new(RwLock::new(state));
    let shutdown = CancellationToken::new();
    spawn_tunnel_manager(Arc::clone(&state), shutdown.clone());
    spawn_shutdown_handler(Arc::clone(&state), shutdown.clone());

    let server_details = InitializeResult {
        server_info: Implementation {
            name: "conduit_local_proxy".to_string(),
            version: "0.1.0".to_string(),
            title: Some("Remote Command Local Proxy".to_string()),
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
    let handler = ProxyHandler::new(Arc::clone(&state), args.client_id, defaults);
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
