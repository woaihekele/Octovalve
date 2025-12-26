mod cli;
mod config;
mod ssh;
mod state;

use crate::cli::Args;
use crate::config::load_daemon_config;
use crate::state::DaemonState;
use anyhow::Context;
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio_util::codec::{Framed, LinesCodec};
use tunnel_protocol::{TunnelRequest, TunnelResponse};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    init_tracing(args.log_to_stderr)?;

    let config = load_daemon_config(&args.config)
        .with_context(|| format!("failed to load config {}", args.config.display()))?;
    let control_dir = resolve_control_dir(args.control_dir.as_ref(), config.control_dir.as_ref());
    std::fs::create_dir_all(&control_dir)
        .with_context(|| format!("failed to create {}", control_dir.display()))?;

    let state = Arc::new(RwLock::new(
        DaemonState::build(config, control_dir).context("failed to build daemon state")?,
    ));

    let listener = TcpListener::bind(&args.listen_addr)
        .await
        .with_context(|| format!("failed to bind {}", args.listen_addr))?;
    tracing::info!(addr = %args.listen_addr, "tunnel-daemon listening");

    loop {
        let (stream, peer) = listener.accept().await?;
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(err) = handle_connection(stream, state).await {
                tracing::warn!(peer = %peer, error = %err, "failed to handle connection");
            }
        });
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    state: Arc<RwLock<DaemonState>>,
) -> anyhow::Result<()> {
    let mut framed = Framed::new(stream, LinesCodec::new());
    let Some(line) = framed.next().await else {
        return Ok(());
    };
    let line = line.context("failed to read request line")?;
    let request: TunnelRequest = match serde_json::from_str(&line) {
        Ok(request) => request,
        Err(err) => {
            let payload = serde_json::to_string(&TunnelResponse::Error {
                message: format!("invalid request: {err}"),
            })?;
            framed.send(payload).await?;
            return Ok(());
        }
    };

    let response = match request {
        TunnelRequest::EnsureForward { client_id, forward } => {
            let mut state = state.write().await;
            match state.ensure_forward(&client_id, forward).await {
                Ok((local_addr, reused)) => TunnelResponse::EnsureForward { local_addr, reused },
                Err(err) => TunnelResponse::Error {
                    message: err.to_string(),
                },
            }
        }
        TunnelRequest::ReleaseForward { client_id, forward } => {
            let mut state = state.write().await;
            match state.release_forward(&client_id, forward).await {
                Ok(released) => TunnelResponse::ReleaseForward { released },
                Err(err) => TunnelResponse::Error {
                    message: err.to_string(),
                },
            }
        }
        TunnelRequest::ListForwards => {
            let state = state.read().await;
            TunnelResponse::Forwards {
                items: state.list_forwards(),
            }
        }
    };

    let payload = serde_json::to_string(&response)?;
    framed.send(payload).await?;
    Ok(())
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

fn resolve_control_dir(
    cli_value: Option<&String>,
    config_value: Option<&String>,
) -> PathBuf {
    let raw = cli_value
        .cloned()
        .or_else(|| config_value.cloned())
        .unwrap_or_else(|| "~/.octovalve/tunnel-control".to_string());
    expand_tilde(&raw)
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
