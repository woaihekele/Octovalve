use std::collections::HashMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager};
use tauri_plugin_shell::{process::CommandEvent, ShellExt};

use crate::paths::sidecar_path;
use crate::services::config::{ensure_file, DEFAULT_BROKER_CONFIG};
use crate::services::logging::append_log_line;
use crate::services::profiles::{
    current_profile_entry, resolve_broker_config_path, sync_proxy_config_runtime_ports,
};
use crate::state::{AppLanguageState, ConsoleSidecar, ConsoleSidecarState, ProfilesState};

pub(crate) const DEFAULT_COMMAND_ADDR: &str = "127.0.0.1:19310";
const DEFAULT_APP_LANGUAGE: &str = "en-US";

fn format_command_output(line: &[u8]) -> String {
    String::from_utf8_lossy(line)
        .trim_end_matches(&['\r', '\n'][..])
        .to_string()
}

pub fn start_console(app: &AppHandle, proxy_config: &Path, app_log: &Path) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|err| err.to_string())?;
    fs::create_dir_all(&config_dir).map_err(|err| err.to_string())?;

    let profiles = app.state::<ProfilesState>().0.lock().unwrap().clone();
    let (remote_dir, remote_listen_addr, remote_control_addr, remote_audit_dir) =
        match current_profile_entry(&profiles) {
            Ok(profile) => {
                let listen_port = normalize_port(profile.remote_listen_port, 19307);
                let control_port = normalize_port(profile.remote_control_port, 19308);
                sync_proxy_config_runtime_ports(proxy_config, listen_port, control_port)?;
                let remote_dir = build_remote_dir(&profile.remote_dir_alias);
                let remote_audit_dir = format!("{remote_dir}/logs");
                (
                    remote_dir,
                    format!("127.0.0.1:{listen_port}"),
                    format!("127.0.0.1:{control_port}"),
                    remote_audit_dir,
                )
            }
            Err(err) => {
                let _ = append_log_line(
                    app_log,
                    &format!("failed to resolve profile runtime settings: {err}"),
                );
                (
                    "~/.octovalve".to_string(),
                    "127.0.0.1:19307".to_string(),
                    "127.0.0.1:19308".to_string(),
                    "~/.octovalve/logs".to_string(),
                )
            }
        };
    let resolved_broker =
        resolve_broker_config_path(app, proxy_config, &config_dir, Some(&profiles))?;
    let broker_config = resolved_broker.path;
    ensure_file(&broker_config, DEFAULT_BROKER_CONFIG)?;
    let logs_dir = config_dir.join("logs");
    fs::create_dir_all(&logs_dir).map_err(|err| err.to_string())?;
    let console_log = logs_dir.join("console.log");
    let _ = append_log_line(
        app_log,
        &format!(
            "console log path: {} broker_config={} source={}",
            console_log.display(),
            broker_config.display(),
            resolved_broker.source
        ),
    );

    let broker_bin_linux_x86_64 = resolve_linux_broker(
        app,
        "remote-broker-linux-x86_64",
        "remote-broker/linux-x86_64/remote-broker",
    );
    let broker_bin = if let Some(path) = broker_bin_linux_x86_64.clone() {
        path
    } else {
        sidecar_path("remote-broker")?
    };
    let mut envs = HashMap::new();
    envs.insert("PATH".to_string(), build_console_path());
    if let Some(locale) = resolve_default_locale(app) {
        envs.insert("OCTOVALVE_TERMINAL_LOCALE".to_string(), locale);
    }

    let mut console_args = vec![
        "--config".to_string(),
        proxy_config.to_string_lossy().to_string(),
        "--exec-mode".to_string(),
        "local".to_string(),
        "--command-listen-addr".to_string(),
        DEFAULT_COMMAND_ADDR.to_string(),
        "--remote-dir".to_string(),
        remote_dir,
        "--remote-listen-addr".to_string(),
        remote_listen_addr,
        "--remote-control-addr".to_string(),
        remote_control_addr,
        "--remote-audit-dir".to_string(),
        remote_audit_dir,
        "--broker-bin".to_string(),
        broker_bin.to_string_lossy().to_string(),
        "--broker-config".to_string(),
        broker_config.to_string_lossy().to_string(),
        "--log-to-stderr".to_string(),
    ];
    if let Some(path) = broker_bin_linux_x86_64 {
        console_args.push("--broker-bin-linux-x86_64".to_string());
        console_args.push(path.to_string_lossy().to_string());
    }

    let (mut rx, child) = app
        .shell()
        .sidecar("console")
        .map_err(|err| err.to_string())?
        .args(console_args)
        .envs(envs)
        .spawn()
        .map_err(|err| err.to_string())?;
    let _ = append_log_line(
        app_log,
        &format!("console sidecar started pid={}", child.pid()),
    );

    let exited = std::sync::Arc::new(AtomicBool::new(false));
    *app.state::<ConsoleSidecarState>().0.lock().unwrap() = Some(ConsoleSidecar {
        child,
        exited: exited.clone(),
    });

    let app_log = app_log.to_path_buf();
    tauri::async_runtime::spawn(async move {
        let mut file = match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&console_log)
        {
            Ok(file) => file,
            Err(err) => {
                let _ = append_log_line(&app_log, &format!("failed to open console log: {err}"));
                return;
            }
        };
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(line) => {
                    let _ = writeln!(file, "[stdout] {}", format_command_output(&line));
                }
                CommandEvent::Stderr(line) => {
                    let _ = writeln!(file, "[stderr] {}", format_command_output(&line));
                }
                CommandEvent::Error(err) => {
                    let _ = writeln!(file, "[error] {err}");
                }
                CommandEvent::Terminated(payload) => {
                    let _ = writeln!(file, "[exit] {:?}", payload.code);
                    exited.store(true, Ordering::SeqCst);
                    let _ = append_log_line(
                        &app_log,
                        &format!("console sidecar exited code={:?}", payload.code),
                    );
                }
                _ => {}
            }
        }
    });

    Ok(())
}

