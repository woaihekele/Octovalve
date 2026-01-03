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
}

pub fn handle_run(app_handle: &AppHandle, event: RunEvent) {
    match event {
        RunEvent::ExitRequested { .. } => {
            stop_console(app_handle);
        }
        RunEvent::Exit => {
            stop_console(app_handle);
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
