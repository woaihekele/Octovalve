use crate::control::{ControlRequest, ControlResponse};
use crate::state::{ConsoleState, ControlCommand, TargetSpec, TargetStatus};
use crate::tunnel::{spawn_tunnel, stop_tunnel, TargetRuntime};
use anyhow::Context;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, RwLock};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tokio_util::sync::CancellationToken;

const RECONNECT_DELAY: Duration = Duration::from_secs(5);

pub(crate) fn spawn_target_workers(state: Arc<RwLock<ConsoleState>>, shutdown: CancellationToken) {
    tokio::spawn(async move {
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
            let state = Arc::clone(&state);
            let shutdown = shutdown.clone();
            tokio::spawn(async move {
                run_target_worker(spec, state, rx, shutdown).await;
            });
        }
    });
}

async fn run_target_worker(
    spec: TargetSpec,
    state: Arc<RwLock<ConsoleState>>,
    mut cmd_rx: mpsc::Receiver<ControlCommand>,
    shutdown: CancellationToken,
) {
    let mut runtime = TargetRuntime::from_spec(spec);
    loop {
        if shutdown.is_cancelled() {
            stop_tunnel(&mut runtime).await;
            break;
        }

        if runtime.ssh.is_some() {
            if !runtime.refresh_tunnel() {
                if let Err(err) = spawn_tunnel(&mut runtime) {
                    let mut state = state.write().await;
                    state.set_status(&runtime.name, TargetStatus::Down, Some(err.to_string()));
                    drop(state);
                    tokio::time::sleep(RECONNECT_DELAY).await;
                    continue;
                }
            }
        }

        let addr = runtime.connect_addr();
        match connect_control(&addr).await {
            Ok(mut framed) => {
                {
                    let mut state = state.write().await;
                    state.set_status(&runtime.name, TargetStatus::Ready, None);
                }
                if let Err(err) = send_request(&mut framed, ControlRequest::Subscribe).await {
                    let mut state = state.write().await;
                    state.set_status(&runtime.name, TargetStatus::Down, Some(err.to_string()));
                    tokio::time::sleep(RECONNECT_DELAY).await;
                    continue;
                }
                if let Err(err) = send_request(&mut framed, ControlRequest::Snapshot).await {
                    let mut state = state.write().await;
                    state.set_status(&runtime.name, TargetStatus::Down, Some(err.to_string()));
                    tokio::time::sleep(RECONNECT_DELAY).await;
                    continue;
                }

                if let Err(err) =
                    session_loop(&mut framed, &runtime.name, &state, &mut cmd_rx, &shutdown).await
                {
                    let mut state = state.write().await;
                    state.set_status(&runtime.name, TargetStatus::Down, Some(err.to_string()));
                }
            }
            Err(err) => {
                let mut state = state.write().await;
                state.set_status(&runtime.name, TargetStatus::Down, Some(err.to_string()));
            }
        }

        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

async fn session_loop(
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    name: &str,
    state: &Arc<RwLock<ConsoleState>>,
    cmd_rx: &mut mpsc::Receiver<ControlCommand>,
    shutdown: &CancellationToken,
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
                handle_response(name, state, response).await;
            }
        }
    }
}

async fn handle_response(name: &str, state: &Arc<RwLock<ConsoleState>>, response: ControlResponse) {
    match response {
        ControlResponse::Snapshot { snapshot } => {
            let mut state = state.write().await;
            state.apply_snapshot(name, snapshot);
        }
        ControlResponse::Event { event } => {
            let mut state = state.write().await;
            state.apply_event(name, event);
        }
        ControlResponse::Ack { .. } => {}
        ControlResponse::Error { message } => {
            let mut state = state.write().await;
            state.set_status(name, TargetStatus::Ready, Some(message));
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
            tunnel: None,
            tunnel_pgid: None,
        }
    }
}
