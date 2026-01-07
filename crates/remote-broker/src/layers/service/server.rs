use crate::activity::ActivityTracker;
use crate::layers::execution::executor::execute_request;
use crate::layers::policy::summary::{deny_message, request_summary};
use crate::layers::policy::whitelist::Whitelist;
use crate::layers::service::audit::{
    spawn_write_request_record, spawn_write_request_record_value, RequestRecord,
};
use crate::layers::service::events::{PendingRequest, ServerEvent};
use anyhow::Context;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use protocol::{CommandRequest, CommandResponse};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tokio_util::sync::CancellationToken;

pub(crate) fn spawn_accept_loop(
    listener: TcpListener,
    server_tx: mpsc::Sender<ServerEvent>,
    output_dir: Arc<PathBuf>,
    whitelist: Arc<Whitelist>,
    activity: Arc<ActivityTracker>,
) {
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let accept_tx = server_tx.clone();
                    let output_dir = Arc::clone(&output_dir);
                    let whitelist = Arc::clone(&whitelist);
                    let activity = Arc::clone(&activity);
                    tokio::spawn(async move {
                        let _guard = activity.track_data();
                        if let Err(err) =
                            handle_connection_tui(stream, addr, accept_tx, output_dir, whitelist)
                                .await
                        {
                            tracing::error!(
                                event = "data.conn.error",
                                peer = %addr,
                                error = %err,
                                "connection handler failed"
                            );
                        }
                    });
                }
                Err(err) => {
                    tracing::error!(
                        event = "data.listener.accept_failed",
                        error = %err,
                        "listener accept failed"
                    );
                }
            }
        }
    });
}

pub(crate) async fn run_headless(
    listener: TcpListener,
    whitelist: Arc<Whitelist>,
    limits: Arc<crate::layers::policy::config::LimitsConfig>,
    output_dir: Arc<PathBuf>,
    activity: Arc<ActivityTracker>,
) -> anyhow::Result<()> {
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    let whitelist = Arc::clone(&whitelist);
                    let limits = Arc::clone(&limits);
                    let output_dir = Arc::clone(&output_dir);
                    let activity = Arc::clone(&activity);
                    tokio::spawn(async move {
                        let _guard = activity.track_data();
                        if let Err(err) =
                            handle_connection_auto(stream, addr, whitelist, limits, output_dir)
                                .await
                        {
                            tracing::error!(
                                event = "data.conn.error",
                                peer = %addr,
                                error = %err,
                                "connection handler failed"
                            );
                        }
                    });
                }
                Err(err) => {
                    tracing::error!(
                        event = "data.listener.accept_failed",
                        error = %err,
                        "listener accept failed"
                    );
                }
            }
        }
    });

    tokio::signal::ctrl_c().await?;
    Ok(())
}

async fn handle_connection_tui(
    stream: TcpStream,
    addr: std::net::SocketAddr,
    server_tx: mpsc::Sender<ServerEvent>,
    output_dir: Arc<PathBuf>,
    whitelist: Arc<Whitelist>,
) -> anyhow::Result<()> {
    tracing::info!(event = "data.conn.open", peer = %addr);
    let _ = server_tx.send(ServerEvent::ConnectionOpened).await;
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
                    event = "request.invalid",
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

        tracing::info!(
            event = "request_received",
            id = %request.id,
            client = %request.client,
            target = %request.target,
            peer = %addr,
            command = %request_summary(&request),
        );

        if let Some(message) = deny_message(&whitelist, &request) {
            tracing::info!(
                event = "request_denied_policy",
                id = %request.id,
                client = %request.client,
                peer = %addr,
                reason = %message,
            );
            let received_at = SystemTime::now();
            let record = RequestRecord::from_request(&request, &addr.to_string(), received_at);
            spawn_write_request_record_value(Arc::clone(&output_dir), record);
            let response =
                CommandResponse::denied(request.id.clone(), format!("denied by policy: {message}"));
            crate::layers::execution::output::write_result_record(
                &output_dir,
                &response,
                Duration::from_secs(0),
            )
            .await;
            let payload = serde_json::to_vec(&response)?;
            let _ = framed.send(Bytes::from(payload)).await;
            continue;
        }

        let (respond_to, response_rx) = oneshot::channel();
        let received_at = SystemTime::now();
        let pending = PendingRequest {
            request,
            peer: addr.to_string(),
            received_at,
            queued_at: Instant::now(),
            respond_to,
        };
        spawn_write_request_record(Arc::clone(&output_dir), &pending);
        if server_tx.send(ServerEvent::Request(pending)).await.is_err() {
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
    let _ = server_tx.send(ServerEvent::ConnectionClosed).await;
    tracing::info!(event = "data.conn.closed", peer = %addr);
    Ok(())
}

async fn handle_connection_auto(
    stream: TcpStream,
    addr: std::net::SocketAddr,
    whitelist: Arc<Whitelist>,
    limits: Arc<crate::layers::policy::config::LimitsConfig>,
    output_dir: Arc<PathBuf>,
) -> anyhow::Result<()> {
    tracing::info!(event = "data.conn.open", peer = %addr);
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
                    event = "request.invalid",
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

        tracing::info!(
            event = "request_received",
            id = %request.id,
            client = %request.client,
            target = %request.target,
            peer = %addr,
            command = %request_summary(&request),
        );

        let received_at = SystemTime::now();
        let record = RequestRecord::from_request(&request, &addr.to_string(), received_at);
        spawn_write_request_record_value(Arc::clone(&output_dir), record);

        let response = execute_request(
            &request,
            &whitelist,
            &limits,
            &output_dir,
            CancellationToken::new(),
        )
        .await;
        let payload = serde_json::to_vec(&response)?;
        framed.send(Bytes::from(payload)).await?;
    }
    tracing::info!(event = "data.conn.closed", peer = %addr);
    Ok(())
}
