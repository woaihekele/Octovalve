use std::fs;
use std::sync::Mutex;

use tauri::Manager;

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
        let _ = append_log_line(
            &app_log,
            "proxy config ready; waiting for user to select a profile to start console",
        );
    } else {
        let _ = append_log_line(
            &app_log,
            "proxy config missing; waiting for user to create local-proxy-config.toml",
        );
    }
    Ok(())
}
