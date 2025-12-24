use protocol::{CommandResponse, CommandStatus};
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

#[derive(Serialize)]
struct ResultRecord {
    id: String,
    status: CommandStatus,
    exit_code: Option<i32>,
    error: Option<String>,
    duration_ms: u128,
}

pub(crate) async fn write_result_record(
    output_dir: &Path,
    response: &CommandResponse,
    duration: Duration,
) {
    let record = ResultRecord {
        id: response.id.clone(),
        status: response.status.clone(),
        exit_code: response.exit_code,
        error: response.error.clone(),
        duration_ms: duration.as_millis(),
    };
    let path = output_dir.join(format!("{}.result.json", response.id));
    if let Ok(payload) = serde_json::to_vec_pretty(&record) {
        if let Err(err) = tokio::fs::write(path, payload).await {
            tracing::warn!(error = %err, "failed to write result record");
        }
    }
}
