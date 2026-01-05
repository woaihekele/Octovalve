use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use protocol::control::ResultSnapshot;
use protocol::CommandResponse;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::layers::execution::executor::execute_request;
use crate::layers::policy::config::LimitsConfig;
use crate::layers::policy::summary::request_summary;
use crate::layers::policy::whitelist::Whitelist;
use crate::layers::service::events::{PendingRequest, ServerEvent, ServiceCommand, ServiceEvent};

use super::snapshots::{build_queue_snapshots, result_snapshot_from_response, running_snapshot_from_pending};
use super::state::ServiceState;

pub(crate) async fn handle_server_event(
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
                start_execution(
                    pending,
                    state,
                    event_tx,
                    result_tx,
                    whitelist,
                    limits,
                    output_dir,
                    "request_auto_approved",
                );
            } else {
                state.pending.push(pending);
                tracing::info!(event = "queue.updated", queue_len = state.pending.len());
                let queue = build_queue_snapshots(&state.pending);
                let _ = event_tx.send(ServiceEvent::QueueUpdated(queue));
            }
        }
    }
}

pub(crate) async fn handle_command(
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
                tracing::info!(event = "queue.updated", queue_len = state.pending.len());
                let queue = build_queue_snapshots(&state.pending);
                let _ = event_tx.send(ServiceEvent::QueueUpdated(queue));

                start_execution(
                    pending,
                    state,
                    event_tx,
                    result_tx,
                    whitelist,
                    limits,
                    output_dir,
                    "request_approved",
                );
            }
        }
        ServiceCommand::Deny(id) => {
            if let Some(pending) = remove_pending(state, &id) {
                tracing::info!(event = "queue.updated", queue_len = state.pending.len());
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
        ServiceCommand::Cancel(id) => {
            if state.cancel_running(&id) {
                tracing::info!(event = "request_cancelled", id = %id);
            } else {
                tracing::warn!(event = "request_cancel_miss", id = %id);
            }
        }
        ServiceCommand::Snapshot(respond_to) => {
            let _ = respond_to.send(state.snapshot());
        }
    }
}

pub(crate) fn handle_result_snapshot(
    result: ResultSnapshot,
    state: &mut ServiceState,
    event_tx: &broadcast::Sender<ServiceEvent>,
) {
    if state.finish_running(&result.id) {
        let _ = event_tx.send(ServiceEvent::RunningUpdated(state.running.clone()));
    }
    state.push_result(result.clone());
    let _ = event_tx.send(ServiceEvent::ResultUpdated(result));
}

fn start_execution(
    pending: PendingRequest,
    state: &mut ServiceState,
    event_tx: &broadcast::Sender<ServiceEvent>,
    result_tx: &mpsc::Sender<ResultSnapshot>,
    whitelist: &Arc<Whitelist>,
    limits: &Arc<LimitsConfig>,
    output_dir: &Arc<PathBuf>,
    event_label: &'static str,
) {
    let started_at = SystemTime::now();
    let running_snapshot = running_snapshot_from_pending(&pending, started_at);
    let cancel_token = CancellationToken::new();
    state.start_running(running_snapshot, cancel_token.clone());
    let _ = event_tx.send(ServiceEvent::RunningUpdated(state.running.clone()));

    let result_tx = result_tx.clone();
    let whitelist = Arc::clone(whitelist);
    let limits = Arc::clone(limits);
    let output_dir = Arc::clone(output_dir);
    tokio::spawn(async move {
        tracing::info!(
            event = event_label,
            id = %pending.request.id,
            command = %request_summary(&pending.request),
        );
        let response =
            execute_request(
                &pending.request,
                &whitelist,
                &limits,
                &output_dir,
                cancel_token,
            )
            .await;
        let finished_at = SystemTime::now();
        let result_snapshot =
            result_snapshot_from_response(&pending, &response, finished_at);
        let _ = pending.respond_to.send(response);
        let _ = result_tx.send(result_snapshot).await;
    });
}

fn remove_pending(state: &mut ServiceState, id: &str) -> Option<PendingRequest> {
    let index = state
        .pending
        .iter()
        .position(|pending| pending.request.id == id)?;
    Some(state.pending.remove(index))
}
