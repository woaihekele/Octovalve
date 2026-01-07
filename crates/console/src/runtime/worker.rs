use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::broadcast;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use tunnel_manager::TunnelManager;
use tunnel_protocol::{ForwardPurpose, ForwardSpec};

use crate::bootstrap::{
    bootstrap_remote_broker, stop_remote_broker, BootstrapConfig, UnsupportedRemotePlatform,
};
use crate::control::ControlRequest;
use crate::events::ConsoleEvent;
use crate::state::{ConsoleState, ControlCommand, TargetSpec, TargetStatus};
use crate::tunnel::TargetRuntime;

use super::control::{connect_control, send_request, wait_for_control_ready};
use super::session::{session_loop, SessionTracker};
use super::status::set_status_and_notify;

const RECONNECT_DELAY: Duration = Duration::from_secs(5);
const FAST_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const INITIAL_CONNECT_GRACE: Duration = Duration::from_secs(15);

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
    let connect_grace_started = Instant::now();
    let mut ever_snapshot = false;
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
                    if wait_reconnect_or_shutdown(&shutdown, RECONNECT_DELAY).await {
                        break true;
                    }
                    continue;
                };
                if let Err(err) = manager.ensure_forward(&tunnel_client_id, forward).await {
                    let error = err.to_string();
                    handle_connect_failure(
                        &runtime.name,
                        error,
                        in_connect_grace(ever_snapshot, connect_grace_started),
                        &state,
                        &event_tx,
                    )
                    .await;
                    warn!(target = %runtime.name, error = %err, "failed to ensure tunnel");
                    if wait_reconnect_or_shutdown(&shutdown, RECONNECT_DELAY).await {
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
                if wait_reconnect_or_shutdown(&shutdown, RECONNECT_DELAY).await {
                    break true;
                }
                continue;
            }
        }

        if bootstrap_needed {
            info!(target = %runtime.name, "bootstrapping remote broker");
            if let Err(err) = bootstrap_remote_broker(&runtime, &bootstrap).await {
                let error = err.to_string();
                if err.downcast_ref::<UnsupportedRemotePlatform>().is_some() {
                    set_status_and_notify(
                        &runtime.name,
                        TargetStatus::Down,
                        Some(error),
                        &state,
                        &event_tx,
                    )
                    .await;
                } else {
                    handle_connect_failure(
                        &runtime.name,
                        error,
                        in_connect_grace(ever_snapshot, connect_grace_started),
                        &state,
                        &event_tx,
                    )
                    .await;
                }
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
                if wait_reconnect_or_shutdown(&shutdown, RECONNECT_DELAY).await {
                    break true;
                }
                continue;
            }
            bootstrap_needed = false;
            connect_failures = 0;
        }

        let addr = runtime.connect_addr();
        if in_connect_grace(ever_snapshot, connect_grace_started) {
            set_connecting_status(&runtime.name, &state, &event_tx).await;
        }
        if runtime.ssh.is_some() && !ever_snapshot {
            if let Err(err) = wait_for_control_ready(&runtime.name, &addr, &shutdown).await {
                let error = err.to_string();
                handle_connect_failure(
                    &runtime.name,
                    error,
                    in_connect_grace(ever_snapshot, connect_grace_started),
                    &state,
                    &event_tx,
                )
                .await;
                warn!(target = %runtime.name, error = %err, "control addr not ready");
                let delay = if in_connect_grace(ever_snapshot, connect_grace_started) {
                    FAST_RECONNECT_DELAY
                } else {
                    RECONNECT_DELAY
                };
                if wait_reconnect_or_shutdown(&shutdown, delay).await {
                    break true;
                }
                continue;
            }
        }
        info!(target = %runtime.name, addr = %addr, "connecting control channel");
        match connect_control(&addr).await {
            Ok(mut framed) => {
                connect_failures = 0;
                let mut tracker = SessionTracker::new();
                if let Err(err) = send_request(&mut framed, ControlRequest::Subscribe).await {
                    let error = err.to_string();
                    handle_connect_failure(
                        &runtime.name,
                        error,
                        in_connect_grace(ever_snapshot, connect_grace_started),
                        &state,
                        &event_tx,
                    )
                    .await;
                    warn!(target = %runtime.name, error = %err, "failed to subscribe");
                    if wait_reconnect_or_shutdown(&shutdown, RECONNECT_DELAY).await {
                        break true;
                    }
                    continue;
                }
                if let Err(err) = send_request(&mut framed, ControlRequest::Snapshot).await {
                    let error = err.to_string();
                    handle_connect_failure(
                        &runtime.name,
                        error,
                        in_connect_grace(ever_snapshot, connect_grace_started),
                        &state,
                        &event_tx,
                    )
                    .await;
                    warn!(target = %runtime.name, error = %err, "failed to request snapshot");
                    if wait_reconnect_or_shutdown(&shutdown, RECONNECT_DELAY).await {
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
                    &mut tracker,
                )
                .await
                {
                    let error = err.to_string();
                    let use_grace = !tracker.snapshot_received
                        && in_connect_grace(ever_snapshot, connect_grace_started);
                    handle_connect_failure(&runtime.name, error, use_grace, &state, &event_tx)
                        .await;
                    warn!(target = %runtime.name, error = %err, "control session ended");
                    if !tracker.snapshot_received {
                        warn!(
                            event = "control.session.no_snapshot",
                            target = %runtime.name,
                            session_ms = tracker.started_at.elapsed().as_millis(),
                            "control session ended before snapshot"
                        );
                    }
                }
                if tracker.snapshot_received {
                    ever_snapshot = true;
                }
                let reconnect_delay = if tracker.snapshot_received {
                    RECONNECT_DELAY
                } else {
                    FAST_RECONNECT_DELAY
                };
                if reconnect_delay != RECONNECT_DELAY {
                    info!(
                        event = "control.session.retry",
                        target = %runtime.name,
                        delay_ms = reconnect_delay.as_millis(),
                        snapshot_received = tracker.snapshot_received,
                        "retrying control session"
                    );
                }
                if wait_reconnect_or_shutdown(&shutdown, reconnect_delay).await {
                    break true;
                }
                continue;
            }
            Err(err) => {
                let error = err.to_string();
                handle_connect_failure(
                    &runtime.name,
                    error,
                    in_connect_grace(ever_snapshot, connect_grace_started),
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

        if wait_reconnect_or_shutdown(&shutdown, RECONNECT_DELAY).await {
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

async fn wait_reconnect_or_shutdown(shutdown: &CancellationToken, delay: Duration) -> bool {
    tokio::select! {
        _ = shutdown.cancelled() => true,
        _ = tokio::time::sleep(delay) => false,
    }
}

fn in_connect_grace(ever_snapshot: bool, started_at: Instant) -> bool {
    !ever_snapshot && started_at.elapsed() < INITIAL_CONNECT_GRACE
}

async fn set_connecting_status(
    name: &str,
    state: &Arc<RwLock<ConsoleState>>,
    event_tx: &broadcast::Sender<ConsoleEvent>,
) {
    set_status_and_notify(name, TargetStatus::Down, None, state, event_tx).await;
}

async fn handle_connect_failure(
    name: &str,
    error: String,
    use_grace: bool,
    state: &Arc<RwLock<ConsoleState>>,
    event_tx: &broadcast::Sender<ConsoleEvent>,
) {
    if use_grace {
        set_connecting_status(name, state, event_tx).await;
    } else {
        set_status_and_notify(name, TargetStatus::Down, Some(error), state, event_tx).await;
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
