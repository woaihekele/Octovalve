use std::path::PathBuf;

use tauri::{Manager, State};

use crate::services::config::{read_config_file, write_config_file, DEFAULT_PROXY_EXAMPLE};
use crate::state::ProxyConfigState;
use crate::types::{BrokerConfigEditor, ConfigFilePayload, ProxyConfigEditor};

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
pub fn parse_proxy_config_toml(content: String) -> Result<ProxyConfigEditor, String> {
    toml::from_str::<ProxyConfigEditor>(&content).map_err(|err| err.to_string())
}

#[tauri::command]
pub fn parse_broker_config_toml(content: String) -> Result<BrokerConfigEditor, String> {
    toml::from_str::<BrokerConfigEditor>(&content).map_err(|err| err.to_string())
}
