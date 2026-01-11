use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use tokio::sync::broadcast;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use protocol::control::{ResultSnapshot, ServiceEvent, ServiceSnapshot};
use protocol::CommandResponse;

use crate::events::ConsoleEvent;
use crate::runtime::emit_target_update;
use crate::state::{ConsoleState, ControlCommand, TargetSpec};

use super::events::{PendingRequest, ServerEvent};
use super::executor::{execute_request, PtySessionManager};
use super::history;
use super::output::spawn_write_result_record;
use super::policy::{request_summary, LimitsConfig, Whitelist};
use super::snapshots::{
    build_queue_snapshots, result_snapshot_from_response, running_snapshot_from_pending,
};

const HISTORY_LIMIT: usize = 50;

pub(crate) struct TargetServiceHandle {
    pub(crate) server_tx: mpsc::Sender<ServerEvent>,
    pub(crate) command_tx: mpsc::Sender<ControlCommand>,
    pub(crate) snapshot: ServiceSnapshot,
    pub(crate) output_dir: Arc<PathBuf>,
}

pub(super) fn spawn_service(
    target: TargetSpec,
    whitelist: Arc<Whitelist>,
    limits: Arc<LimitsConfig>,
    output_dir: Arc<PathBuf>,
    state: Arc<RwLock<ConsoleState>>,
    event_tx: broadcast::Sender<ConsoleEvent>,
) -> TargetServiceHandle {
    let (server_tx, server_rx) = mpsc::channel::<ServerEvent>(128);
    let (command_tx, command_rx) = mpsc::channel::<ControlCommand>(128);
    let (result_tx, result_rx) = mpsc::channel::<ResultSnapshot>(128);
    let history = history::load_history(&output_dir, limits.max_output_bytes, HISTORY_LIMIT);
    let pty_manager = if target.tty {
        Some(Arc::new(PtySessionManager::new(target.clone())))
    } else {
        None
    };
    let snapshot = ServiceSnapshot {
        queue: Vec::new(),
        running: Vec::new(),
        history: history.clone(),
        last_result: history.first().cloned(),
    };
    let target_name = target.name.clone();
    let service_output_dir = Arc::clone(&output_dir);
    tokio::spawn(async move {
        let service_state = ServiceState::new(history, HISTORY_LIMIT);
        service_loop(
            target_name,
            target,
            server_rx,
            command_rx,
            result_rx,
            result_tx,
            service_state,
            whitelist,
            limits,
            service_output_dir,
            pty_manager,
            state,
            event_tx,
        )
        .await;
    });
    TargetServiceHandle {
        server_tx,
        command_tx,
        snapshot,
        output_dir,
    }
}

async fn service_loop(
    target_name: String,
    target: TargetSpec,
    mut server_rx: mpsc::Receiver<ServerEvent>,
    mut command_rx: mpsc::Receiver<ControlCommand>,
    mut result_rx: mpsc::Receiver<ResultSnapshot>,
    result_tx: mpsc::Sender<ResultSnapshot>,
    mut service_state: ServiceState,
    whitelist: Arc<Whitelist>,
    limits: Arc<LimitsConfig>,
    output_dir: Arc<PathBuf>,
    pty_manager: Option<Arc<PtySessionManager>>,
    state: Arc<RwLock<ConsoleState>>,
    event_tx: broadcast::Sender<ConsoleEvent>,
) {
    loop {
        tokio::select! {
            Some(event) = server_rx.recv() => {
                handle_server_event(
                    event,
                    &target_name,
                    &mut service_state,
                    &state,
                    &event_tx,
                )
                .await;
            }
            Some(command) = command_rx.recv() => {
                handle_command(
                    command,
                    &target_name,
                    &target,
                    &mut service_state,
                    &result_tx,
                    &whitelist,
                    &limits,
                    &output_dir,
                    &pty_manager,
                    &state,
                    &event_tx,
                )
                .await;
            }
            Some(result) = result_rx.recv() => {
                handle_result_snapshot(
                    result,
                    &target_name,
                    &mut service_state,
                    &state,
                    &event_tx,
                )
                .await;
            }
            else => break,
        }
    }
}

