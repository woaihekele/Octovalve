use std::time::{SystemTime, UNIX_EPOCH};

use protocol::control::{RequestSnapshot, ResultSnapshot, RunningSnapshot};
use protocol::CommandResponse;

use super::events::PendingRequest;

pub(super) fn build_queue_snapshots(pending: &[PendingRequest]) -> Vec<RequestSnapshot> {
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

pub(super) fn running_snapshot_from_pending(
    pending: &PendingRequest,
    started_at: SystemTime,
) -> RunningSnapshot {
    let request = &pending.request;
    let queued_for_secs = started_at
        .duration_since(pending.received_at)
        .map(|duration| duration.as_secs())
        .unwrap_or_else(|_| pending.queued_at.elapsed().as_secs());
    RunningSnapshot {
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
        queued_for_secs,
        started_at_ms: system_time_ms(started_at),
    }
}

pub(super) fn result_snapshot_from_response(
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
