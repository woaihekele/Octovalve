use protocol::{CommandResponse, CommandStatus};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

#[derive(Serialize)]
struct ResultRecord {
    id: String,
    status: CommandStatus,
    exit_code: Option<i32>,
    error: Option<String>,
    duration_ms: u128,
}

pub(crate) fn spawn_write_result_record(
    output_dir: Arc<PathBuf>,
    response: CommandResponse,
    duration: Duration,
) {
    tokio::spawn(async move {
        write_result_record(&output_dir, &response, duration).await;
        write_output_files(&output_dir, &response).await;
    });
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

pub(crate) async fn write_output_files(output_dir: &Path, response: &CommandResponse) {
    if let Some(stdout) = response.stdout.as_ref() {
        let path = output_dir.join(format!("{}.stdout", response.id));
        if let Err(err) = tokio::fs::write(path, stdout).await {
            tracing::warn!(error = %err, "failed to write stdout output");
        }
    }
    if let Some(stderr) = response.stderr.as_ref() {
        let path = output_dir.join(format!("{}.stderr", response.id));
        if let Err(err) = tokio::fs::write(path, stderr).await {
            tracing::warn!(error = %err, "failed to write stderr output");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_utils::temp_dir;
    use std::fs;

    #[test]
    fn writes_result_and_outputs() {
        let dir = temp_dir("octovalve-output");
        let response = CommandResponse {
            id: "req-1".to_string(),
            status: CommandStatus::Completed,
            exit_code: Some(0),
            stdout: Some("ok".to_string()),
            stderr: Some("warn".to_string()),
            error: None,
        };
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        rt.block_on(async {
            write_result_record(&dir, &response, Duration::from_millis(10)).await;
            write_output_files(&dir, &response).await;
        });
        assert!(dir.join("req-1.result.json").exists());
        assert_eq!(fs::read_to_string(dir.join("req-1.stdout")).unwrap(), "ok");
        assert_eq!(
            fs::read_to_string(dir.join("req-1.stderr")).unwrap(),
            "warn"
        );
        fs::remove_dir_all(&dir).ok();
    }
}
