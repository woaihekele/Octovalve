use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use futures_util::StreamExt;
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio::sync::{mpsc, RwLock};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::control::{ControlRequest, ControlResponse, ServiceEvent};
use crate::events::ConsoleEvent;
use crate::state::{ConsoleState, ControlCommand, TargetStatus};

use super::control::send_request;
use super::status::emit_target_update;

pub(crate) struct SessionTracker {
    pub(crate) started_at: Instant,
    pub(crate) snapshot_received: bool,
}

impl SessionTracker {
    pub(crate) fn new() -> Self {
        Self {
            started_at: Instant::now(),
            snapshot_received: false,
        }
    }
}

pub(crate) async fn session_loop(
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    name: &str,
    state: &Arc<RwLock<ConsoleState>>,
    cmd_rx: &mut mpsc::Receiver<ControlCommand>,
    shutdown: &CancellationToken,
    event_tx: &broadcast::Sender<ConsoleEvent>,
    tracker: &mut SessionTracker,
) -> anyhow::Result<()> {
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            Some(command) = cmd_rx.recv() => {
                let request = match command {
                    ControlCommand::Approve(id) => ControlRequest::Approve { id },
                    ControlCommand::Deny(id) => ControlRequest::Deny { id },
                    ControlCommand::Cancel(id) => ControlRequest::Cancel { id },
                };
                if let Err(err) = send_request(framed, request).await {
                    return Err(err);
                }
            }
            frame = framed.next() => {
                let frame = frame.context("control stream closed")?;
                let bytes = frame.context("read control frame")?;
                let response: ControlResponse = serde_json::from_slice(&bytes)?;
                handle_response(name, state, response, event_tx, tracker).await;
            }
        }
    }
}

async fn handle_response(
    name: &str,
    state: &Arc<RwLock<ConsoleState>>,
    response: ControlResponse,
    event_tx: &broadcast::Sender<ConsoleEvent>,
    tracker: &mut SessionTracker,
) {
    match response {
        ControlResponse::Snapshot { snapshot } => {
            let queue_len = snapshot.queue.len();
            let history_len = snapshot.history.len();
            let last_id = snapshot
                .last_result
                .as_ref()
                .map(|result| result.id.as_str());
            let latency_ms = tracker.started_at.elapsed().as_millis();
            if !tracker.snapshot_received {
                tracker.snapshot_received = true;
                info!(
                    event = "control.snapshot.received",
                    target = %name,
                    latency_ms = latency_ms,
                    queue_len = queue_len,
                    history_len = history_len,
                    last_result_id = ?last_id,
                    "control snapshot received"
                );
            } else {
                info!(
                    event = "control.snapshot.update",
                    target = %name,
                    queue_len = queue_len,
                    history_len = history_len,
                    last_result_id = ?last_id,
                    "control snapshot updated"
                );
            }
            let mut guard = state.write().await;
            guard.set_status(name, TargetStatus::Ready, None);
            guard.apply_snapshot(name, snapshot);
            drop(guard);
            emit_target_update(name, state, event_tx).await;
        }
        ControlResponse::Event { event } => {
            match &event {
                ServiceEvent::QueueUpdated(queue) => {
                    info!(
                        target = %name,
                        queue_len = queue.len(),
                        "control queue updated"
                    );
                }
                ServiceEvent::RunningUpdated(running) => {
                    info!(
                        target = %name,
                        running_len = running.len(),
                        "control running updated"
                    );
                }
                ServiceEvent::ResultUpdated(result) => {
                    info!(
                        target = %name,
                        result_id = %result.id,
                        "control result updated"
                    );
                }
                ServiceEvent::ConnectionsChanged => {
                    info!(target = %name, "control connections changed");
                }
            }
            let mut guard = state.write().await;
            guard.set_status(name, TargetStatus::Ready, None);
            guard.apply_event(name, event);
            drop(guard);
            emit_target_update(name, state, event_tx).await;
        }
        ControlResponse::Ack { .. } => {}
        ControlResponse::Error { message } => {
            let mut guard = state.write().await;
            guard.set_status(name, TargetStatus::Ready, Some(message));
            drop(guard);
            emit_target_update(name, state, event_tx).await;
        }
    }
}
