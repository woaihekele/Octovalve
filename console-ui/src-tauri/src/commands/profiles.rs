use tauri::State;

use crate::services::profiles;
use crate::state::{ProfilesState, ProxyConfigState};
use crate::types::{ProfilesStatus, ProxyConfigStatus};

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
pub fn create_profile(
    name: String,
    app: tauri::AppHandle,
    profiles_state: State<ProfilesState>,
) -> Result<(), String> {
    profiles::create_profile(name, app, profiles_state)
}

#[tauri::command]
pub fn delete_profile(
    name: String,
    app: tauri::AppHandle,
    profiles_state: State<ProfilesState>,
) -> Result<(), String> {
    profiles::delete_profile(name, app, profiles_state)
}

#[tauri::command]
pub fn select_profile(
    name: String,
    app: tauri::AppHandle,
    profiles_state: State<ProfilesState>,
    proxy_state: State<ProxyConfigState>,
) -> Result<(), String> {
    profiles::select_profile(name, app, profiles_state, proxy_state)
}

#[tauri::command]
pub fn read_profile_proxy_config(
    name: String,
    app: tauri::AppHandle,
    profiles_state: State<ProfilesState>,
) -> Result<crate::types::ConfigFilePayload, String> {
    profiles::read_profile_proxy_config(name, app, profiles_state)
}

#[tauri::command]
pub fn write_profile_proxy_config(
    name: String,
    content: String,
    app: tauri::AppHandle,
    profiles_state: State<ProfilesState>,
) -> Result<(), String> {
    profiles::write_profile_proxy_config(name, content, app, profiles_state)
}

#[tauri::command]
pub fn read_profile_broker_config(
    name: String,
    app: tauri::AppHandle,
    profiles_state: State<ProfilesState>,
) -> Result<crate::types::ConfigFilePayload, String> {
    profiles::read_profile_broker_config(name, app, profiles_state)
}

#[tauri::command]
pub fn write_profile_broker_config(
    name: String,
    content: String,
    app: tauri::AppHandle,
    profiles_state: State<ProfilesState>,
) -> Result<(), String> {
    profiles::write_profile_broker_config(name, content, app, profiles_state)
}
