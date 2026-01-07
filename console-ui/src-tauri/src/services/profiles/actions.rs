use std::fs;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, State};

use crate::services::config::{
    ensure_file, read_config_file, write_config_file, DEFAULT_BROKER_CONFIG, DEFAULT_PROXY_EXAMPLE,
};
use crate::state::{ProfilesState, ProxyConfigState};
use crate::types::{ProfileRecord, ProfileRuntimeSettings};

use super::index::{
    current_profile_entry, profile_entry_by_name, validate_profile_name, write_profiles_file,
};
use super::lifecycle::sync_legacy_proxy_config;
use super::paths::{
    profile_broker_path, profile_dir_for, profile_proxy_path, profiles_dir, profiles_index_path,
};
use super::proxy_config::sync_proxy_config_runtime_ports;

fn remove_profile_files(profile: &ProfileRecord, profiles_base: &Path) {
    let proxy_path = Path::new(&profile.proxy_path);
    let broker_path = Path::new(&profile.broker_path);
    if let Some(dir) = proxy_path.parent() {
        if dir.starts_with(profiles_base) {
            let _ = fs::remove_dir_all(dir);
            return;
        }
    }
    if proxy_path.starts_with(profiles_base) {
        let _ = fs::remove_file(proxy_path);
    }
    if broker_path.starts_with(profiles_base) {
        let _ = fs::remove_file(broker_path);
    }
}

pub fn create_profile(
    name: String,
    app: AppHandle,
    profiles_state: State<ProfilesState>,
) -> Result<(), String> {
    validate_profile_name(&name)?;
    let profiles_base = profiles_dir(&app)?;
    let index_path = profiles_index_path(&app)?;

    let mut profiles = profiles_state.0.lock().unwrap().clone();
    if profiles.profiles.iter().any(|profile| profile.name == name) {
        return Err(format!("环境 {} 已存在", name));
    }
    let current = current_profile_entry(&profiles)?;
    let current_proxy_path = PathBuf::from(&current.proxy_path);
    let current_broker_path = PathBuf::from(&current.broker_path);
    let current_remote_dir_alias = current.remote_dir_alias.clone();
    let current_remote_listen_port = current.remote_listen_port;
    let current_remote_control_port = current.remote_control_port;

    let new_dir = profile_dir_for(&profiles_base, &name);
    fs::create_dir_all(&new_dir).map_err(|err| err.to_string())?;
    let new_proxy_path = new_dir.join("local-proxy-config.toml");
    let new_broker_path = new_dir.join("remote-broker-config.toml");

    if !new_proxy_path.exists() {
        let content = if current_proxy_path.exists() {
            fs::read_to_string(&current_proxy_path).map_err(|err| err.to_string())?
        } else {
            DEFAULT_PROXY_EXAMPLE.to_string()
        };
        write_config_file(&new_proxy_path, &content)?;
    }

    if !new_broker_path.exists() {
        let content = if current_broker_path.exists() {
            fs::read_to_string(&current_broker_path).map_err(|err| err.to_string())?
        } else {
            DEFAULT_BROKER_CONFIG.to_string()
        };
        write_config_file(&new_broker_path, &content)?;
    }

    let record = ProfileRecord {
        name: name.clone(),
        proxy_path: new_proxy_path.to_string_lossy().to_string(),
        broker_path: new_broker_path.to_string_lossy().to_string(),
        remote_dir_alias: current_remote_dir_alias,
        remote_listen_port: current_remote_listen_port,
        remote_control_port: current_remote_control_port,
    };
    profiles.profiles.push(record);
    write_profiles_file(&index_path, &profiles)?;
    *profiles_state.0.lock().unwrap() = profiles;
    Ok(())
}

pub fn delete_profile(
    name: String,
    app: AppHandle,
    profiles_state: State<ProfilesState>,
) -> Result<(), String> {
    let profiles_base = profiles_dir(&app)?;
    let index_path = profiles_index_path(&app)?;
    let mut profiles = profiles_state.0.lock().unwrap().clone();
    if profiles.current == name {
        return Err("不能删除当前环境，请先切换到其他环境".to_string());
    }
    if profiles.profiles.len() <= 1 {
        return Err("至少保留一个环境".to_string());
    }
    let idx = profiles
        .profiles
        .iter()
        .position(|profile| profile.name == name)
        .ok_or_else(|| format!("未找到环境 {}", name))?;
    let removed = profiles.profiles.remove(idx);
    write_profiles_file(&index_path, &profiles)?;
    *profiles_state.0.lock().unwrap() = profiles;
    remove_profile_files(&removed, &profiles_base);
    Ok(())
}

