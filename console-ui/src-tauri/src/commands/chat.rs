use std::fs;
use std::path::PathBuf;

use tauri::{AppHandle, State};

use crate::services::chat_diagnostics;
use crate::state::{AppLogState, ProfilesState, ProxyConfigState};
use crate::types::{ChatProviderCheckInput, ChatProviderCheckResult};

const MAX_IMPORT_BYTES: u64 = 10 * 1024 * 1024;

#[tauri::command]
pub async fn read_text_file(path: String) -> Result<String, String> {
    let path = PathBuf::from(path);
    let metadata = fs::metadata(&path).map_err(|err| err.to_string())?;
    if !metadata.is_file() {
        return Err("路径不是文件".to_string());
    }
    if metadata.len() > MAX_IMPORT_BYTES {
        return Err(format!("文件过大（{} bytes）", metadata.len()));
    }
    fs::read_to_string(&path).map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn run_chat_provider_checks(
    app: AppHandle,
    input: ChatProviderCheckInput,
    proxy_state: State<'_, ProxyConfigState>,
    profiles_state: State<'_, ProfilesState>,
    log_state: State<'_, AppLogState>,
) -> Result<ChatProviderCheckResult, String> {
    let proxy_status = proxy_state.0.lock().unwrap().clone();
    let profiles = profiles_state.0.lock().unwrap().clone();
    Ok(
        chat_diagnostics::run_chat_provider_checks(
            &app,
            &log_state.app_log,
            &proxy_status,
            &profiles,
            input,
        )
        .await,
    )
}
