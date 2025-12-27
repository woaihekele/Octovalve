use crate::activity::ActivityTracker;
use crate::layers::execution::executor::execute_request;
use crate::layers::policy::config::LimitsConfig;
use crate::layers::policy::summary::request_summary;
use crate::layers::policy::whitelist::Whitelist;
use crate::layers::service::events::{PendingRequest, ServerEvent, ServiceCommand, ServiceEvent};
use protocol::control::{RequestSnapshot, ResultSnapshot, ServiceSnapshot};
use protocol::CommandResponse;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc};

struct ServiceState {
    pending: Vec<PendingRequest>,
    history: Vec<ResultSnapshot>,
    history_limit: usize,
}

impl ServiceState {
    fn new(history: Vec<ResultSnapshot>, history_limit: usize) -> Self {
        Self {
            pending: Vec::new(),
            history,
            history_limit,
        }
    }

    fn push_result(&mut self, result: ResultSnapshot) {
        self.history.insert(0, result);
        if self.history.len() > self.history_limit {
            self.history.truncate(self.history_limit);
        }
    }

    fn snapshot(&self) -> ServiceSnapshot {
        let queue = build_queue_snapshots(&self.pending);
        let last_result = self.history.first().cloned();
        ServiceSnapshot {
            queue,
            history: self.history.clone(),
            last_result,
        }
    }
}

pub(crate) fn run_tui_service(
    listener: tokio::net::TcpListener,
    whitelist: Arc<Whitelist>,
    limits: Arc<LimitsConfig>,
    output_dir: Arc<PathBuf>,
    auto_approve_allowed: bool,
    history: Vec<ResultSnapshot>,
    history_limit: usize,
    event_tx: broadcast::Sender<ServiceEvent>,
    cmd_rx: mpsc::Receiver<ServiceCommand>,
    activity: Arc<ActivityTracker>,
) {
    let (server_tx, server_rx) = mpsc::channel::<ServerEvent>(128);
    let (result_tx, result_rx) = mpsc::channel::<ResultSnapshot>(128);
    crate::layers::service::server::spawn_accept_loop(
        listener,
        server_tx,
        Arc::clone(&output_dir),
        Arc::clone(&whitelist),
        activity,
    );
    tokio::spawn(async move {
        service_loop(
            server_rx,
            cmd_rx,
            result_rx,
            result_tx,
            event_tx,
            whitelist,
            limits,
            output_dir,
            auto_approve_allowed,
            history,
            history_limit,
        )
        .await;
    });
}

async fn service_loop(
    mut server_rx: mpsc::Receiver<ServerEvent>,
    mut cmd_rx: mpsc::Receiver<ServiceCommand>,
    mut result_rx: mpsc::Receiver<ResultSnapshot>,
    result_tx: mpsc::Sender<ResultSnapshot>,
    event_tx: broadcast::Sender<ServiceEvent>,
    whitelist: Arc<Whitelist>,
    limits: Arc<LimitsConfig>,
    output_dir: Arc<PathBuf>,
    auto_approve_allowed: bool,
    history: Vec<ResultSnapshot>,
    history_limit: usize,
) {
    let mut state = ServiceState::new(history, history_limit);
    loop {
        tokio::select! {
            Some(event) = server_rx.recv() => {
                handle_server_event(
                    event,
                    &mut state,
                    &event_tx,
                    &result_tx,
                    &whitelist,
                    &limits,
                    &output_dir,
                    auto_approve_allowed,
                )
                .await;
            }
            Some(command) = cmd_rx.recv() => {
                handle_command(
                    command,
                    &mut state,
                    &event_tx,
                    &result_tx,
                    &whitelist,
                    &limits,
                    &output_dir,
                )
                .await;
            }
            Some(result) = result_rx.recv() => {
                handle_result_snapshot(result, &mut state, &event_tx);
            }
            else => break,
        }
    }
}

async fn handle_server_event(
    event: ServerEvent,
    state: &mut ServiceState,
    event_tx: &broadcast::Sender<ServiceEvent>,
    result_tx: &mpsc::Sender<ResultSnapshot>,
    whitelist: &Arc<Whitelist>,
    limits: &Arc<LimitsConfig>,
    output_dir: &Arc<PathBuf>,
    auto_approve_allowed: bool,
) {
    match event {
        ServerEvent::ConnectionOpened => {
            let _ = event_tx.send(ServiceEvent::ConnectionsChanged);
        }
        ServerEvent::ConnectionClosed => {
            let _ = event_tx.send(ServiceEvent::ConnectionsChanged);
        }
        ServerEvent::Request(pending) => {
            if auto_approve_allowed && whitelist.allows_request(&pending.request) {
                let result_tx = result_tx.clone();
                let whitelist = Arc::clone(whitelist);
                let limits = Arc::clone(limits);
                let output_dir = Arc::clone(output_dir);
                tokio::spawn(async move {
                    tracing::info!(
                        event = "request_auto_approved",
                        id = %pending.request.id,
                        command = %request_summary(&pending.request),
                    );
                    let response =
                        execute_request(&pending.request, &whitelist, &limits, &output_dir).await;
                    let finished_at = SystemTime::now();
                    let result_snapshot =
                        result_snapshot_from_response(&pending, &response, finished_at);
                    let _ = pending.respond_to.send(response);
                    let _ = result_tx.send(result_snapshot).await;
                });
            } else {
                state.pending.push(pending);
                let queue = build_queue_snapshots(&state.pending);
                let _ = event_tx.send(ServiceEvent::QueueUpdated(queue));
            }
        }
    }
}

