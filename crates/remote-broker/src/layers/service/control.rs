use crate::activity::ActivityTracker;
use crate::layers::service::events::{ServiceCommand, ServiceEvent};
use anyhow::Context;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use protocol::control::{ControlRequest, ControlResponse};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

pub(crate) async fn spawn_control_server(
    addr: String,
    cmd_tx: mpsc::Sender<ServiceCommand>,
    event_tx: broadcast::Sender<ServiceEvent>,
    activity: Arc<ActivityTracker>,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("failed to bind control addr {addr}"))?;
    tracing::info!(event = "control.listener.bound", addr = %addr);
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let peer = stream.peer_addr().ok();
                    let cmd_tx = cmd_tx.clone();
                    let event_rx = event_tx.subscribe();
                    let activity = Arc::clone(&activity);
                    tokio::spawn(async move {
                        let _guard = activity.track_control();
                        if let Some(peer) = peer {
                            tracing::info!(event = "control.conn.accepted", peer = %peer);
                        }
                        if let Err(err) =
                            handle_control_connection(stream, cmd_tx, event_rx, peer).await
                        {
                            if let Some(peer) = peer {
                                tracing::warn!(
                                    event = "control.conn.error",
                                    peer = %peer,
                                    error = %err,
                                    "control connection failed"
                                );
                            } else {
                                tracing::warn!(
                                    event = "control.conn.error",
                                    error = %err,
                                    "control connection failed"
                                );
                            }
                        }
                    });
                }
                Err(err) => {
                    tracing::warn!(
                        event = "control.listener.accept_failed",
                        error = %err,
                        "control listener accept failed"
                    );
                }
            }
        }
    });
    Ok(())
}

async fn handle_control_connection(
    stream: TcpStream,
    cmd_tx: mpsc::Sender<ServiceCommand>,
    mut event_rx: broadcast::Receiver<ServiceEvent>,
    peer: Option<SocketAddr>,
) -> anyhow::Result<()> {
    if let Some(peer) = peer {
        tracing::info!(event = "control.conn.start", peer = %peer);
    }
    let codec = LengthDelimitedCodec::builder()
        .max_frame_length(protocol::framing::MAX_FRAME_LENGTH)
        .new_codec();
    let mut framed = Framed::new(stream, codec);
    let mut subscribed = false;
    loop {
        tokio::select! {
            Some(frame) = framed.next() => {
                let bytes = frame.context("read frame")?;
                let request: ControlRequest = match serde_json::from_slice(&bytes) {
                    Ok(request) => request,
                    Err(err) => {
                        if let Some(peer) = peer {
                            tracing::warn!(
                                event = "control.request.invalid",
                                peer = %peer,
                                error = %err,
                                "invalid control request"
                            );
                        } else {
                            tracing::warn!(
                                event = "control.request.invalid",
                                error = %err,
                                "invalid control request"
                            );
                        }
                        let response = ControlResponse::Error {
                            message: format!("invalid request: {err}"),
                        };
                        send_response(&mut framed, &response).await?;
                        continue;
                    }
                };
                match request {
                    ControlRequest::Snapshot => {
                        if let Some(peer) = peer {
                            tracing::info!(event = "control.snapshot_request", peer = %peer);
                        }
                        let (tx, rx) = oneshot::channel();
                        if cmd_tx.send(ServiceCommand::Snapshot(tx)).await.is_err() {
                            let response = ControlResponse::Error {
                                message: "service unavailable".to_string(),
                            };
                            send_response(&mut framed, &response).await?;
                            continue;
                        }
                        match rx.await {
                            Ok(snapshot) => {
                                let response = ControlResponse::Snapshot { snapshot };
                                send_response(&mut framed, &response).await?;
                            }
                            Err(_) => {
                                let response = ControlResponse::Error {
                                    message: "snapshot failed".to_string(),
                                };
                                send_response(&mut framed, &response).await?;
                            }
                        }
                    }
                    ControlRequest::Approve { id } => {
                        if let Some(peer) = peer {
                            tracing::info!(event = "control.approve_request", peer = %peer, id = %id);
                        }
                        let _ = cmd_tx.send(ServiceCommand::Approve(id)).await;
                        let response = ControlResponse::Ack {
                            message: "approve queued".to_string(),
                        };
                        send_response(&mut framed, &response).await?;
                    }
                    ControlRequest::Deny { id } => {
                        if let Some(peer) = peer {
                            tracing::info!(event = "control.deny_request", peer = %peer, id = %id);
                        }
                        let _ = cmd_tx.send(ServiceCommand::Deny(id)).await;
                        let response = ControlResponse::Ack {
                            message: "deny queued".to_string(),
                        };
                        send_response(&mut framed, &response).await?;
                    }
                    ControlRequest::Cancel { id } => {
                        if let Some(peer) = peer {
                            tracing::info!(event = "control.cancel_request", peer = %peer, id = %id);
                        }
                        let _ = cmd_tx.send(ServiceCommand::Cancel(id)).await;
                        let response = ControlResponse::Ack {
                            message: "cancel queued".to_string(),
                        };
                        send_response(&mut framed, &response).await?;
                    }
                    ControlRequest::Subscribe => {
                        subscribed = true;
                        if let Some(peer) = peer {
                            tracing::info!(event = "control.subscribe", peer = %peer);
                        }
                        let response = ControlResponse::Ack {
                            message: "subscribed".to_string(),
                        };
                        send_response(&mut framed, &response).await?;
                    }
                }
            }
            Ok(event) = event_rx.recv(), if subscribed => {
                let response = ControlResponse::Event { event };
                send_response(&mut framed, &response).await?;
            }
            else => break,
        }
    }
    if let Some(peer) = peer {
        tracing::info!(event = "control.conn.closed", peer = %peer);
    }
    Ok(())
}

async fn send_response(
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    response: &ControlResponse,
) -> anyhow::Result<()> {
    let payload = serde_json::to_vec(response)?;
    framed.send(Bytes::from(payload)).await?;
    Ok(())
}
