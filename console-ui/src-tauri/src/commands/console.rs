use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

use crate::services::console_http::{
    console_get, console_post, console_post_with_timeout, HTTP_RELOAD_TIMEOUT,
};
use crate::services::console_sidecar::{start_console, stop_console};
use crate::services::startup_check;
use crate::services::console_ws::start_console_stream as start_console_stream_service;
use crate::services::logging::append_log_line;
use crate::state::{AppLogState, ProfilesState, ProxyConfigState};
use crate::types::{LogChunk, StartupCheckResult};

fn console_log_path(app: &AppHandle) -> Result<PathBuf, String> {
    let config_dir = app.path().app_config_dir().map_err(|err| err.to_string())?;
    Ok(config_dir.join("logs").join("console.log"))
}

#[tauri::command]
pub fn read_console_log(offset: u64, max_bytes: u64, app: AppHandle) -> Result<LogChunk, String> {
    let path = console_log_path(&app)?;
    if !path.exists() {
        return Ok(LogChunk {
            content: String::new(),
            next_offset: 0,
        });
    }
    let mut file = OpenOptions::new()
        .read(true)
        .open(&path)
        .map_err(|err| err.to_string())?;
    let len = file.metadata().map_err(|err| err.to_string())?.len();
    let start = if offset > len { 0 } else { offset };
    file.seek(SeekFrom::Start(start))
        .map_err(|err| err.to_string())?;
    if max_bytes == 0 {
        return Ok(LogChunk {
            content: String::new(),
            next_offset: len,
        });
    }
    let capped = max_bytes.min(256 * 1024) as usize;
    let mut buffer = vec![0u8; capped];
    let read = file.read(&mut buffer).map_err(|err| err.to_string())?;
    buffer.truncate(read);
    Ok(LogChunk {
        content: String::from_utf8_lossy(&buffer).to_string(),
        next_offset: start + read as u64,
    })
}

#[tauri::command]
pub fn restart_console(
    app: AppHandle,
    state: State<ProxyConfigState>,
    log_state: State<AppLogState>,
) -> Result<(), String> {
    let console_log = console_log_path(&app)?;
    let _ = append_log_line(&console_log, "console restart requested");
    stop_console(&app);
    let status = state.0.lock().unwrap().clone();
    if !status.present {
        return Err("proxy config missing".to_string());
    }
    match start_console(&app, Path::new(&status.path), &log_state.app_log) {
        Ok(_) => {
            let _ = append_log_line(&console_log, "console restart started");
        }
        Err(err) => {
            let _ = append_log_line(&console_log, &format!("console restart failed: {err}"));
            return Err(err);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn validate_startup_config(
    app: AppHandle,
    proxy_state: State<ProxyConfigState>,
    profiles_state: State<ProfilesState>,
) -> Result<StartupCheckResult, String> {
    let status = proxy_state.0.lock().unwrap().clone();
    let profiles = profiles_state.0.lock().unwrap().clone();
    startup_check::validate_startup_config(&app, &status, &profiles)
}

#[tauri::command]
pub fn log_ui_event(message: String, state: State<AppLogState>) -> Result<(), String> {
    append_log_line(&state.app_log, &message)
}

#[tauri::command]
pub async fn proxy_fetch_targets(log_state: State<'_, AppLogState>) -> Result<Value, String> {
    let targets = console_get("/targets", &log_state.app_log).await?;
    let count = targets.as_array().map(|value| value.len()).unwrap_or(0);
    let _ = append_log_line(
        &log_state.app_log,
        &format!("fetch targets ok count={count}"),
    );
    Ok(targets)
}

#[tauri::command]
pub async fn proxy_fetch_snapshot(
    name: String,
    log_state: State<'_, AppLogState>,
) -> Result<Value, String> {
    let path = format!("/targets/{name}/snapshot");
    let snapshot = console_get(&path, &log_state.app_log).await?;
    let queue_len = snapshot
        .get("queue")
        .and_then(|value| value.as_array())
        .map(|value| value.len())
        .unwrap_or(0);
    let history_len = snapshot
        .get("history")
        .and_then(|value| value.as_array())
        .map(|value| value.len())
        .unwrap_or(0);
    let _ = append_log_line(
        &log_state.app_log,
        &format!("fetch snapshot ok target={name} queue_len={queue_len} history_len={history_len}"),
    );
    Ok(snapshot)
}

#[tauri::command]
pub async fn proxy_approve(
    name: String,
    id: String,
    log_state: State<'_, AppLogState>,
) -> Result<(), String> {
    let path = format!("/targets/{name}/approve");
    console_post(&path, json!({ "id": id }), &log_state.app_log).await
}

#[tauri::command]
pub async fn proxy_deny(
    name: String,
    id: String,
    log_state: State<'_, AppLogState>,
) -> Result<(), String> {
    let path = format!("/targets/{name}/deny");
    console_post(&path, json!({ "id": id }), &log_state.app_log).await
}

#[tauri::command]
pub async fn proxy_cancel(
    name: String,
    id: String,
    log_state: State<'_, AppLogState>,
) -> Result<(), String> {
    let path = format!("/targets/{name}/cancel");
    console_post(&path, json!({ "id": id }), &log_state.app_log).await
}

#[tauri::command]
pub async fn proxy_reload_remote_brokers(log_state: State<'_, AppLogState>) -> Result<(), String> {
    console_post_with_timeout(
        "/targets/reload-brokers",
        json!({}),
        &log_state.app_log,
        HTTP_RELOAD_TIMEOUT,
    )
    .await
}

#[tauri::command]
pub async fn start_console_stream(
    app: AppHandle,
    stream_state: State<'_, crate::state::ConsoleStreamState>,
    log_state: State<'_, AppLogState>,
) -> Result<(), String> {
    start_console_stream_service(app, stream_state, log_state).await
}
