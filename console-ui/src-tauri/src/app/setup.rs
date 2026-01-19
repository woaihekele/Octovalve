use std::fs;
use std::path::Path;
use std::sync::Mutex;

use tauri::Manager;

use crate::services::logging::append_log_line;
use crate::services::profiles::{octovalve_dir, prepare_profiles};
use crate::state::{AppLogState, ProfilesState, ProxyConfigState};

const RUNTIME_AGENTS_TEMPLATE: &str = include_str!("../../assets/runtime/AGENTS.md");

#[cfg(target_os = "macos")]
fn disable_scroll_elasticity(app: &tauri::AppHandle) {
    use objc2::ClassType;
    use objc2_app_kit::NSScrollElasticity;
    use objc2_web_kit::WKWebView;
    use tauri::Manager;

    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let _ = window.with_webview(|webview| unsafe {
        let view: &WKWebView = &*webview.inner().cast();
        if let Some(scroll_view) = view.as_super().enclosingScrollView() {
            scroll_view.setVerticalScrollElasticity(NSScrollElasticity::None);
            scroll_view.setHorizontalScrollElasticity(NSScrollElasticity::None);
        }
    });
}

fn ensure_runtime_agents_file(app: &tauri::AppHandle, log_path: &Path) -> Result<(), String> {
    let workspace_dir = octovalve_dir(app)?.join("workspace");
    fs::create_dir_all(&workspace_dir).map_err(|err| err.to_string())?;
    let agents_path = workspace_dir.join("AGENTS.md");
    if agents_path.exists() {
        if agents_path.is_file() {
            return Ok(());
        }
        return Err(format!(
            "AGENTS.md exists but is not a file: {}",
            agents_path.display()
        ));
    }
    fs::write(&agents_path, RUNTIME_AGENTS_TEMPLATE).map_err(|err| err.to_string())?;
    let _ = append_log_line(
        log_path,
        &format!(
            "[setup] wrote runtime AGENTS.md to {}",
            agents_path.display()
        ),
    );
    Ok(())
}

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
    if let Err(err) = ensure_runtime_agents_file(&app_handle, &app_log) {
        let _ = append_log_line(
            &app_log,
            &format!("[setup] failed to write runtime AGENTS.md: {}", err),
        );
    }
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
    #[cfg(target_os = "macos")]
    disable_scroll_elasticity(&app_handle);
    Ok(())
}
