use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use tauri::{AppHandle, Manager};

use crate::services::console_sidecar::restart_console_sidecar;
use crate::services::logging::append_log_line;
use crate::state::{AppLogState, ConsoleSidecarState, ProxyConfigState};

fn sidecar_path(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    #[cfg(windows)]
    {
        Some(dir.join(format!("{name}.exe")))
    }
    #[cfg(not(windows))]
    {
        Some(dir.join(name))
    }
}

fn file_stamp(path: &PathBuf) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    Some((modified, meta.len()))
}

/// Dev-only: auto-restart the console sidecar when its binary is replaced.
///
/// `tauri_plugin_shell::ShellExt::sidecar()` resolves the binary path as:
/// `parent_dir(current_exe) + <name>(.exe)`.
/// If we sync rebuilt binaries into that directory, a restart loads the new version.
#[cfg(debug_assertions)]
pub fn start_dev_sidecar_autorestart(app: AppHandle) {
    let app_log = app.state::<AppLogState>().app_log.clone();

    let Some(console_path) = sidecar_path("octovalve-console") else {
        let _ = append_log_line(&app_log, "[dev] sidecar autorestart disabled: current_exe has no parent");
        return;
    };

    let _ = append_log_line(
        &app_log,
        &format!(
            "[dev] sidecar autorestart enabled; watching {}",
            console_path.display()
        ),
    );

    std::thread::spawn(move || {
        // Initialize stamp so we don't trigger on boot.
        let mut last = file_stamp(&console_path);
        let mut last_restart = Instant::now() - Duration::from_secs(60);
        let boot_grace_deadline = Instant::now() + Duration::from_secs(2);

        loop {
            std::thread::sleep(Duration::from_millis(350));
            if Instant::now() < boot_grace_deadline {
                continue;
            }

            let now = file_stamp(&console_path);
            if now.is_none() || now == last {
                continue;
            }

            // Debounce copy/replace operations: wait for a stable stamp before restarting.
            std::thread::sleep(Duration::from_millis(300));
            let stable = file_stamp(&console_path);
            if stable.is_none() || stable != now {
                last = stable;
                continue;
            }

            // Throttle restarts to avoid a restart storm.
            if last_restart.elapsed() < Duration::from_secs(2) {
                last = stable;
                continue;
            }

            // Only restart if the console is already running; otherwise just update the stamp.
            let running = app
                .state::<ConsoleSidecarState>()
                .0
                .lock()
                .ok()
                .and_then(|guard| guard.as_ref().map(|_| ()))
                .is_some();

            if !running {
                last = stable;
                continue;
            }

            let proxy = app.state::<ProxyConfigState>().0.lock().unwrap().clone();
            if !proxy.present {
                last = stable;
                continue;
            }

            let _ = append_log_line(
                &app_log,
                &format!(
                    "[dev] detected sidecar binary change; restarting console (path={})",
                    console_path.display()
                ),
            );

            let res = restart_console_sidecar(&app, std::path::Path::new(&proxy.path), &app_log);
            match res {
                Ok(_) => {
                    last_restart = Instant::now();
                    let _ = append_log_line(&app_log, "[dev] console restarted due to sidecar update");
                }
                Err(err) => {
                    let _ = append_log_line(
                        &app_log,
                        &format!("[dev] console restart failed after sidecar update: {err}"),
                    );
                }
            }

            last = stable;
        }
    });
}

#[cfg(not(debug_assertions))]
pub fn start_dev_sidecar_autorestart(_app: AppHandle) {}