async fn handle_server_event(
    event: ServerEvent,
    target_name: &str,
    state: &mut ServiceState,
    console_state: &Arc<RwLock<ConsoleState>>,
    event_tx: &broadcast::Sender<ConsoleEvent>,
) {
    match event {
        ServerEvent::ConnectionOpened | ServerEvent::ConnectionClosed => {
            apply_service_event(
                target_name,
                ServiceEvent::ConnectionsChanged,
                console_state,
                event_tx,
            )
            .await;
        }
        ServerEvent::Request(pending) => {
            state.pending.push(pending);
            let queue = build_queue_snapshots(&state.pending);
            apply_service_event(
                target_name,
                ServiceEvent::QueueUpdated(queue),
                console_state,
                event_tx,
            )
            .await;
            tracing::info!(
                event = "queue.updated",
                target = %target_name,
                queue_len = state.pending.len()
            );
        }
    }
}

async fn handle_command(
    command: ControlCommand,
    target_name: &str,
    target: &TargetSpec,
    state: &mut ServiceState,
    result_tx: &mpsc::Sender<ResultSnapshot>,
    whitelist: &Arc<Whitelist>,
    limits: &Arc<LimitsConfig>,
    output_dir: &Arc<PathBuf>,
    pty_manager: &Option<Arc<PtySessionManager>>,
    console_state: &Arc<RwLock<ConsoleState>>,
    event_tx: &broadcast::Sender<ConsoleEvent>,
) {
    match command {
        ControlCommand::Approve(id) => {
            if let Some(pending) = remove_pending(state, &id) {
                let queue = build_queue_snapshots(&state.pending);
                apply_service_event(
                    target_name,
                    ServiceEvent::QueueUpdated(queue),
                    console_state,
                    event_tx,
                )
                .await;
                start_execution(
                    target_name,
                    target,
                    pending,
                    state,
                    result_tx,
                    whitelist,
                    limits,
                    output_dir,
                    pty_manager.clone(),
                    console_state,
                    event_tx,
                );
            }
        }
        ControlCommand::Deny(id) => {
            if let Some(pending) = remove_pending(state, &id) {
                let queue = build_queue_snapshots(&state.pending);
                apply_service_event(
                    target_name,
                    ServiceEvent::QueueUpdated(queue),
                    console_state,
                    event_tx,
                )
                .await;

                tracing::info!(
                    event = "request_denied",
                    target = %target_name,
                    id = %pending.request.id,
                    command = %request_summary(&pending.request),
                );
                let response =
                    CommandResponse::denied(pending.request.id.clone(), "denied by operator");
                let finished_at = SystemTime::now();
                let result_snapshot =
                    result_snapshot_from_response(&pending, &response, finished_at);
                let _ = pending.respond_to.send(response.clone());
                let _ = result_tx.send(result_snapshot).await;
                spawn_write_result_record(Arc::clone(output_dir), response, Duration::from_secs(0));
            }
        }
        ControlCommand::Cancel(id) => {
            if state.cancel_running(&id) {
                tracing::info!(event = "request_cancelled", target = %target_name, id = %id);
            } else {
                tracing::warn!(event = "request_cancel_miss", target = %target_name, id = %id);
            }
        }
    }
}

async fn handle_result_snapshot(
    result: ResultSnapshot,
    target_name: &str,
    state: &mut ServiceState,
    console_state: &Arc<RwLock<ConsoleState>>,
    event_tx: &broadcast::Sender<ConsoleEvent>,
) {
    if state.finish_running(&result.id) {
        apply_service_event(
            target_name,
            ServiceEvent::RunningUpdated(state.running.clone()),
            console_state,
            event_tx,
        )
        .await;
    }
    state.push_result(result.clone());
    apply_service_event(
        target_name,
        ServiceEvent::ResultUpdated(result),
        console_state,
        event_tx,
    )
    .await;
}

