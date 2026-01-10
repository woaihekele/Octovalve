use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use anyhow::Context;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use protocol::{CommandRequest, CommandResponse};

use super::events::{PendingRequest, ServerEvent};
use super::policy::{deny_message, request_summary, Whitelist};
use super::service::TargetServiceHandle;

pub(super) async fn spawn_command_server(
    listen_addr: SocketAddr,
    services: HashMap<String, TargetServiceHandle>,
    whitelist: Arc<Whitelist>,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(listen_addr).await.map_err(|err| {
        anyhow::anyhow!("failed to bind command listener {}: {}", listen_addr, err)
    })?;
    let services = Arc::new(services);
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let services = Arc::clone(&services);
                    let whitelist = Arc::clone(&whitelist);
                    tokio::spawn(async move {
                        if let Err(err) = handle_connection(stream, addr, services, whitelist).await
                        {
                            tracing::error!(
                                event = "command.conn.error",
                                peer = %addr,
                                error = %err,
                                "command connection failed"
                            );
                        }
                    });
                }
                Err(err) => {
                    tracing::error!(
                        event = "command.listener.accept_failed",
                        error = %err,
                        "command listener accept failed"
                    );
                }
            }
        }
    });
    Ok(())
}

async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    services: Arc<HashMap<String, TargetServiceHandle>>,
    whitelist: Arc<Whitelist>,
) -> anyhow::Result<()> {
    tracing::info!(event = "command.conn.open", peer = %addr);
    let codec = LengthDelimitedCodec::builder()
        .max_frame_length(protocol::framing::MAX_FRAME_LENGTH)
        .new_codec();
    let mut framed = Framed::new(stream, codec);
    while let Some(frame) = framed.next().await {
        let bytes = frame.context("frame read")?;
        let request: CommandRequest = match serde_json::from_slice(&bytes) {
            Ok(request) => request,
            Err(err) => {
                tracing::warn!(
                    event = "command.request.invalid",
                    peer = %addr,
                    error = %err,
                    "invalid request payload"
                );
                let response = CommandResponse::error("invalid", "invalid request");
                let payload = serde_json::to_vec(&response)?;
                let _ = framed.send(Bytes::from(payload)).await;
                continue;
            }
        };

        let handle = match services.get(&request.target) {
            Some(handle) => handle.clone(),
            None => {
                let response = CommandResponse::error(
                    request.id.clone(),
                    format!("unknown target {}", request.target),
                );
                let payload = serde_json::to_vec(&response)?;
                let _ = framed.send(Bytes::from(payload)).await;
                continue;
            }
        };

        tracing::info!(
            event = "command.request_received",
            id = %request.id,
            client = %request.client,
            target = %request.target,
            peer = %addr,
            command = %request_summary(&request),
        );

        if let Some(message) = deny_message(&whitelist, &request) {
            tracing::info!(
                event = "command.request_denied_policy",
                id = %request.id,
                client = %request.client,
                peer = %addr,
                reason = %message,
            );
            let response =
                CommandResponse::denied(request.id.clone(), format!("denied by policy: {message}"));
            let payload = serde_json::to_vec(&response)?;
            let _ = framed.send(Bytes::from(payload)).await;
            continue;
        }

        let (respond_to, response_rx) = tokio::sync::oneshot::channel();
        let pending = PendingRequest {
            request,
            peer: addr.to_string(),
            received_at: SystemTime::now(),
            queued_at: Instant::now(),
            respond_to,
        };
        if handle
            .server_tx
            .send(ServerEvent::Request(pending))
            .await
            .is_err()
        {
            break;
        }

        match response_rx.await {
            Ok(response) => {
                let payload = serde_json::to_vec(&response)?;
                framed.send(Bytes::from(payload)).await?;
            }
            Err(_) => break,
        }
    }
    tracing::info!(event = "command.conn.closed", peer = %addr);
    Ok(())
}

impl Clone for TargetServiceHandle {
    fn clone(&self) -> Self {
        Self {
            server_tx: self.server_tx.clone(),
            command_tx: self.command_tx.clone(),
            snapshot: self.snapshot.clone(),
        }
    }
}
