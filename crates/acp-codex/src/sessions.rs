use std::fs;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::Value;

use crate::logging::{log_fmt, LogLevel};
use crate::utils::SessionHandler;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SessionSummary {
    pub(crate) session_id: String,
    pub(crate) title: String,
    pub(crate) cwd: String,
    pub(crate) created_at: u64,
    pub(crate) updated_at: u64,
    pub(crate) message_count: u64,
}

#[derive(Debug)]
struct SessionMetadata {
    session_id: String,
    cwd: String,
}

pub(crate) fn list_workspace_sessions() -> Result<Vec<SessionSummary>> {
    log_fmt(
        LogLevel::Info,
        format_args!("list_workspace_sessions called"),
    );
    let sessions_root = SessionHandler::sessions_root()?;
    let workspace_root = workspace_root()?;
    let mut sessions = Vec::new();
    scan_session_dir(&sessions_root, &workspace_root, &mut sessions)?;
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(sessions)
}

pub(crate) fn delete_workspace_session(session_id: &str) -> Result<()> {
    let rollout_path = SessionHandler::find_rollout_file_path(session_id)?;
    let workspace_root = workspace_root()?;
    let meta = read_session_metadata(&rollout_path)?
        .ok_or_else(|| anyhow!("failed to read session metadata"))?;
    if meta.session_id != session_id {
        return Err(anyhow!("session_id mismatch"));
    }
    if !is_within_workspace(&meta.cwd, &workspace_root) {
        return Err(anyhow!("only workspace sessions can be deleted"));
    }

    if rollout_path.exists() {
        fs::remove_file(&rollout_path)?;
    }
    let mcp_path = mcp_metadata_path(&rollout_path);
    if mcp_path.exists() {
        let _ = fs::remove_file(mcp_path);
    }
    Ok(())
}

fn scan_session_dir(
    dir: &Path,
    workspace_root: &Path,
    output: &mut Vec<SessionSummary>,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            scan_session_dir(&path, workspace_root, output)?;
            continue;
        }
        if !is_rollout_file(&path) {
            continue;
        }
        if let Ok(Some(summary)) = parse_rollout_summary(&path, workspace_root) {
            output.push(summary);
        }
    }
    Ok(())
}

fn is_rollout_file(path: &Path) -> bool {
    let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    filename.starts_with("rollout-") && filename.ends_with(".jsonl")
}

fn parse_rollout_summary(path: &Path, workspace_root: &Path) -> Result<Option<SessionSummary>> {
    let file = fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let mut session_id: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut title: Option<String> = None;
    let mut message_count = 0u64;

    for line in reader.lines() {
        let line = line?;
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let Some(entry_type) = value.get("type").and_then(|v| v.as_str()) else {
            continue;
        };
        if entry_type == "session_meta" {
            if let Some(payload) = value.get("payload") {
                if session_id.is_none() {
                    session_id = payload
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(|v| v.to_string());
                }
                if cwd.is_none() {
                    cwd = payload
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(|v| v.to_string());
                }
            }
            continue;
        }
        if entry_type == "event_msg" {
            let Some(payload) = value.get("payload") else {
                continue;
            };
            let kind = payload.get("type").and_then(|v| v.as_str());
            match kind {
                Some("user_message") => {
                    message_count = message_count.saturating_add(1);
                    if title.is_none() {
                        title = payload
                            .get("message")
                            .and_then(|v| v.as_str())
                            .map(|v| normalize_title(v));
                    }
                }
                Some("agent_message") => {
                    message_count = message_count.saturating_add(1);
                }
                _ => {}
            }
        }
    }

    let session_id = match session_id {
        Some(value) => value,
        None => return Ok(None),
    };
    let cwd = match cwd {
        Some(value) => value,
        None => return Ok(None),
    };
    if !is_within_workspace(&cwd, workspace_root) {
        return Ok(None);
    }

    let updated_at = file_time_ms(path)?.unwrap_or_else(now_ms);
    let created_at = file_created_ms(path).unwrap_or(updated_at);
    let title = title.unwrap_or_else(|| format!("Session {}", session_id));

    Ok(Some(SessionSummary {
        session_id,
        title,
        cwd,
        created_at,
        updated_at,
        message_count,
    }))
}

fn read_session_metadata(path: &Path) -> Result<Option<SessionMetadata>> {
    let file = fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    for line in reader.lines() {
        let line = line?;
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value.get("type").and_then(|v| v.as_str()) != Some("session_meta") {
            continue;
        }
        let Some(payload) = value.get("payload") else {
            continue;
        };
        let Some(session_id) = payload.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(cwd) = payload.get("cwd").and_then(|v| v.as_str()) else {
            continue;
        };
        return Ok(Some(SessionMetadata {
            session_id: session_id.to_string(),
            cwd: cwd.to_string(),
        }));
    }
    Ok(None)
}

fn workspace_root() -> Result<PathBuf> {
    let home_dir = dirs::home_dir().ok_or_else(|| anyhow!("failed to resolve HOME dir"))?;
    Ok(home_dir.join(".octovalve").join("workspace"))
}

fn is_within_workspace(cwd: &str, workspace_root: &Path) -> bool {
    let cwd_path = Path::new(cwd);
    let workspace_root = match workspace_root.canonicalize() {
        Ok(root) => root,
        Err(err) => {
            log_fmt(
                LogLevel::Warn,
                format_args!("workspace_root canonicalize failed: {err}"),
            );
            return cwd_path.starts_with(workspace_root);
        }
    };

    let cwd_display = cwd_path.display().to_string();
    let root_display = workspace_root.display().to_string();
    if !cwd_display.starts_with(&root_display) {
        return false;
    }

    match cwd_path.canonicalize() {
        Ok(target) => target.starts_with(&workspace_root),
        Err(err) => {
            log_fmt(
                LogLevel::Warn,
                format_args!("cwd canonicalize failed: {cwd_display}: {err}"),
            );
            true
        }
    }
}

fn file_time_ms(path: &Path) -> Result<Option<u64>> {
    let metadata = fs::metadata(path)?;
    let modified = metadata.modified().ok();
    Ok(modified.map(system_time_ms))
}

fn file_created_ms(path: &Path) -> Option<u64> {
    let metadata = fs::metadata(path).ok()?;
    metadata.created().ok().map(system_time_ms)
}

fn system_time_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(0)
}

fn now_ms() -> u64 {
    system_time_ms(SystemTime::now())
}

fn normalize_title(raw: &str) -> String {
    let single = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = single.trim();
    let max_len = 200;
    if trimmed.len() <= max_len {
        trimmed.to_string()
    } else {
        format!(
            "{}...",
            trimmed.chars().take(max_len - 3).collect::<String>()
        )
    }
}

fn mcp_metadata_path(rollout_path: &Path) -> PathBuf {
    let mut path = rollout_path.to_path_buf();
    path.set_extension("mcp.json");
    path
}
