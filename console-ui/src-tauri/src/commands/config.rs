use std::path::PathBuf;

use tauri::{Manager, State};

use crate::services::config::{read_config_file, write_config_file, DEFAULT_BROKER_CONFIG, DEFAULT_PROXY_EXAMPLE};
use crate::services::profiles::resolve_broker_config_path;
use crate::state::{ProfilesState, ProxyConfigState};
use crate::types::ConfigFilePayload;

#[tauri::command]
pub async fn read_proxy_config(
    state: State<'_, ProxyConfigState>,
) -> Result<ConfigFilePayload, String> {
    let path = {
        let status = state.0.lock().unwrap();
        PathBuf::from(status.path.clone())
    };
    tauri::async_runtime::spawn_blocking(move || {
        read_config_file(&path, Some(DEFAULT_PROXY_EXAMPLE))
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn write_proxy_config(
    content: String,
    _app: tauri::AppHandle,
    state: State<'_, ProxyConfigState>,
) -> Result<(), String> {
    let path = {
        let status = state.0.lock().unwrap();
        PathBuf::from(status.path.clone())
    };
    tauri::async_runtime::spawn_blocking(move || write_config_file(&path, &content))
        .await
        .map_err(|err| err.to_string())??;
    let mut status = state.0.lock().unwrap();
    status.present = true;
    Ok(())
}

#[tauri::command]
pub async fn read_broker_config(
    app: tauri::AppHandle,
    state: State<'_, ProxyConfigState>,
    profiles_state: State<'_, ProfilesState>,
) -> Result<ConfigFilePayload, String> {
    let config_dir = app.path().app_config_dir().map_err(|err| err.to_string())?;
    let proxy_path = {
        let status = state.0.lock().unwrap();
        PathBuf::from(status.path.clone())
    };
    let profiles = profiles_state.0.lock().unwrap().clone();
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let resolved =
            resolve_broker_config_path(&app_handle, &proxy_path, &config_dir, Some(&profiles))?;
        let existed = resolved.path.exists();
        crate::services::config::ensure_file(&resolved.path, DEFAULT_BROKER_CONFIG)?;
        let mut payload = read_config_file(&resolved.path, Some(DEFAULT_BROKER_CONFIG))?;
        payload.exists = existed;
        Ok(payload)
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn write_broker_config(
    content: String,
    app: tauri::AppHandle,
    state: State<'_, ProxyConfigState>,
    profiles_state: State<'_, ProfilesState>,
) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|err| err.to_string())?;
    let proxy_path = {
        let status = state.0.lock().unwrap();
        PathBuf::from(status.path.clone())
    };
    let profiles = profiles_state.0.lock().unwrap().clone();
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let resolved =
            resolve_broker_config_path(&app_handle, &proxy_path, &config_dir, Some(&profiles))?;
        write_config_file(&resolved.path, &content)
    })
    .await
    .map_err(|err| err.to_string())?
}
