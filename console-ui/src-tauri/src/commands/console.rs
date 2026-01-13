use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

use crate::services::console_http::{console_get, console_post, console_post_json};
use crate::services::console_sidecar::{start_console, stop_console};
use crate::services::console_ws::start_console_stream as start_console_stream_service;
use crate::services::logging::append_log_line;
use crate::services::startup_check;
use crate::state::{AppLanguageState, AppLogState, ProfilesState, ProxyConfigState};
use crate::types::{LogChunk, StartupCheckResult};
use urlencoding::encode;

fn console_log_path(app: &AppHandle) -> Result<PathBuf, String> {
    let config_dir = app.path().app_config_dir().map_err(|err| err.to_string())?;
    Ok(config_dir.join("logs").join("console.log"))
}

fn read_log_blocking(offset: u64, max_bytes: u64, path: &Path) -> Result<LogChunk, String> {
    if !path.exists() {
        return Ok(LogChunk {
            content: String::new(),
            next_offset: 0,
        });
    }
    let mut file = OpenOptions::new()
        .read(true)
        .open(path)
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

fn read_console_log_blocking(
    offset: u64,
    max_bytes: u64,
    app: &AppHandle,
) -> Result<LogChunk, String> {
    let path = console_log_path(app)?;
    read_log_blocking(offset, max_bytes, &path)
}

#[tauri::command]
pub async fn read_console_log(
    offset: u64,
    max_bytes: u64,
    app: AppHandle,
) -> Result<LogChunk, String> {
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        read_console_log_blocking(offset, max_bytes, &app_handle)
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn read_app_log(
    offset: u64,
    max_bytes: u64,
    log_state: State<'_, AppLogState>,
) -> Result<LogChunk, String> {
    let log_path = log_state.app_log.clone();
    tauri::async_runtime::spawn_blocking(move || read_log_blocking(offset, max_bytes, &log_path))
        .await
        .map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn restart_console(
    app: AppHandle,
    state: State<'_, ProxyConfigState>,
    log_state: State<'_, AppLogState>,
) -> Result<(), String> {
    let status = state.0.lock().unwrap().clone();
    let app_log = log_state.app_log.clone();
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let console_log = console_log_path(&app_handle)?;
        let _ = append_log_line(&console_log, "console restart requested");
        stop_console(&app_handle);
        if !status.present {
            return Err("proxy config missing".to_string());
        }
        match start_console(&app_handle, Path::new(&status.path), &app_log) {
            Ok(_) => {
                let _ = append_log_line(&console_log, "console restart started");
                Ok(())
            }
            Err(err) => {
                let _ = append_log_line(&console_log, &format!("console restart failed: {err}"));
                Err(err)
            }
        }
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn validate_startup_config(
    app: AppHandle,
    proxy_state: State<'_, ProxyConfigState>,
    profiles_state: State<'_, ProfilesState>,
) -> Result<StartupCheckResult, String> {
    let status = proxy_state.0.lock().unwrap().clone();
    let profiles = profiles_state.0.lock().unwrap().clone();
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        startup_check::validate_startup_config(&app_handle, &status, &profiles)
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn log_ui_event(message: String, state: State<'_, AppLogState>) -> Result<(), String> {
    let log_path = state.app_log.clone();
    tauri::async_runtime::spawn_blocking(move || append_log_line(&log_path, &message))
        .await
        .map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn set_app_language(
    language: String,
    state: State<'_, AppLanguageState>,
) -> Result<(), String> {
    let normalized = match language.as_str() {
        "zh-CN" | "en-US" => language,
        _ => "en-US".to_string(),
    };
    *state.0.lock().unwrap() = Some(normalized);
    Ok(())
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
pub async fn proxy_list_target_dirs(
    name: String,
    path: String,
    log_state: State<'_, AppLogState>,
) -> Result<Value, String> {
    let encoded = encode(&path);
    let path = format!("/targets/{name}/dirs?path={encoded}");
    console_get(&path, &log_state.app_log).await
}

#[tauri::command]
pub async fn proxy_start_upload(
    name: String,
    local_path: String,
    remote_path: String,
    log_state: State<'_, AppLogState>,
) -> Result<Value, String> {
    let path = format!("/targets/{name}/upload");
    console_post_json(
        &path,
        json!({
            "local_path": local_path,
            "remote_path": remote_path
        }),
        &log_state.app_log,
    )
    .await
}

#[tauri::command]
pub async fn proxy_upload_status(
    id: String,
    log_state: State<'_, AppLogState>,
) -> Result<Value, String> {
    let path = format!("/uploads/{id}");
    console_get(&path, &log_state.app_log).await
}

#[tauri::command]
pub async fn start_console_stream(
    app: AppHandle,
    stream_state: State<'_, crate::state::ConsoleStreamState>,
    log_state: State<'_, AppLogState>,
) -> Result<(), String> {
    start_console_stream_service(app, stream_state, log_state).await
}