pub fn stop_console(app: &AppHandle) {
    let state = app.state::<ConsoleSidecarState>();
    let mut guard = state.0.lock().unwrap();
    let Some(sidecar) = guard.take() else {
        return;
    };
    let pid = sidecar.child.pid();
    let exited = sidecar.exited.clone();
    let log_path = app.state::<crate::state::AppLogState>().app_log.clone();
    let _ = append_log_line(&log_path, &format!("console stop requested pid={pid}"));
    #[cfg(unix)]
    unsafe {
        libc::kill(pid as i32, libc::SIGINT);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while !exited.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    if exited.load(Ordering::SeqCst) {
        let _ = append_log_line(&log_path, "console stopped gracefully");
        return;
    }
    let _ = append_log_line(&log_path, "console stop timed out; sending kill");
    let _ = sidecar.child.kill();
}

fn resolve_linux_broker(app: &AppHandle, filename: &str, resource_path: &str) -> Option<PathBuf> {
    if let Ok(value) = std::env::var("OCTOVALVE_BROKER_BIN_LINUX_X86_64") {
        let candidate = PathBuf::from(value);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let bundled = dir.join(filename);
    if bundled.exists() {
        return Some(bundled);
    }
    app.path()
        .resolve(resource_path, BaseDirectory::Resource)
        .ok()
}

pub fn build_console_path() -> String {
    let base = std::env::var("PATH").unwrap_or_default();
    if base.is_empty() {
        "/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin".to_string()
    } else {
        format!("/usr/local/bin:/opt/homebrew/bin:{base}")
    }
}

fn build_remote_dir(alias: &str) -> String {
    let trimmed = alias.trim();
    if trimmed.is_empty() {
        "~/.octovalve".to_string()
    } else {
        format!("~/.octovalve_{trimmed}")
    }
}

fn normalize_port(port: u16, fallback: u16) -> u16 {
    if port == 0 {
        fallback
    } else {
        port
    }
}

fn resolve_default_locale(app: &AppHandle) -> Option<String> {
    let language = app
        .state::<AppLanguageState>()
        .0
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| DEFAULT_APP_LANGUAGE.to_string());
    match language.as_str() {
        "zh-CN" => Some("zh_CN.utf8".to_string()),
        "en-US" => Some("en_US.utf8".to_string()),
        _ => None,
    }
}
