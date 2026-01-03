use std::fs;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager, State};

use crate::services::config::{
    ensure_file, read_config_file, write_config_file, DEFAULT_BROKER_CONFIG,
    DEFAULT_PROXY_EXAMPLE,
};
use crate::services::logging::append_log_line;
use crate::state::{ProfilesState, ProxyConfigState};
use crate::types::{
    ProfileRecord, ProfileSummary, ProfilesFile, ProfilesStatus, ProxyConfigOverrides,
    ProxyConfigStatus, ResolvedBrokerConfig,
};

pub fn octovalve_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let home = app.path().home_dir().map_err(|err| err.to_string())?;
    Ok(home.join(".octovalve"))
}

pub fn profiles_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(octovalve_dir(app)?.join("profiles"))
}

pub fn profiles_index_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(profiles_dir(app)?.join("profiles.toml"))
}

pub fn legacy_proxy_config_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(octovalve_dir(app)?.join("local-proxy-config.toml"))
}

fn profile_dir_for(base: &Path, name: &str) -> PathBuf {
    base.join(name)
}

pub fn validate_profile_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("环境名称不能为空".to_string());
    }
    if trimmed.len() > 48 {
        return Err("环境名称最长支持 48 个字符".to_string());
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err("环境名称仅支持字母、数字、- 或 _".to_string());
    }
    Ok(())
}

pub fn profiles_status(data: &ProfilesFile) -> ProfilesStatus {
    ProfilesStatus {
        current: data.current.clone(),
        profiles: data
            .profiles
            .iter()
            .map(|profile| ProfileSummary {
                name: profile.name.clone(),
            })
            .collect(),
    }
}

pub fn current_profile_entry(data: &ProfilesFile) -> Result<ProfileRecord, String> {
    data.profiles
        .iter()
        .find(|profile| profile.name == data.current)
        .cloned()
        .ok_or_else(|| "current profile missing in profiles list".to_string())
}

pub fn profile_entry_by_name(data: &ProfilesFile, name: &str) -> Result<ProfileRecord, String> {
    data.profiles
        .iter()
        .find(|profile| profile.name == name)
        .cloned()
        .ok_or_else(|| format!("未找到环境 {}", name))
}

fn load_profiles_file(path: &Path) -> Result<ProfilesFile, String> {
    let raw = fs::read_to_string(path).map_err(|err| err.to_string())?;
    let parsed: ProfilesFile = toml::from_str(&raw).map_err(|err| err.to_string())?;
    if parsed.profiles.is_empty() {
        return Err("profiles.toml 必须至少包含一个环境".to_string());
    }
    let mut seen = std::collections::HashSet::new();
    for profile in &parsed.profiles {
        if profile.name.trim().is_empty() {
            return Err("环境名称不能为空".to_string());
        }
        if !seen.insert(profile.name.clone()) {
            return Err(format!("重复的环境名称：{}", profile.name));
        }
    }
    Ok(parsed)
}

fn write_profiles_file(path: &Path, data: &ProfilesFile) -> Result<(), String> {
    let content = toml::to_string_pretty(data).map_err(|err| err.to_string())?;
    write_config_file(path, &content)
}

fn ensure_broker_file(profile: &ProfileRecord) -> Result<(), String> {
    ensure_file(Path::new(&profile.broker_path), DEFAULT_BROKER_CONFIG)?;
    Ok(())
}

fn sync_legacy_proxy_config(app: &AppHandle, proxy_path: &Path) -> Result<(), String> {
    let legacy_path = legacy_proxy_config_path(app)?;
    if !proxy_path.exists() {
        return Ok(());
    }
    let content = fs::read_to_string(proxy_path).unwrap_or_default();
    write_config_file(&legacy_path, &content)
}

fn create_default_profile(
    app: &AppHandle,
    profiles_base: &Path,
    legacy_proxy: &Path,
    app_config_dir: &Path,
) -> Result<ProfileRecord, String> {
    let name = "default";
    let profile_dir = profile_dir_for(profiles_base, name);
    fs::create_dir_all(&profile_dir).map_err(|err| err.to_string())?;
    let proxy_path = profile_dir.join("local-proxy-config.toml");
    let broker_path = profile_dir.join("remote-broker-config.toml");

    if !proxy_path.exists() {
        let content = if legacy_proxy.exists() {
            fs::read_to_string(legacy_proxy).map_err(|err| err.to_string())?
        } else {
            DEFAULT_PROXY_EXAMPLE.to_string()
        };
        write_config_file(&proxy_path, &content)?;
    }

    if !broker_path.exists() {
        let broker_source = if legacy_proxy.exists() {
            resolve_broker_config_path(app, legacy_proxy, app_config_dir, None)?.path
        } else {
            app_config_dir.join("remote-broker-config.toml")
        };
        let broker_content = if broker_source.exists() {
            fs::read_to_string(&broker_source).map_err(|err| err.to_string())?
        } else {
            DEFAULT_BROKER_CONFIG.to_string()
        };
        write_config_file(&broker_path, &broker_content)?;
    }

    Ok(ProfileRecord {
        name: name.to_string(),
        proxy_path: proxy_path.to_string_lossy().to_string(),
        broker_path: broker_path.to_string_lossy().to_string(),
    })
}

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

