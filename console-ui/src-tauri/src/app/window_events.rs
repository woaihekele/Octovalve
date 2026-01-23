use tauri::{AppHandle, Manager, RunEvent, Window, WindowEvent};

use crate::services::console_sidecar::stop_console;

pub fn handle(window: &Window, event: &WindowEvent) {
    if window.label() != "main" {
        return;
    }
    #[cfg(target_os = "macos")]
    if let WindowEvent::CloseRequested { api, .. } = event {
        let _ = window.hide();
        api.prevent_close();
    }
    #[cfg(not(target_os = "macos"))]
    if let WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        let app_handle = window.app_handle();
        tauri::async_runtime::spawn_blocking(move || {
            stop_console(&app_handle);
            app_handle.exit(0);
        });
    }
}

pub fn handle_run(app_handle: &AppHandle, event: RunEvent) {
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
