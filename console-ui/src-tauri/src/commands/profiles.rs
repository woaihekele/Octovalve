use tauri::{Manager, State};

use crate::services::profiles;
use crate::state::{ProfilesState, ProxyConfigState};
use crate::types::{ProfileRuntimeSettings, ProfilesStatus, ProxyConfigStatus};

#[tauri::command]
pub fn get_proxy_config_status(state: State<ProxyConfigState>) -> ProxyConfigStatus {
    state.0.lock().unwrap().clone()
}

#[tauri::command]
pub fn list_profiles(state: State<ProfilesState>) -> ProfilesStatus {
    let data = state.0.lock().unwrap();
    profiles::profiles_status(&data)
}

#[tauri::command]
pub async fn create_profile(name: String, app: tauri::AppHandle) -> Result<(), String> {
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state_handle = app_handle.clone();
        let profiles_state = state_handle.state::<ProfilesState>();
        profiles::create_profile(name, app_handle, profiles_state)
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn delete_profile(name: String, app: tauri::AppHandle) -> Result<(), String> {
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state_handle = app_handle.clone();
        let profiles_state = state_handle.state::<ProfilesState>();
        profiles::delete_profile(name, app_handle, profiles_state)
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn select_profile(name: String, app: tauri::AppHandle) -> Result<(), String> {
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state_handle = app_handle.clone();
        let profiles_state = state_handle.state::<ProfilesState>();
        let proxy_state = state_handle.state::<ProxyConfigState>();
        profiles::select_profile(name, app_handle, profiles_state, proxy_state)
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn read_profile_proxy_config(
    name: String,
    app: tauri::AppHandle,
) -> Result<crate::types::ConfigFilePayload, String> {
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state_handle = app_handle.clone();
        let profiles_state = state_handle.state::<ProfilesState>();
        profiles::read_profile_proxy_config(name, app_handle, profiles_state)
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn write_profile_proxy_config(
    name: String,
    content: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state_handle = app_handle.clone();
        let profiles_state = state_handle.state::<ProfilesState>();
        profiles::write_profile_proxy_config(name, content, app_handle, profiles_state)
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn read_profile_broker_config(
    name: String,
    app: tauri::AppHandle,
) -> Result<crate::types::ConfigFilePayload, String> {
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state_handle = app_handle.clone();
        let profiles_state = state_handle.state::<ProfilesState>();
        profiles::read_profile_broker_config(name, app_handle, profiles_state)
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn write_profile_broker_config(
    name: String,
    content: String,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state_handle = app_handle.clone();
        let profiles_state = state_handle.state::<ProfilesState>();
        profiles::write_profile_broker_config(name, content, app_handle, profiles_state)
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn read_profile_runtime_settings(
    name: String,
    app: tauri::AppHandle,
) -> Result<ProfileRuntimeSettings, String> {
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state_handle = app_handle.clone();
        let profiles_state = state_handle.state::<ProfilesState>();
        profiles::read_profile_runtime_settings(name, profiles_state)
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
pub async fn write_profile_runtime_settings(
    name: String,
    settings: ProfileRuntimeSettings,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let app_handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state_handle = app_handle.clone();
        let profiles_state = state_handle.state::<ProfilesState>();
        profiles::write_profile_runtime_settings(name, settings, app_handle, profiles_state)
    })
    .await
    .map_err(|err| err.to_string())?
}
