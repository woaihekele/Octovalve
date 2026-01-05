use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use crate::types::ProfileRecord;

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

pub(crate) fn profile_dir_for(base: &Path, name: &str) -> PathBuf {
    base.join(name)
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