pub fn select_profile(
    name: String,
    app: AppHandle,
    profiles_state: State<ProfilesState>,
    proxy_state: State<ProxyConfigState>,
) -> Result<(), String> {
    let index_path = profiles_index_path(&app)?;
    let mut profiles = profiles_state.0.lock().unwrap().clone();
    let entry = profiles
        .profiles
        .iter()
        .find(|profile| profile.name == name)
        .cloned()
        .ok_or_else(|| format!("未找到环境 {}", name))?;
    if profiles.current == name {
        return Ok(());
    }
    profiles.current = name;
    write_profiles_file(&index_path, &profiles)?;
    *profiles_state.0.lock().unwrap() = profiles;

    let mut status = proxy_state.0.lock().unwrap();
    status.path = entry.proxy_path.clone();
    status.present = Path::new(&entry.proxy_path).exists();
    drop(status);
    let _ = sync_legacy_proxy_config(&app, Path::new(&entry.proxy_path));
    Ok(())
}

pub fn read_profile_proxy_config(
    name: String,
    app: AppHandle,
    profiles_state: State<ProfilesState>,
) -> Result<crate::types::ConfigFilePayload, String> {
    let profiles = profiles_state.0.lock().unwrap().clone();
    let profile = profile_entry_by_name(&profiles, &name)?;
    let path = profile_proxy_path(&app, &profile)?;
    read_config_file(&path, Some(DEFAULT_PROXY_EXAMPLE))
}

pub fn write_profile_proxy_config(
    name: String,
    content: String,
    app: AppHandle,
    profiles_state: State<ProfilesState>,
) -> Result<(), String> {
    let profiles = profiles_state.0.lock().unwrap().clone();
    let profile = profile_entry_by_name(&profiles, &name)?;
    let path = profile_proxy_path(&app, &profile)?;
    write_config_file(&path, &content)
}

pub fn read_profile_broker_config(
    name: String,
    app: AppHandle,
    profiles_state: State<ProfilesState>,
) -> Result<crate::types::ConfigFilePayload, String> {
    let profiles = profiles_state.0.lock().unwrap().clone();
    let profile = profile_entry_by_name(&profiles, &name)?;
    let path = profile_broker_path(&app, &profile)?;
    let existed = path.exists();
    ensure_file(&path, DEFAULT_BROKER_CONFIG)?;
    let mut payload = read_config_file(&path, Some(DEFAULT_BROKER_CONFIG))?;
    payload.exists = existed;
    Ok(payload)
}

pub fn write_profile_broker_config(
    name: String,
    content: String,
    app: AppHandle,
    profiles_state: State<ProfilesState>,
) -> Result<(), String> {
    let profiles = profiles_state.0.lock().unwrap().clone();
    let profile = profile_entry_by_name(&profiles, &name)?;
    let path = profile_broker_path(&app, &profile)?;
    write_config_file(&path, &content)
}

pub fn read_profile_runtime_settings(
    name: String,
    profiles_state: State<ProfilesState>,
) -> Result<ProfileRuntimeSettings, String> {
    let profiles = profiles_state.0.lock().unwrap().clone();
    let profile = profile_entry_by_name(&profiles, &name)?;
    Ok(ProfileRuntimeSettings {
        remote_dir_alias: profile.remote_dir_alias,
        remote_listen_port: profile.remote_listen_port,
        remote_control_port: profile.remote_control_port,
    })
}

pub fn write_profile_runtime_settings(
    name: String,
    settings: ProfileRuntimeSettings,
    app: AppHandle,
    profiles_state: State<ProfilesState>,
) -> Result<(), String> {
    let index_path = profiles_index_path(&app)?;
    let mut profiles = profiles_state.0.lock().unwrap().clone();
    let profile = profiles
        .profiles
        .iter_mut()
        .find(|profile| profile.name == name)
        .ok_or_else(|| format!("未找到环境 {}", name))?;
    let listen_port = if settings.remote_listen_port == 0 {
        19307
    } else {
        settings.remote_listen_port
    };
    let control_port = if settings.remote_control_port == 0 {
        19308
    } else {
        settings.remote_control_port
    };
    profile.remote_dir_alias = settings.remote_dir_alias.trim().to_string();
    profile.remote_listen_port = listen_port;
    profile.remote_control_port = control_port;
    let proxy_path = profile.proxy_path.clone();
    write_profiles_file(&index_path, &profiles)?;
    *profiles_state.0.lock().unwrap() = profiles;

    sync_proxy_config_runtime_ports(Path::new(&proxy_path), listen_port, control_port)?;
    Ok(())
}