fn start_execution(
    target_name: &str,
    target: &TargetSpec,
    pending: PendingRequest,
    state: &mut ServiceState,
    result_tx: &mpsc::Sender<ResultSnapshot>,
    whitelist: &Arc<Whitelist>,
    limits: &Arc<LimitsConfig>,
    output_dir: &Arc<PathBuf>,
    pty_manager: Option<Arc<PtySessionManager>>,
    console_state: &Arc<RwLock<ConsoleState>>,
    event_tx: &broadcast::Sender<ConsoleEvent>,
) {
    tracing::info!(
        event = "request_approved",
        target = %target_name,
        id = %pending.request.id,
        command = %request_summary(&pending.request),
    );
    let started_at = SystemTime::now();
    let running_snapshot = running_snapshot_from_pending(&pending, started_at);
    let cancel_token = CancellationToken::new();
    state.start_running(running_snapshot, cancel_token.clone());
    let event = ServiceEvent::RunningUpdated(state.running.clone());
    let console_state = Arc::clone(console_state);
    let event_tx = event_tx.clone();
    let target_name = target_name.to_string();
    tokio::spawn(async move {
        apply_service_event(&target_name, event, &console_state, &event_tx).await;
    });

    let result_tx = result_tx.clone();
    let whitelist = Arc::clone(whitelist);
    let limits = Arc::clone(limits);
    let target = target.clone();
    let output_dir = Arc::clone(output_dir);
    tokio::spawn(async move {
        let started_at = Instant::now();
        let response = execute_request(
            &target,
            &pending.request,
            &whitelist,
            &limits,
            pty_manager,
            cancel_token,
        )
        .await;
        let duration = started_at.elapsed();
        let finished_at = SystemTime::now();
        let result_snapshot = result_snapshot_from_response(&pending, &response, finished_at);
        spawn_write_result_record(Arc::clone(&output_dir), response.clone(), duration);
        let _ = pending.respond_to.send(response);
        let _ = result_tx.send(result_snapshot).await;
    });
}

async fn apply_service_event(
    target_name: &str,
    event: ServiceEvent,
    console_state: &Arc<RwLock<ConsoleState>>,
    event_tx: &broadcast::Sender<ConsoleEvent>,
) {
    {
        let mut guard = console_state.write().await;
        guard.apply_event(target_name, event);
    }
    emit_target_update(target_name, console_state, event_tx).await;
}

fn remove_pending(state: &mut ServiceState, id: &str) -> Option<PendingRequest> {
    let index = state
        .pending
        .iter()
        .position(|pending| pending.request.id == id)?;
    Some(state.pending.remove(index))
}

struct ServiceState {
    pending: Vec<PendingRequest>,
    running: Vec<protocol::control::RunningSnapshot>,
    running_tokens: HashMap<String, CancellationToken>,
    history: Vec<ResultSnapshot>,
    history_limit: usize,
}

impl ServiceState {
    fn new(history: Vec<ResultSnapshot>, history_limit: usize) -> Self {
        Self {
            pending: Vec::new(),
            running: Vec::new(),
            running_tokens: HashMap::new(),
            history,
            history_limit,
        }
    }

    fn start_running(
        &mut self,
        running: protocol::control::RunningSnapshot,
        token: CancellationToken,
    ) {
        self.running.retain(|item| item.id != running.id);
        self.running.insert(0, running.clone());
        self.running_tokens.insert(running.id, token);
    }

    fn finish_running(&mut self, id: &str) -> bool {
        let before = self.running.len();
        self.running.retain(|item| item.id != id);
        self.running_tokens.remove(id);
        before != self.running.len()
    }

    fn cancel_running(&mut self, id: &str) -> bool {
        if let Some(token) = self.running_tokens.get(id) {
            token.cancel();
            return true;
        }
        false
    }

    fn push_result(&mut self, result: ResultSnapshot) {
        self.history.insert(0, result);
        if self.history.len() > self.history_limit {
            self.history.truncate(self.history_limit);
        }
    }
}
