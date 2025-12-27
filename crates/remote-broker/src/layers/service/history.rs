use protocol::control::ResultSnapshot;
use protocol::{CommandMode, CommandStage, CommandStatus};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Deserialize)]
struct RequestRecord {
    id: String,
    peer: String,
    intent: String,
    mode: CommandMode,
    #[serde(default)]
    command: String,
    #[serde(default)]
    raw_command: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    received_at_ms: u64,
    #[serde(default)]
    pipeline: Vec<CommandStage>,
}

#[derive(Debug, Deserialize)]
struct ResultRecord {
    id: String,
    status: CommandStatus,
    #[serde(default)]
    exit_code: Option<i32>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    duration_ms: u128,
}

pub(crate) fn load_history(
    output_dir: &Path,
    max_output_bytes: u64,
    limit: usize,
) -> Vec<ResultSnapshot> {
    let request_records = load_request_records(output_dir);
    let result_files = collect_result_files(output_dir);
    let mut results = Vec::new();
    for (path, finished_at_ms) in result_files.into_iter().take(limit) {
        let record = match read_json::<ResultRecord>(&path) {
            Ok(record) => record,
            Err(err) => {
                tracing::warn!(error = %err, path = %path.display(), "failed to read result record");
                continue;
            }
        };
        let Some(request) = request_records.get(&record.id) else {
            tracing::warn!(
                id = %record.id,
                path = %path.display(),
                "missing request record for result"
            );
            continue;
        };
        let raw_command = if request.raw_command.is_empty() {
            request.command.clone()
        } else {
            request.raw_command.clone()
        };
        let finished_at_ms = finished_at_ms
            .or_else(|| {
                request
                    .received_at_ms
                    .checked_add(record.duration_ms as u64)
            })
            .unwrap_or(0);
        let queued_for_secs =
            if request.received_at_ms > 0 && finished_at_ms >= request.received_at_ms {
                (finished_at_ms - request.received_at_ms) / 1000
            } else {
                (record.duration_ms / 1000) as u64
            };
        let stdout = read_text_limited(
            output_dir.join(format!("{}.stdout", record.id)),
            max_output_bytes,
        );
        let stderr = read_text_limited(
            output_dir.join(format!("{}.stderr", record.id)),
            max_output_bytes,
        );
        results.push(ResultSnapshot {
            id: record.id.clone(),
            status: record.status,
            exit_code: record.exit_code,
            error: record.error,
            intent: request.intent.clone(),
            mode: request.mode.clone(),
            raw_command,
            pipeline: request.pipeline.clone(),
            cwd: request.cwd.clone(),
            peer: request.peer.clone(),
            queued_for_secs,
            finished_at_ms,
            stdout,
            stderr,
        });
    }
    results.sort_by(|a, b| b.finished_at_ms.cmp(&a.finished_at_ms));
    if results.len() > limit {
        results.truncate(limit);
    }
    results
}

fn load_request_records(output_dir: &Path) -> HashMap<String, RequestRecord> {
    let mut records = HashMap::new();
    for entry in fs::read_dir(output_dir).into_iter().flatten() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::warn!(error = %err, "failed to read request directory");
                continue;
            }
        };
        let path = entry.path();
        if !is_request_record(&path) {
            continue;
        }
        match read_json::<RequestRecord>(&path) {
            Ok(record) => {
                records.insert(record.id.clone(), record);
            }
            Err(err) => {
                tracing::warn!(error = %err, path = %path.display(), "failed to read request record");
            }
        }
    }
    records
}

fn collect_result_files(output_dir: &Path) -> Vec<(PathBuf, Option<u64>)> {
    let mut files = Vec::new();
    for entry in fs::read_dir(output_dir).into_iter().flatten() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                tracing::warn!(error = %err, "failed to read result directory");
                continue;
            }
        };
        let path = entry.path();
        if !is_result_record(&path) {
            continue;
        }
        let finished_at_ms = entry
            .metadata()
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(system_time_ms);
        files.push((path, finished_at_ms));
    }
    files.sort_by(|a, b| b.1.cmp(&a.1));
    files
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> anyhow::Result<T> {
    let payload = fs::read(path)?;
    let record = serde_json::from_slice(&payload)?;
    Ok(record)
}

fn read_text_limited(path: PathBuf, max_bytes: u64) -> Option<String> {
    let file = File::open(&path).ok()?;
    let mut buf = Vec::new();
    let mut handle = file.take(max_bytes);
    handle.read_to_end(&mut buf).ok()?;
    let mut text = String::from_utf8_lossy(&buf).to_string();
    if let Ok(meta) = fs::metadata(&path) {
        if meta.len() > max_bytes {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str("[output truncated]");
        }
    }
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn is_request_record(path: &Path) -> bool {
    path.extension().map(|ext| ext == "json").unwrap_or(false)
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.ends_with(".request.json"))
            .unwrap_or(false)
}

fn is_result_record(path: &Path) -> bool {
    path.extension().map(|ext| ext == "json").unwrap_or(false)
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.ends_with(".result.json"))
            .unwrap_or(false)
}

fn system_time_ms(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}
