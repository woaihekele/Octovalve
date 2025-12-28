use crate::bootstrap::{
    bootstrap_remote_broker, stop_remote_broker, BootstrapConfig, UnsupportedRemotePlatform,
};
use crate::control::{ControlRequest, ControlResponse, ServiceEvent};
use crate::events::ConsoleEvent;
use crate::state::{ConsoleState, ControlCommand, TargetSpec, TargetStatus};
use crate::tunnel::TargetRuntime;
use anyhow::Context;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use tokio::sync::{mpsc, RwLock};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use tunnel_manager::TunnelManager;
use tunnel_protocol::{ForwardPurpose, ForwardSpec};

const RECONNECT_DELAY: Duration = Duration::from_secs(5);

pub(crate) async fn spawn_target_workers(
    state: Arc<RwLock<ConsoleState>>,
    bootstrap: BootstrapConfig,
    shutdown: CancellationToken,
    event_tx: broadcast::Sender<ConsoleEvent>,
    tunnel_manager: Option<Arc<TunnelManager>>,
    tunnel_client_id: String,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut handles = Vec::new();
    let targets = {
        let state = state.read().await;
        state.target_specs()
    };
    for spec in targets {
        let (tx, rx) = mpsc::channel(64);
        {
            let mut state = state.write().await;
            state.register_command_sender(spec.name.clone(), tx.clone());
        }
        info!(target = %spec.name, "console worker spawned");
        let state = Arc::clone(&state);
        let shutdown = shutdown.clone();
        let bootstrap = bootstrap.clone();
        let event_tx = event_tx.clone();
        let tunnel_manager = tunnel_manager.clone();
        let tunnel_client_id = tunnel_client_id.clone();
        let handle = tokio::spawn(async move {
            run_target_worker(
                spec,
                state,
                rx,
                bootstrap,
                shutdown,
                event_tx,
                tunnel_manager,
                tunnel_client_id,
            )
            .await;
        });
        handles.push(handle);
    }
    handles
}

async fn run_target_worker(
    spec: TargetSpec,
    state: Arc<RwLock<ConsoleState>>,
    mut cmd_rx: mpsc::Receiver<ControlCommand>,
    bootstrap: BootstrapConfig,
    shutdown: CancellationToken,
    event_tx: broadcast::Sender<ConsoleEvent>,
    tunnel_manager: Option<Arc<TunnelManager>>,
    tunnel_client_id: String,
) {
    let runtime = TargetRuntime::from_spec(spec);
    let forward_spec = control_forward_spec(&runtime);
    let mut bootstrap_needed = true;
    let mut connect_failures = 0;
    let shutdown_requested = loop {
        if shutdown.is_cancelled() {
            break true;
        }

        if runtime.ssh.is_some() {
            if let Some(forward) = forward_spec.as_ref() {
                let Some(manager) = tunnel_manager.as_ref() else {
                    set_status_and_notify(
                        &runtime.name,
                        TargetStatus::Down,
                        Some("tunnel manager not available".to_string()),
                        &state,
                        &event_tx,
                    )
                    .await;
                    if wait_reconnect_or_shutdown(&shutdown).await {
                        break true;
                    }
                    continue;
                };
                if let Err(err) = manager.ensure_forward(&tunnel_client_id, forward).await {
                    set_status_and_notify(
                        &runtime.name,
                        TargetStatus::Down,
                        Some(err.to_string()),
                        &state,
                        &event_tx,
                    )
                    .await;
                    warn!(target = %runtime.name, error = %err, "failed to ensure tunnel");
                    if wait_reconnect_or_shutdown(&shutdown).await {
                        break true;
                    }
                    continue;
                }
            } else {
                set_status_and_notify(
                    &runtime.name,
                    TargetStatus::Down,
                    Some("missing control local bind/port".to_string()),
                    &state,
                    &event_tx,
                )
                .await;
                if wait_reconnect_or_shutdown(&shutdown).await {
                    break true;
                }
                continue;
            }
        }

        if bootstrap_needed {
            info!(target = %runtime.name, "bootstrapping remote broker");
            if let Err(err) = bootstrap_remote_broker(&runtime, &bootstrap).await {
                set_status_and_notify(
                    &runtime.name,
                    TargetStatus::Down,
                    Some(err.to_string()),
                    &state,
                    &event_tx,
                )
                .await;
                warn!(target = %runtime.name, error = %err, "failed to bootstrap remote broker");
                if err.downcast_ref::<UnsupportedRemotePlatform>().is_some() {
                    info!(
                        target = %runtime.name,
                        "unsupported remote platform, stopping worker"
                    );
                    if let Some(forward) = forward_spec.as_ref() {
                        if let Some(manager) = tunnel_manager.as_ref() {
                            let _ = manager.release_forward(&tunnel_client_id, forward).await;
                        }
                    }
                    break false;
                }
                if wait_reconnect_or_shutdown(&shutdown).await {
                    break true;
                }
                continue;
            }
            bootstrap_needed = false;
            connect_failures = 0;
        }

        let addr = runtime.connect_addr();
        info!(target = %runtime.name, addr = %addr, "connecting control channel");
        match connect_control(&addr).await {
            Ok(mut framed) => {
                connect_failures = 0;
                set_status_and_notify(&runtime.name, TargetStatus::Ready, None, &state, &event_tx)
                    .await;
                if let Err(err) = send_request(&mut framed, ControlRequest::Subscribe).await {
                    set_status_and_notify(
                        &runtime.name,
                        TargetStatus::Down,
                        Some(err.to_string()),
                        &state,
                        &event_tx,
                    )
                    .await;
                    warn!(target = %runtime.name, error = %err, "failed to subscribe");
                    if wait_reconnect_or_shutdown(&shutdown).await {
                        break true;
                    }
                    continue;
                }
                if let Err(err) = send_request(&mut framed, ControlRequest::Snapshot).await {
                    set_status_and_notify(
                        &runtime.name,
                        TargetStatus::Down,
                        Some(err.to_string()),
                        &state,
                        &event_tx,
                    )
                    .await;
                    warn!(target = %runtime.name, error = %err, "failed to request snapshot");
                    if wait_reconnect_or_shutdown(&shutdown).await {
                        break true;
                    }
                    continue;
                }

                info!(target = %runtime.name, "control session started");
                if let Err(err) = session_loop(
                    &mut framed,
                    &runtime.name,
                    &state,
                    &mut cmd_rx,
                    &shutdown,
                    &event_tx,
                )
                .await
                {
                    set_status_and_notify(
                        &runtime.name,
                        TargetStatus::Down,
                        Some(err.to_string()),
                        &state,
                        &event_tx,
                    )
                    .await;
                    warn!(target = %runtime.name, error = %err, "control session ended");
                }
            }
            Err(err) => {
                set_status_and_notify(
                    &runtime.name,
                    TargetStatus::Down,
                    Some(err.to_string()),
                    &state,
                    &event_tx,
                )
                .await;
                warn!(target = %runtime.name, error = %err, "failed to connect control channel");
                connect_failures += 1;
                if connect_failures >= 3 {
                    bootstrap_needed = true;
                    connect_failures = 0;
                }
            }
        }

        if wait_reconnect_or_shutdown(&shutdown).await {
            break true;
        }
    };
    if shutdown_requested {
            info!(target = %runtime.name, "shutdown requested, stopping remote broker");
            if let Err(err) = stop_remote_broker(&runtime, &bootstrap).await {
                warn!(target = %runtime.name, error = %err, "failed to stop remote broker");
            }
            if let Some(forward) = forward_spec.as_ref() {
                if let Some(manager) = tunnel_manager.as_ref() {
                    let _ = manager.release_forward(&tunnel_client_id, forward).await;
                }
            }
            info!(target = %runtime.name, "worker stopped");
        }
}

