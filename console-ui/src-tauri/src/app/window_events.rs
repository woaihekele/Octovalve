use tauri::{AppHandle, Manager, RunEvent, Window, WindowEvent};

use crate::services::console_sidecar::stop_console;
use crate::services::logging::append_log_line;
use crate::state::AppLogState;

fn log_window_event(window: &Window, message: &str) {
    let app_log = window.app_handle().state::<AppLogState>().app_log.clone();
    let maximized = window.is_maximized().unwrap_or(false);
    let minimized = window.is_minimized().unwrap_or(false);
    let decorated = window.is_decorated().unwrap_or(true);
    let _ = append_log_line(
        &app_log,
        &format!(
            "[window_event] {} label={} maximized={} minimized={} decorated={}",
            message,
            window.label(),
            maximized,
            minimized,
            decorated
        ),
    );
}

pub fn handle(window: &Window, event: &WindowEvent) {
    if window.label() != "main" {
        return;
    }

    match event {
        WindowEvent::Resized(size) => {
            log_window_event(window, &format!("resized {}x{}", size.width, size.height));
        }
        WindowEvent::Moved(pos) => {
            log_window_event(window, &format!("moved x={} y={}", pos.x, pos.y));
        }
        WindowEvent::ScaleFactorChanged {
            scale_factor,
            new_inner_size,
            ..
        } => {
            log_window_event(
                window,
                &format!(
                    "scale_factor_changed scale={} new_inner={}x{}",
                    scale_factor, new_inner_size.width, new_inner_size.height
                ),
            );
        }
        WindowEvent::Focused(focused) => {
            log_window_event(window, &format!("focused={focused}"));
        }
        WindowEvent::ThemeChanged(theme) => {
            log_window_event(window, &format!("theme_changed={theme:?}"));
        }
        _ => {}
    }

    #[cfg(target_os = "macos")]
    if let WindowEvent::CloseRequested { api, .. } = event {
        log_window_event(window, "close_requested hide_window");
        let _ = window.hide();
        api.prevent_close();
    }
    #[cfg(not(target_os = "macos"))]
    if let WindowEvent::CloseRequested { api, .. } = event {
        log_window_event(window, "close_requested stop_console_and_exit");
        api.prevent_close();
        let app_handle = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            stop_console(&app_handle);
            app_handle.exit(0);
        });
    }
}

pub fn handle_run(app_handle: &AppHandle, event: RunEvent) {
    // Run events are useful when diagnosing maximize/restore behavior around app lifecycle.
    if let Some(log_state) = app_handle.try_state::<AppLogState>() {
        let _ = append_log_line(
            &log_state.app_log,
            &format!("[window_run_event] {event:?}"),
        );
    }

    match event {
        RunEvent::ExitRequested { .. } => {
            let app_handle = app_handle.clone();
            tauri::async_runtime::spawn_blocking(move || {
                stop_console(&app_handle);
            });
        }
        RunEvent::Exit => {
            let app_handle = app_handle.clone();
            tauri::async_runtime::spawn_blocking(move || {
                stop_console(&app_handle);
            });
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen {
            has_visible_windows,
            ..
        } => {
            if !has_visible_windows {
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
        }
        _ => {}
    }
}
