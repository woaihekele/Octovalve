use std::fs;
use std::path::Path;

use tauri::{AppHandle, Manager};

use crate::services::config::{
    ensure_file, write_config_file, DEFAULT_BROKER_CONFIG, DEFAULT_PROXY_EXAMPLE,
};
use crate::services::logging::append_log_line;
use crate::types::{
    ProfileRecord, ProfilesFile, ProxyConfigOverrides, ProxyConfigStatus, ResolvedBrokerConfig,
};

use super::index::{current_profile_entry, load_profiles_file, write_profiles_file};
use super::paths::{
    expand_tilde_path, legacy_proxy_config_path, octovalve_dir, profile_broker_path,
    profile_dir_for, profile_proxy_path, profiles_dir, profiles_index_path, resolve_config_path,
};

fn ensure_broker_file(profile: &ProfileRecord) -> Result<(), String> {
    ensure_file(Path::new(&profile.broker_path), DEFAULT_BROKER_CONFIG)?;
    Ok(())
}

pub(crate) fn sync_legacy_proxy_config(app: &AppHandle, proxy_path: &Path) -> Result<(), String> {
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
        remote_dir_alias: String::new(),
        remote_listen_port: 19307,
        remote_control_port: 19308,
    })
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

    let _ = append_log_line(
        log_path,
        &format!("profiles loaded; current={}", profiles.current),
    );
    for profile in &profiles.profiles {
        let proxy_resolved = profile_proxy_path(app, profile)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|err| format!("error: {err}"));
        let broker_resolved = profile_broker_path(app, profile)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|err| format!("error: {err}"));
        let _ = append_log_line(
            log_path,
            &format!(
                "profile {} proxy_path={} (resolved={}) broker_path={} (resolved={})",
                profile.name, profile.proxy_path, proxy_resolved, profile.broker_path, broker_resolved
            ),
        );
    }

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