async fn wait_reconnect_or_shutdown(shutdown: &CancellationToken) -> bool {
    tokio::select! {
        _ = shutdown.cancelled() => true,
        _ = tokio::time::sleep(RECONNECT_DELAY) => false,
    }
}

fn control_forward_spec(runtime: &TargetRuntime) -> Option<ForwardSpec> {
    if runtime.ssh.is_none() {
        return None;
    }
    let bind = runtime.control_local_bind.clone()?;
    let port = runtime.control_local_port?;
    Some(ForwardSpec {
        target: runtime.name.clone(),
        purpose: ForwardPurpose::Control,
        local_bind: bind,
        local_port: port,
        remote_addr: runtime.control_remote_addr.clone(),
    })
}

async fn session_loop(
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    name: &str,
    state: &Arc<RwLock<ConsoleState>>,
    cmd_rx: &mut mpsc::Receiver<ControlCommand>,
    shutdown: &CancellationToken,
    event_tx: &broadcast::Sender<ConsoleEvent>,
) -> anyhow::Result<()> {
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            Some(command) = cmd_rx.recv() => {
                let request = match command {
                    ControlCommand::Approve(id) => ControlRequest::Approve { id },
                    ControlCommand::Deny(id) => ControlRequest::Deny { id },
                };
                if let Err(err) = send_request(framed, request).await {
                    return Err(err);
                }
            }
            frame = framed.next() => {
                let frame = frame.context("control stream closed")?;
                let bytes = frame.context("read control frame")?;
                let response: ControlResponse = serde_json::from_slice(&bytes)?;
                handle_response(name, state, response, event_tx).await;
            }
        }
    }
}

async fn handle_response(
    name: &str,
    state: &Arc<RwLock<ConsoleState>>,
    response: ControlResponse,
    event_tx: &broadcast::Sender<ConsoleEvent>,
) {
    match response {
        ControlResponse::Snapshot { snapshot } => {
            let queue_len = snapshot.queue.len();
            let history_len = snapshot.history.len();
            let last_id = snapshot.last_result.as_ref().map(|result| result.id.as_str());
            info!(
                target = %name,
                queue_len = queue_len,
                history_len = history_len,
                last_result_id = ?last_id,
                "control snapshot received"
            );
            let mut guard = state.write().await;
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

async fn connect_control(addr: &str) -> anyhow::Result<Framed<TcpStream, LengthDelimitedCodec>> {
    let stream = TcpStream::connect(addr)
        .await
        .with_context(|| format!("failed to connect control addr {addr}"))?;
    Ok(Framed::new(stream, LengthDelimitedCodec::new()))
}

async fn send_request(
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    request: ControlRequest,
) -> anyhow::Result<()> {
    let payload = serde_json::to_vec(&request)?;
    framed.send(Bytes::from(payload)).await?;
    Ok(())
}

impl TargetRuntime {
    fn from_spec(spec: TargetSpec) -> Self {
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
}

async fn set_status_and_notify(
    name: &str,
    status: TargetStatus,
    error: Option<String>,
    state: &Arc<RwLock<ConsoleState>>,
    event_tx: &broadcast::Sender<ConsoleEvent>,
) {
    {
        let mut state = state.write().await;
        state.set_status(name, status, error);
    }
    emit_target_update(name, state, event_tx).await;
}

async fn emit_target_update(
    name: &str,
    state: &Arc<RwLock<ConsoleState>>,
    event_tx: &broadcast::Sender<ConsoleEvent>,
) {
    let target = {
        let state = state.read().await;
        state.target_info(name)
    };
    if let Some(target) = target {
        let _ = event_tx.send(ConsoleEvent::TargetUpdated { target });
    }
}
