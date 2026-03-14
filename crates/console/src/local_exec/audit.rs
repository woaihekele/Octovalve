use protocol::{CommandRequest, CommandStage};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::events::PendingRequest;

#[derive(Serialize)]
pub(crate) struct RequestRecord {
    id: String,
    client: String,
    target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_desc: Option<String>,
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
    aggressive_mode: bool,
}

#[derive(Serialize)]
pub(crate) struct AiReviewRecord {
    pub(crate) id: String,
    pub(crate) endpoint: String,
    pub(crate) model: String,
    pub(crate) min_confidence: f64,
    pub(crate) read_only: bool,
    pub(crate) confidence: f64,
    pub(crate) reason: String,
    pub(crate) risk_flags: Vec<String>,
    pub(crate) auto_approved: bool,
    pub(crate) reviewed_at_ms: u64,
}

impl RequestRecord {
    pub(crate) fn from_request(
        request: &CommandRequest,
        peer: &str,
        received_at: SystemTime,
    ) -> Self {
        Self {
            id: request.id.clone(),
            client: request.client.clone(),
            target: request.target.clone(),
            target_host: None,
            target_desc: None,
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
            aggressive_mode: false,
        }
    }
}

pub(crate) fn spawn_write_request_record(output_dir: Arc<PathBuf>, pending: &PendingRequest) {
    let mut record =
        RequestRecord::from_request(&pending.request, &pending.peer, pending.received_at);
    record.target_host = pending.target_host.clone();
    record.target_desc = pending.target_desc.clone();
    record.aggressive_mode = pending.aggressive_mode;
    spawn_write_request_record_value(output_dir, record);
}

pub(crate) fn spawn_write_request_record_value(output_dir: Arc<PathBuf>, record: RequestRecord) {
    tokio::spawn(async move {
        if let Err(err) = write_request_record(&output_dir, &record).await {
            tracing::warn!(error = %err, "failed to write request record");
        }
    });
}

pub(crate) fn spawn_write_ai_review_record(output_dir: Arc<PathBuf>, record: AiReviewRecord) {
    tokio::spawn(async move {
        if let Err(err) = write_ai_review_record(&output_dir, &record).await {
            tracing::warn!(error = %err, "failed to write ai review record");
        }
    });
}

pub(crate) async fn write_request_record(
    output_dir: &Path,
    record: &RequestRecord,
) -> anyhow::Result<()> {
    let path = output_dir.join(format!("{}.request.json", record.id));
    let payload = serde_json::to_vec_pretty(record)?;
    tokio::fs::write(path, payload).await?;
    Ok(())
}

pub(crate) async fn write_ai_review_record(
    output_dir: &Path,
    record: &AiReviewRecord,
) -> anyhow::Result<()> {
    let path = output_dir.join(format!("{}.ai-review.json", record.id));
    let payload = serde_json::to_vec_pretty(record)?;
    tokio::fs::write(path, payload).await?;
    Ok(())
}

fn system_time_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