async fn handle_command(
    command: ServiceCommand,
    state: &mut ServiceState,
    event_tx: &broadcast::Sender<ServiceEvent>,
    result_tx: &mpsc::Sender<ResultSnapshot>,
    whitelist: &Arc<Whitelist>,
    limits: &Arc<LimitsConfig>,
    output_dir: &Arc<PathBuf>,
) {
    match command {
        ServiceCommand::Approve(id) => {
            if let Some(pending) = remove_pending(state, &id) {
                let queue = build_queue_snapshots(&state.pending);
                let _ = event_tx.send(ServiceEvent::QueueUpdated(queue));

                let result_tx = result_tx.clone();
                let whitelist = Arc::clone(whitelist);
                let limits = Arc::clone(limits);
                let output_dir = Arc::clone(output_dir);
                tokio::spawn(async move {
                    tracing::info!(
                        event = "request_approved",
                        id = %pending.request.id,
                        command = %request_summary(&pending.request),
                    );
                    let response =
                        execute_request(&pending.request, &whitelist, &limits, &output_dir).await;
                    let finished_at = SystemTime::now();
                    let result_snapshot =
                        result_snapshot_from_response(&pending, &response, finished_at);
                    let _ = pending.respond_to.send(response);
                    let _ = result_tx.send(result_snapshot).await;
                });
            }
        }
        ServiceCommand::Deny(id) => {
            if let Some(pending) = remove_pending(state, &id) {
                let queue = build_queue_snapshots(&state.pending);
                let _ = event_tx.send(ServiceEvent::QueueUpdated(queue));

                tracing::info!(
                    event = "request_denied",
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
                let output_dir = Arc::clone(output_dir);
                tokio::spawn(async move {
                    crate::layers::execution::output::write_result_record(
                        &output_dir,
                        &response,
                        Duration::from_secs(0),
                    )
                    .await;
                });
            }
        }
        ServiceCommand::Snapshot(respond_to) => {
            let _ = respond_to.send(state.snapshot());
        }
    }
}

fn handle_result_snapshot(
    result: ResultSnapshot,
    state: &mut ServiceState,
    event_tx: &broadcast::Sender<ServiceEvent>,
) {
    state.push_result(result.clone());
    let _ = event_tx.send(ServiceEvent::ResultUpdated(result));
}

fn remove_pending(state: &mut ServiceState, id: &str) -> Option<PendingRequest> {
    let index = state
        .pending
        .iter()
        .position(|pending| pending.request.id == id)?;
    Some(state.pending.remove(index))
}

fn build_queue_snapshots(pending: &[PendingRequest]) -> Vec<RequestSnapshot> {
    pending.iter().map(to_request_snapshot).collect()
}

fn to_request_snapshot(pending: &PendingRequest) -> RequestSnapshot {
    let request = &pending.request;
    RequestSnapshot {
        id: request.id.clone(),
        client: request.client.clone(),
        target: request.target.clone(),
        peer: pending.peer.clone(),
        intent: request.intent.clone(),
        mode: request.mode.clone(),
        raw_command: request.raw_command.clone(),
        pipeline: request.pipeline.clone(),
        cwd: request.cwd.clone(),
        timeout_ms: request.timeout_ms,
        max_output_bytes: request.max_output_bytes,
        received_at_ms: system_time_ms(pending.received_at),
    }
}

fn result_snapshot_from_response(
    pending: &PendingRequest,
    response: &CommandResponse,
    finished_at: SystemTime,
) -> ResultSnapshot {
    ResultSnapshot {
        id: pending.request.id.clone(),
        status: response.status.clone(),
        exit_code: response.exit_code,
        error: response.error.clone(),
        intent: pending.request.intent.clone(),
        mode: pending.request.mode.clone(),
        raw_command: pending.request.raw_command.clone(),
        pipeline: pending.request.pipeline.clone(),
        cwd: pending.request.cwd.clone(),
        peer: pending.peer.clone(),
        queued_for_secs: pending.queued_at.elapsed().as_secs(),
        finished_at_ms: system_time_ms(finished_at),
        stdout: response.stdout.clone(),
        stderr: response.stderr.clone(),
    }
}

fn system_time_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
