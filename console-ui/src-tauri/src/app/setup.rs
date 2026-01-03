use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use tauri::Manager;

use crate::services::console_sidecar::start_console;
use crate::services::logging::append_log_line;
use crate::services::profiles::prepare_profiles;
use crate::state::{AppLogState, ProfilesState, ProxyConfigState};

pub fn init(app: &mut tauri::App) -> Result<(), String> {
    let app_handle = app.handle();
    let config_dir = app_handle
        .path()
        .app_config_dir()
        .map_err(|err| err.to_string())?;
    fs::create_dir_all(&config_dir).map_err(|err| err.to_string())?;
    let logs_dir = config_dir.join("logs");
    fs::create_dir_all(&logs_dir).map_err(|err| err.to_string())?;
    let app_log = logs_dir.join("app.log");
    app.manage(AppLogState {
        app_log: app_log.clone(),
    });
    let (profiles, proxy_status) = prepare_profiles(&app_handle, &app_log)?;
    app.manage(ProfilesState(Mutex::new(profiles)));
    app.manage(ProxyConfigState(Mutex::new(proxy_status.clone())));
    if proxy_status.present {
        let proxy_path = PathBuf::from(proxy_status.path.clone());
        if let Err(err) = start_console(&app_handle, &proxy_path, &app_log) {
            let _ = append_log_line(&app_log, &format!("console start failed: {err}"));
        }
    } else {
        let _ = append_log_line(
            &app_log,
            "proxy config missing; waiting for user to create local-proxy-config.toml",
        );
    }
    Ok(())
}
