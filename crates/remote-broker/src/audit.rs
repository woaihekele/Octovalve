use crate::app::PendingRequest;
use protocol::{CommandRequest, CommandStage};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize)]
pub(crate) struct RequestRecord {
    id: String,
    client: String,
    target: String,
    peer: String,
    received_at_ms: u64,
    intent: String,
    mode: protocol::CommandMode,
    command: String,
    raw_command: String,
    cwd: Option<String>,
    env: Option<std::collections::BTreeMap<String, String>>,
    timeout_ms: Option<u64>,
    max_output_bytes: Option<u64>,
    pipeline: Vec<CommandStage>,
}

impl RequestRecord {
    pub(crate) fn from_request(request: &CommandRequest, peer: &str, received_at: SystemTime) -> Self {
        Self {
            id: request.id.clone(),
            client: request.client.clone(),
            target: request.target.clone(),
            peer: peer.to_string(),
            received_at_ms: system_time_ms(received_at),
            intent: request.intent.clone(),
            mode: request.mode.clone(),
            command: request.raw_command.clone(),
            raw_command: request.raw_command.clone(),
            cwd: request.cwd.clone(),
            env: request.env.clone(),
            timeout_ms: request.timeout_ms,
            max_output_bytes: request.max_output_bytes,
            pipeline: request.pipeline.clone(),
        }
    }
}

pub(crate) fn spawn_write_request_record(output_dir: Arc<PathBuf>, pending: &PendingRequest) {
    let record = RequestRecord::from_request(&pending.request, &pending.peer, pending.received_at);
    spawn_write_request_record_value(output_dir, record);
}

pub(crate) fn spawn_write_request_record_value(output_dir: Arc<PathBuf>, record: RequestRecord) {
    tokio::spawn(async move {
        if let Err(err) = write_request_record(&output_dir, &record).await {
            tracing::warn!(error = %err, "failed to write request record");
        }
    });
}

pub(crate) async fn write_request_record(output_dir: &Path, record: &RequestRecord) -> anyhow::Result<()> {
    let path = output_dir.join(format!("{}.request.json", record.id));
    let payload = serde_json::to_vec_pretty(record)?;
    tokio::fs::write(path, payload).await?;
    Ok(())
}

fn system_time_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