pub fn prepare_profiles(
    app: &AppHandle,
    log_path: &Path,
) -> Result<(ProfilesFile, ProxyConfigStatus), String> {
    let config_dir = octovalve_dir(app)?;
    fs::create_dir_all(&config_dir).map_err(|err| err.to_string())?;
    let profiles_base = profiles_dir(app)?;
    fs::create_dir_all(&profiles_base).map_err(|err| err.to_string())?;
    let index_path = profiles_index_path(app)?;

    let example_path = config_dir.join("local-proxy-config.toml.example");
    ensure_file(&example_path, DEFAULT_PROXY_EXAMPLE)?;

    let mut profiles = if index_path.exists() {
        load_profiles_file(&index_path)?
    } else {
        let legacy_path = legacy_proxy_config_path(app)?;
        let app_config_dir = app.path().app_config_dir().map_err(|err| err.to_string())?;
        let default_profile =
            create_default_profile(app, &profiles_base, &legacy_path, &app_config_dir)?;
        let profiles = ProfilesFile {
            current: default_profile.name.clone(),
            profiles: vec![default_profile],
        };
        write_profiles_file(&index_path, &profiles)?;
        profiles
    };

    if !profiles
        .profiles
        .iter()
        .any(|profile| profile.name == profiles.current)
    {
        profiles.current = profiles
            .profiles
            .first()
            .map(|profile| profile.name.clone())
            .unwrap_or_else(|| "default".to_string());
        let _ = write_profiles_file(&index_path, &profiles);
    }

    for profile in &profiles.profiles {
        let _ = ensure_broker_file(profile);
    }

    let current = current_profile_entry(&profiles)?;
    let present = Path::new(&current.proxy_path).exists();
    let status = ProxyConfigStatus {
        present,
        path: current.proxy_path.clone(),
        example_path: example_path.to_string_lossy().to_string(),
    };
    if !present {
        let _ = append_log_line(
            log_path,
            &format!("proxy config missing at {}", status.path),
        );
        let _ = append_log_line(
            log_path,
            &format!("proxy config example at {}", status.example_path),
        );
    }
    let _ = sync_legacy_proxy_config(app, Path::new(&current.proxy_path));
    Ok((profiles, status))
}

pub fn resolve_broker_config_path(
    app: &AppHandle,
    proxy_config: &Path,
    app_config_dir: &Path,
    profiles: Option<&ProfilesFile>,
) -> Result<ResolvedBrokerConfig, String> {
    let default_path = app_config_dir.join("remote-broker-config.toml");
    if let Some(profiles) = profiles {
        if let Ok(current) = current_profile_entry(profiles) {
            let raw = current.broker_path.clone();
            if raw.trim().is_empty() {
                // fall back to proxy config resolution
            } else {
                let mut path = expand_tilde_path(app, &raw)?;
                if !path.is_absolute() {
                    let base = profiles_index_path(app)?;
                    path = resolve_config_path(app, &base, &raw)?;
                }
                return Ok(ResolvedBrokerConfig {
                    path,
                    source: "profile".to_string(),
                });
            }
        }
    }
    if !proxy_config.exists() {
        return Ok(ResolvedBrokerConfig {
            path: default_path,
            source: "default".to_string(),
        });
    }
    let raw = fs::read_to_string(proxy_config).map_err(|err| err.to_string())?;
    let parsed: ProxyConfigOverrides = toml::from_str(&raw).map_err(|err| err.to_string())?;
    if let Some(path) = parsed.broker_config_path {
        let resolved = resolve_config_path(app, proxy_config, &path)?;
        return Ok(ResolvedBrokerConfig {
            path: resolved,
            source: "config".to_string(),
        });
    }
    Ok(ResolvedBrokerConfig {
        path: default_path,
        source: "default".to_string(),
    })
}

pub fn resolve_config_path(app: &AppHandle, base: &Path, value: &str) -> Result<PathBuf, String> {
    let expanded = expand_tilde_path(app, value)?;
    if expanded.is_absolute() {
        return Ok(expanded);
    }
    let base_dir = base
        .parent()
        .ok_or_else(|| "failed to resolve config dir".to_string())?;
    Ok(base_dir.join(expanded))
}

pub fn resolve_profile_path(app: &AppHandle, value: &str) -> Result<PathBuf, String> {
    let expanded = expand_tilde_path(app, value)?;
    if expanded.is_absolute() {
        return Ok(expanded);
    }
    let base = profiles_index_path(app)?;
    resolve_config_path(app, &base, value)
}

pub fn profile_proxy_path(app: &AppHandle, profile: &ProfileRecord) -> Result<PathBuf, String> {
    resolve_profile_path(app, &profile.proxy_path)
}

pub fn profile_broker_path(app: &AppHandle, profile: &ProfileRecord) -> Result<PathBuf, String> {
    resolve_profile_path(app, &profile.broker_path)
}

pub fn expand_tilde_path(app: &AppHandle, value: &str) -> Result<PathBuf, String> {
    if value == "~" {
        return app.path().home_dir().map_err(|err| err.to_string());
    }
    if let Some(rest) = value.strip_prefix("~/") {
        let home = app.path().home_dir().map_err(|err| err.to_string())?;
        return Ok(home.join(rest));
    }
    Ok(PathBuf::from(value))
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
