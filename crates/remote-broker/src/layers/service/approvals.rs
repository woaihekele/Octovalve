use crate::layers::execution::executor::execute_request;
use crate::layers::policy::config::LimitsConfig;
use crate::layers::policy::summary::{format_mode, format_pipeline, request_summary};
use crate::layers::policy::whitelist::Whitelist;
use crate::layers::service::events::{PendingRequest, ServerEvent, ServiceCommand, ServiceEvent};
use crate::shared::dto::{RequestView, ResultView};
use protocol::{CommandResponse, CommandStatus};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

struct ServiceState {
    connections: usize,
    pending: Vec<PendingRequest>,
}

impl Default for ServiceState {
    fn default() -> Self {
        Self {
            connections: 0,
            pending: Vec::new(),
        }
    }
}

pub(crate) fn run_tui_service(
    listener: tokio::net::TcpListener,
    whitelist: Arc<Whitelist>,
    limits: Arc<LimitsConfig>,
    output_dir: Arc<PathBuf>,
    auto_approve_allowed: bool,
    ui_tx: mpsc::Sender<ServiceEvent>,
    ui_cmd_rx: mpsc::Receiver<ServiceCommand>,
) {
    let (server_tx, server_rx) = mpsc::channel::<ServerEvent>(128);
    crate::layers::service::server::spawn_accept_loop(
        listener,
        server_tx,
        Arc::clone(&output_dir),
        Arc::clone(&whitelist),
    );
    tokio::spawn(async move {
        service_loop(
            server_rx,
            ui_cmd_rx,
            ui_tx,
            whitelist,
            limits,
            output_dir,
            auto_approve_allowed,
        )
        .await;
    });
}

async fn service_loop(
    mut server_rx: mpsc::Receiver<ServerEvent>,
    mut ui_cmd_rx: mpsc::Receiver<ServiceCommand>,
    ui_tx: mpsc::Sender<ServiceEvent>,
    whitelist: Arc<Whitelist>,
    limits: Arc<LimitsConfig>,
    output_dir: Arc<PathBuf>,
    auto_approve_allowed: bool,
) {
    let mut state = ServiceState::default();
    loop {
        tokio::select! {
            Some(event) = server_rx.recv() => {
                handle_server_event(
                    event,
                    &mut state,
                    &ui_tx,
                    &whitelist,
                    &limits,
                    &output_dir,
                    auto_approve_allowed,
                )
                .await;
            }
            Some(command) = ui_cmd_rx.recv() => {
                handle_command(command, &mut state, &ui_tx, &whitelist, &limits, &output_dir).await;
            }
            else => break,
        }
    }
}

async fn handle_server_event(
    event: ServerEvent,
    state: &mut ServiceState,
    ui_tx: &mpsc::Sender<ServiceEvent>,
    whitelist: &Arc<Whitelist>,
    limits: &Arc<LimitsConfig>,
    output_dir: &Arc<PathBuf>,
    auto_approve_allowed: bool,
) {
    match event {
        ServerEvent::ConnectionOpened => {
            state.connections += 1;
            let _ = ui_tx
                .send(ServiceEvent::ConnectionsChanged(state.connections))
                .await;
        }
        ServerEvent::ConnectionClosed => {
            state.connections = state.connections.saturating_sub(1);
            let _ = ui_tx
                .send(ServiceEvent::ConnectionsChanged(state.connections))
                .await;
        }
        ServerEvent::Request(pending) => {
            if auto_approve_allowed && whitelist.allows_request(&pending.request) {
                let ui_tx = ui_tx.clone();
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
                    let result_view = result_view_from_response(&pending, &response, finished_at);
                    let _ = pending.respond_to.send(response);
                    let _ = ui_tx.send(ServiceEvent::ResultUpdated(result_view)).await;
                });
            } else {
                state.pending.push(pending);
                let queue = build_queue_views(&state.pending);
                let _ = ui_tx.send(ServiceEvent::QueueUpdated(queue)).await;
            }
        }
    }
}

async fn handle_command(
    command: ServiceCommand,
    state: &mut ServiceState,
    ui_tx: &mpsc::Sender<ServiceEvent>,
    whitelist: &Arc<Whitelist>,
    limits: &Arc<LimitsConfig>,
    output_dir: &Arc<PathBuf>,
) {
    match command {
        ServiceCommand::Approve(id) => {
            if let Some(pending) = remove_pending(state, &id) {
                let queue = build_queue_views(&state.pending);
                let _ = ui_tx.send(ServiceEvent::QueueUpdated(queue)).await;

                let ui_tx = ui_tx.clone();
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
                    let result_view = result_view_from_response(&pending, &response, finished_at);
                    let _ = pending.respond_to.send(response);
                    let _ = ui_tx.send(ServiceEvent::ResultUpdated(result_view)).await;
                });
            }
        }
        ServiceCommand::Deny(id) => {
            if let Some(pending) = remove_pending(state, &id) {
                let queue = build_queue_views(&state.pending);
                let _ = ui_tx.send(ServiceEvent::QueueUpdated(queue)).await;

                tracing::info!(
                    event = "request_denied",
                    id = %pending.request.id,
                    command = %request_summary(&pending.request),
                );
                let response =
                    CommandResponse::denied(pending.request.id.clone(), "denied by operator");
                let finished_at = SystemTime::now();
                let result_view = result_view_from_response(&pending, &response, finished_at);
                let _ = pending.respond_to.send(response.clone());
                let _ = ui_tx.send(ServiceEvent::ResultUpdated(result_view)).await;
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
    }
}

fn remove_pending(state: &mut ServiceState, id: &str) -> Option<PendingRequest> {
    let index = state
        .pending
        .iter()
        .position(|pending| pending.request.id == id)?;
    Some(state.pending.remove(index))
}

fn build_queue_views(pending: &[PendingRequest]) -> Vec<RequestView> {
    pending.iter().map(to_request_view).collect()
}

fn to_request_view(pending: &PendingRequest) -> RequestView {
    let request = &pending.request;
    let pipeline = if request.pipeline.is_empty() {
        None
    } else {
        Some(format_pipeline(&request.pipeline))
    };
    RequestView {
        id: request.id.clone(),
        summary: request_summary(request),
        client: request.client.clone(),
        target: request.target.clone(),
        peer: pending.peer.clone(),
        intent: request.intent.clone(),
        mode: format_mode(&request.mode).to_string(),
        command: request.raw_command.clone(),
        pipeline,
        cwd: request.cwd.clone(),
        timeout_ms: request.timeout_ms,
        max_output_bytes: request.max_output_bytes,
        queued_at: pending.queued_at,
    }
}

fn result_view_from_response(
    pending: &PendingRequest,
    response: &CommandResponse,
    finished_at: SystemTime,
) -> ResultView {
    let summary = match response.status {
        CommandStatus::Completed => format!("completed (exit={:?})", response.exit_code),
        CommandStatus::Denied => "denied".to_string(),
        CommandStatus::Error => "error".to_string(),
        CommandStatus::Approved => "approved".to_string(),
    };
    let pipeline = if pending.request.pipeline.is_empty() {
        Some(pending.request.raw_command.clone())
    } else {
        Some(format_pipeline(&pending.request.pipeline))
    };
    ResultView {
        id: pending.request.id.clone(),
        summary,
        command: pending.request.raw_command.clone(),
        peer: pending.peer.clone(),
        intent: pending.request.intent.clone(),
        mode: format_mode(&pending.request.mode).to_string(),
        pipeline,
        cwd: pending.request.cwd.clone(),
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
