mod cli;
mod config;
mod mcp;
mod state;

use clap::Parser;
use cli::Args;
use mcp::ProxyHandler;
use rmcp::model::{
    Implementation, InitializeResult, ProtocolVersion, ServerCapabilities, ToolsCapability,
};
use rmcp::service::ServiceExt;
use rmcp::transport::stdio;
use state::build_proxy_state;
use std::io;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::prelude::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let args = Args::parse();
    let (state, defaults) = build_proxy_state(&args)?;
    let state = Arc::new(RwLock::new(state));
    let shutdown = CancellationToken::new();

    let server_details = InitializeResult {
        server_info: Implementation {
            name: "octovalve_proxy".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            title: Some("Octovalve Proxy".to_string()),
            icons: None,
            website_url: None,
        },
        capabilities: ServerCapabilities {
            tools: Some(ToolsCapability { list_changed: None }),
            ..Default::default()
        },
        instructions: Some(
            "Use run_command to execute commands on a target after approval. target is required. Use list_targets to see available targets."
                .to_string(),
        ),
        protocol_version: ProtocolVersion::V_2025_06_18,
    };

    let handler = ProxyHandler::new(Arc::clone(&state), args.client_id, defaults, server_details);
    let server = handler
        .serve_with_ct(stdio(), shutdown.clone())
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let result = server.waiting().await;
    shutdown.cancel();
    result.map_err(|err| anyhow::anyhow!(err.to_string()))?;
    Ok(())
}

fn init_tracing() {
    let layer = tracing_subscriber::fmt::layer()
        .with_writer(io::stderr)
        .with_target(false);
    tracing_subscriber::registry().with(layer).init();
}
