#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};
use tauri_plugin_shell::{
    process::{CommandChild, CommandEvent},
    ShellExt,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

const DEFAULT_PROXY_EXAMPLE: &str = include_str!("../resources/local-proxy-config.toml.example");
const DEFAULT_BROKER_CONFIG: &str = include_str!("../../../config/config.toml");

struct ConsoleSidecar {
    child: CommandChild,
    exited: std::sync::Arc<AtomicBool>,
}

struct ConsoleSidecarState(Mutex<Option<ConsoleSidecar>>);
struct ConsoleStreamState(Mutex<bool>);
struct ProxyConfigState(Mutex<ProxyConfigStatus>);
struct ProfilesState(Mutex<ProfilesFile>);
struct TerminalSessions(Mutex<HashMap<String, TerminalSession>>);
struct AppLogState {
    app_log: PathBuf,
}

#[derive(Clone, serde::Serialize)]
struct ProxyConfigStatus {
    present: bool,
    path: String,
    example_path: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct ProfileRecord {
    name: String,
    proxy_path: String,
    broker_path: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct ProfilesFile {
    current: String,
    profiles: Vec<ProfileRecord>,
}

#[derive(Clone, Serialize)]
struct ProfileSummary {
    name: String,
}

#[derive(Clone, Serialize)]
struct ProfilesStatus {
    current: String,
    profiles: Vec<ProfileSummary>,
}

#[derive(Clone, serde::Serialize)]
struct ConfigFilePayload {
    path: String,
    exists: bool,
    content: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LogChunk {
    content: String,
    next_offset: u64,
}

#[derive(Deserialize)]
struct ProxyConfigOverrides {
    broker_config_path: Option<String>,
}

struct ResolvedBrokerConfig {
    path: PathBuf,
    source: String,
}

fn console_log_path(app: &AppHandle) -> Result<PathBuf, String> {
    let config_dir = app.path().app_config_dir().map_err(|err| err.to_string())?;
    Ok(config_dir.join("logs").join("console.log"))
}

const CONSOLE_HTTP_HOST: &str = "127.0.0.1:19309";
const CONSOLE_WS_URL: &str = "ws://127.0.0.1:19309/ws";
const DEFAULT_TERM: &str = "xterm-256color";
const WS_RECONNECT_DELAY: Duration = Duration::from_secs(3);
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const HTTP_IO_TIMEOUT: Duration = Duration::from_secs(5);
const HTTP_RELOAD_TIMEOUT: Duration = Duration::from_secs(120);
static HTTP_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

struct TerminalSession {
    tx: mpsc::UnboundedSender<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TerminalMessage {
    Ready { cols: u16, rows: u16, term: String },
    Output { data: String },
    Exit { code: Option<i32> },
    Error { message: String },
}

#[derive(Debug, Deserialize)]
struct AiRiskRequest {
    base_url: String,
    chat_path: String,
    model: String,
    api_key: String,
    prompt: String,
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct AiRiskModelResponse {
    risk: String,
    reason: Option<String>,
    #[serde(default)]
    key_points: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct AiRiskResponse {
    risk: String,
    reason: String,
    key_points: Vec<String>,
}

fn main() {
    tauri::Builder::default()
        .manage(ConsoleSidecarState(Mutex::new(None)))
        .manage(ConsoleStreamState(Mutex::new(false)))
        .manage(TerminalSessions(Mutex::new(HashMap::new())))
        .invoke_handler(tauri::generate_handler![
            list_profiles,
            create_profile,
            delete_profile,
            select_profile,
            read_profile_proxy_config,
            write_profile_proxy_config,
            read_profile_broker_config,
            write_profile_broker_config,
            get_proxy_config_status,
            read_proxy_config,
            write_proxy_config,
            read_broker_config,
            write_broker_config,
            restart_console,
            log_ui_event,
            proxy_fetch_targets,
            proxy_fetch_snapshot,
            proxy_approve,
            proxy_deny,
            proxy_reload_remote_brokers,
            read_console_log,
            ai_risk_assess,
            start_console_stream,
            terminal_open,
            terminal_input,
            terminal_resize,
            terminal_close
        ])
        .setup(|app| {
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
                    eprintln!("failed to start console sidecar: {err}");
                    let _ = append_log_line(&app_log, &format!("console start failed: {err}"));
                }
            } else {
                let _ = append_log_line(
                    &app_log,
                    "proxy config missing; waiting for user to create local-proxy-config.toml",
                );
            }
            Ok(())
        })
        .plugin(tauri_plugin_shell::init())
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            #[cfg(target_os = "macos")]
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app_handle, event| match event {
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
        });
}

fn format_command_output(line: &[u8]) -> String {
    String::from_utf8_lossy(line)
        .trim_end_matches(&['\r', '\n'][..])
        .to_string()
}

fn start_console(app: &AppHandle, proxy_config: &Path, app_log: &Path) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|err| err.to_string())?;
    fs::create_dir_all(&config_dir).map_err(|err| err.to_string())?;

    let profiles = app.state::<ProfilesState>().0.lock().unwrap().clone();
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

    let mut console_args = vec![
        "--config".to_string(),
        proxy_config.to_string_lossy().to_string(),
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
                eprintln!("failed to open console log: {err}");
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

fn stop_console(app: &AppHandle) {
    let state = app.state::<ConsoleSidecarState>();
    let mut guard = state.0.lock().unwrap();
    let Some(sidecar) = guard.take() else {
        return;
    };
    let pid = sidecar.child.pid();
    let exited = sidecar.exited.clone();
    let log_path = app.state::<AppLogState>().app_log.clone();
    let _ = append_log_line(&log_path, &format!("console stop requested pid={pid}"));
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(pid as i32, libc::SIGINT);
        }
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

fn ensure_file(path: &Path, contents: &str) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    fs::write(path, contents).map_err(|err| err.to_string())?;
    Ok(())
}

fn sidecar_path(name: &str) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    let dir = exe
        .parent()
        .ok_or_else(|| "failed to resolve sidecar dir".to_string())?;
    #[cfg(windows)]
    {
        return Ok(dir.join(format!("{name}.exe")));
    }
    #[cfg(not(windows))]
    {
        return Ok(dir.join(name));
    }
}

fn resolve_linux_broker(
    app: &AppHandle,
    override_name: &str,
    resource_path: &str,
) -> Option<PathBuf> {
    if let Ok(dir) = octovalve_dir(app) {
        let candidate = dir.join(override_name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    app.path()
        .resolve(resource_path, BaseDirectory::Resource)
        .ok()
}

fn build_console_path() -> String {
    let base = std::env::var("PATH").unwrap_or_default();
    if base.is_empty() {
        "/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin".to_string()
    } else {
        format!("/usr/local/bin:/opt/homebrew/bin:{base}")
    }
}

fn octovalve_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let home = app.path().home_dir().map_err(|err| err.to_string())?;
    Ok(home.join(".octovalve"))
}

fn profiles_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(octovalve_dir(app)?.join("profiles"))
}

fn profiles_index_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(profiles_dir(app)?.join("profiles.toml"))
}

fn legacy_proxy_config_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(octovalve_dir(app)?.join("local-proxy-config.toml"))
}

fn profile_dir_for(base: &Path, name: &str) -> PathBuf {
    base.join(name)
}

fn validate_profile_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("环境名称不能为空".to_string());
    }
    if trimmed.len() > 48 {
        return Err("环境名称最长支持 48 个字符".to_string());
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err("环境名称仅支持字母、数字、- 或 _".to_string());
    }
    Ok(())
}

fn profiles_status(data: &ProfilesFile) -> ProfilesStatus {
    ProfilesStatus {
        current: data.current.clone(),
        profiles: data
            .profiles
            .iter()
            .map(|profile| ProfileSummary {
                name: profile.name.clone(),
            })
            .collect(),
    }
}

fn current_profile_entry(data: &ProfilesFile) -> Result<ProfileRecord, String> {
    data.profiles
        .iter()
        .find(|profile| profile.name == data.current)
        .cloned()
        .ok_or_else(|| "current profile missing in profiles list".to_string())
}

fn profile_entry_by_name(data: &ProfilesFile, name: &str) -> Result<ProfileRecord, String> {
    data.profiles
        .iter()
        .find(|profile| profile.name == name)
        .cloned()
        .ok_or_else(|| format!("未找到环境 {}", name))
}

fn load_profiles_file(path: &Path) -> Result<ProfilesFile, String> {
    let raw = fs::read_to_string(path).map_err(|err| err.to_string())?;
    let parsed: ProfilesFile = toml::from_str(&raw).map_err(|err| err.to_string())?;
    if parsed.profiles.is_empty() {
        return Err("profiles.toml 必须至少包含一个环境".to_string());
    }
    let mut seen = std::collections::HashSet::new();
    for profile in &parsed.profiles {
        if profile.name.trim().is_empty() {
            return Err("环境名称不能为空".to_string());
        }
        if !seen.insert(profile.name.clone()) {
            return Err(format!("重复的环境名称：{}", profile.name));
        }
    }
    Ok(parsed)
}

fn write_profiles_file(path: &Path, data: &ProfilesFile) -> Result<(), String> {
    let content = toml::to_string_pretty(data).map_err(|err| err.to_string())?;
    write_config_file(path, &content)
}

fn ensure_broker_file(profile: &ProfileRecord) -> Result<(), String> {
    ensure_file(Path::new(&profile.broker_path), DEFAULT_BROKER_CONFIG)?;
    Ok(())
}

fn sync_legacy_proxy_config(app: &AppHandle, proxy_path: &Path) -> Result<(), String> {
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
    })
}

fn remove_profile_files(profile: &ProfileRecord, profiles_base: &Path) {
    let proxy_path = Path::new(&profile.proxy_path);
    let broker_path = Path::new(&profile.broker_path);
    if let Some(dir) = proxy_path.parent() {
        if dir.starts_with(profiles_base) {
            let _ = fs::remove_dir_all(dir);
            return;
        }
    }
    if proxy_path.starts_with(profiles_base) {
        let _ = fs::remove_file(proxy_path);
    }
    if broker_path.starts_with(profiles_base) {
        let _ = fs::remove_file(broker_path);
    }
}

fn prepare_profiles(
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

fn resolve_broker_config_path(
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

fn resolve_config_path(app: &AppHandle, base: &Path, value: &str) -> Result<PathBuf, String> {
    let expanded = expand_tilde_path(app, value)?;
    if expanded.is_absolute() {
        return Ok(expanded);
    }
    let base_dir = base
        .parent()
        .ok_or_else(|| "failed to resolve config dir".to_string())?;
    Ok(base_dir.join(expanded))
}

fn resolve_profile_path(app: &AppHandle, value: &str) -> Result<PathBuf, String> {
    let expanded = expand_tilde_path(app, value)?;
    if expanded.is_absolute() {
        return Ok(expanded);
    }
    let base = profiles_index_path(app)?;
    resolve_config_path(app, &base, value)
}

fn profile_proxy_path(app: &AppHandle, profile: &ProfileRecord) -> Result<PathBuf, String> {
    resolve_profile_path(app, &profile.proxy_path)
}

fn profile_broker_path(app: &AppHandle, profile: &ProfileRecord) -> Result<PathBuf, String> {
    resolve_profile_path(app, &profile.broker_path)
}

fn expand_tilde_path(app: &AppHandle, value: &str) -> Result<PathBuf, String> {
    if value == "~" {
        return app.path().home_dir().map_err(|err| err.to_string());
    }
    if let Some(rest) = value.strip_prefix("~/") {
        let home = app.path().home_dir().map_err(|err| err.to_string())?;
        return Ok(home.join(rest));
    }
    Ok(PathBuf::from(value))
}

fn read_config_file(path: &Path, fallback: Option<&str>) -> Result<ConfigFilePayload, String> {
    let exists = path.exists();
    let content = if exists {
        fs::read_to_string(path).map_err(|err| err.to_string())?
    } else {
        fallback.unwrap_or_default().to_string()
    };
    Ok(ConfigFilePayload {
        path: path.to_string_lossy().to_string(),
        exists,
        content,
    })
}

fn write_config_file(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    fs::write(path, content).map_err(|err| err.to_string())
}

#[tauri::command]
fn get_proxy_config_status(state: State<ProxyConfigState>) -> ProxyConfigStatus {
    state.0.lock().unwrap().clone()
}

#[tauri::command]
fn list_profiles(state: State<ProfilesState>) -> ProfilesStatus {
    let data = state.0.lock().unwrap();
    profiles_status(&data)
}

#[tauri::command]
fn create_profile(
    name: String,
    app: AppHandle,
    profiles_state: State<ProfilesState>,
) -> Result<(), String> {
    validate_profile_name(&name)?;
    let profiles_base = profiles_dir(&app)?;
    let index_path = profiles_index_path(&app)?;

    let mut profiles = profiles_state.0.lock().unwrap().clone();
    if profiles.profiles.iter().any(|profile| profile.name == name) {
        return Err(format!("环境 {} 已存在", name));
    }
    let current = current_profile_entry(&profiles)?;
    let current_proxy_path = PathBuf::from(&current.proxy_path);
    let current_broker_path = PathBuf::from(&current.broker_path);

    let new_dir = profile_dir_for(&profiles_base, &name);
    fs::create_dir_all(&new_dir).map_err(|err| err.to_string())?;
    let new_proxy_path = new_dir.join("local-proxy-config.toml");
    let new_broker_path = new_dir.join("remote-broker-config.toml");

    if !new_proxy_path.exists() {
        let content = if current_proxy_path.exists() {
            fs::read_to_string(&current_proxy_path).map_err(|err| err.to_string())?
        } else {
            DEFAULT_PROXY_EXAMPLE.to_string()
        };
        write_config_file(&new_proxy_path, &content)?;
    }

    if !new_broker_path.exists() {
        let content = if current_broker_path.exists() {
            fs::read_to_string(&current_broker_path).map_err(|err| err.to_string())?
        } else {
            DEFAULT_BROKER_CONFIG.to_string()
        };
        write_config_file(&new_broker_path, &content)?;
    }

    let record = ProfileRecord {
        name: name.clone(),
        proxy_path: new_proxy_path.to_string_lossy().to_string(),
        broker_path: new_broker_path.to_string_lossy().to_string(),
    };
    profiles.profiles.push(record);
    write_profiles_file(&index_path, &profiles)?;
    *profiles_state.0.lock().unwrap() = profiles;
    Ok(())
}

#[tauri::command]
fn delete_profile(
    name: String,
    app: AppHandle,
    profiles_state: State<ProfilesState>,
) -> Result<(), String> {
    let profiles_base = profiles_dir(&app)?;
    let index_path = profiles_index_path(&app)?;
    let mut profiles = profiles_state.0.lock().unwrap().clone();
    if profiles.current == name {
        return Err("不能删除当前环境，请先切换到其他环境".to_string());
    }
    if profiles.profiles.len() <= 1 {
        return Err("至少保留一个环境".to_string());
    }
    let idx = profiles
        .profiles
        .iter()
        .position(|profile| profile.name == name)
        .ok_or_else(|| format!("未找到环境 {}", name))?;
    let removed = profiles.profiles.remove(idx);
    write_profiles_file(&index_path, &profiles)?;
    *profiles_state.0.lock().unwrap() = profiles;
    remove_profile_files(&removed, &profiles_base);
    Ok(())
}

#[tauri::command]
fn select_profile(
    name: String,
    app: AppHandle,
    profiles_state: State<ProfilesState>,
    proxy_state: State<ProxyConfigState>,
) -> Result<(), String> {
    let index_path = profiles_index_path(&app)?;
    let mut profiles = profiles_state.0.lock().unwrap().clone();
    let entry = profiles
        .profiles
        .iter()
        .find(|profile| profile.name == name)
        .cloned()
        .ok_or_else(|| format!("未找到环境 {}", name))?;
    if profiles.current == name {
        return Ok(());
    }
    profiles.current = name;
    write_profiles_file(&index_path, &profiles)?;
    *profiles_state.0.lock().unwrap() = profiles;

    let mut status = proxy_state.0.lock().unwrap();
    status.path = entry.proxy_path.clone();
    status.present = Path::new(&entry.proxy_path).exists();
    drop(status);
    let _ = sync_legacy_proxy_config(&app, Path::new(&entry.proxy_path));
    Ok(())
}

#[tauri::command]
fn read_profile_proxy_config(
    name: String,
    app: AppHandle,
    profiles_state: State<ProfilesState>,
) -> Result<ConfigFilePayload, String> {
    let profiles = profiles_state.0.lock().unwrap().clone();
    let profile = profile_entry_by_name(&profiles, &name)?;
    let path = profile_proxy_path(&app, &profile)?;
    read_config_file(&path, Some(DEFAULT_PROXY_EXAMPLE))
}

#[tauri::command]
fn write_profile_proxy_config(
    name: String,
    content: String,
    app: AppHandle,
    profiles_state: State<ProfilesState>,
) -> Result<(), String> {
    let profiles = profiles_state.0.lock().unwrap().clone();
    let profile = profile_entry_by_name(&profiles, &name)?;
    let path = profile_proxy_path(&app, &profile)?;
    write_config_file(&path, &content)
}

#[tauri::command]
fn read_profile_broker_config(
    name: String,
    app: AppHandle,
    profiles_state: State<ProfilesState>,
) -> Result<ConfigFilePayload, String> {
    let profiles = profiles_state.0.lock().unwrap().clone();
    let profile = profile_entry_by_name(&profiles, &name)?;
    let path = profile_broker_path(&app, &profile)?;
    let existed = path.exists();
    ensure_file(&path, DEFAULT_BROKER_CONFIG)?;
    let mut payload = read_config_file(&path, Some(DEFAULT_BROKER_CONFIG))?;
    payload.exists = existed;
    Ok(payload)
}

#[tauri::command]
fn write_profile_broker_config(
    name: String,
    content: String,
    app: AppHandle,
    profiles_state: State<ProfilesState>,
) -> Result<(), String> {
    let profiles = profiles_state.0.lock().unwrap().clone();
    let profile = profile_entry_by_name(&profiles, &name)?;
    let path = profile_broker_path(&app, &profile)?;
    write_config_file(&path, &content)
}

#[tauri::command]
fn read_proxy_config(state: State<ProxyConfigState>) -> Result<ConfigFilePayload, String> {
    let path = {
        let status = state.0.lock().unwrap();
        PathBuf::from(status.path.clone())
    };
    read_config_file(&path, Some(DEFAULT_PROXY_EXAMPLE))
}

#[tauri::command]
fn write_proxy_config(
    content: String,
    _app: AppHandle,
    state: State<ProxyConfigState>,
) -> Result<(), String> {
    let path = {
        let status = state.0.lock().unwrap();
        PathBuf::from(status.path.clone())
    };
    write_config_file(&path, &content)?;
    let mut status = state.0.lock().unwrap();
    status.present = true;
    Ok(())
}

#[tauri::command]
fn read_broker_config(
    app: AppHandle,
    state: State<ProxyConfigState>,
    profiles_state: State<ProfilesState>,
) -> Result<ConfigFilePayload, String> {
    let config_dir = app.path().app_config_dir().map_err(|err| err.to_string())?;
    let proxy_path = {
        let status = state.0.lock().unwrap();
        PathBuf::from(status.path.clone())
    };
    let profiles = profiles_state.0.lock().unwrap().clone();
    let resolved = resolve_broker_config_path(&app, &proxy_path, &config_dir, Some(&profiles))?;
    let existed = resolved.path.exists();
    ensure_file(&resolved.path, DEFAULT_BROKER_CONFIG)?;
    let mut payload = read_config_file(&resolved.path, Some(DEFAULT_BROKER_CONFIG))?;
    payload.exists = existed;
    Ok(payload)
}

#[tauri::command]
fn write_broker_config(
    content: String,
    app: AppHandle,
    state: State<ProxyConfigState>,
    profiles_state: State<ProfilesState>,
) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|err| err.to_string())?;
    let proxy_path = {
        let status = state.0.lock().unwrap();
        PathBuf::from(status.path.clone())
    };
    let profiles = profiles_state.0.lock().unwrap().clone();
    let resolved = resolve_broker_config_path(&app, &proxy_path, &config_dir, Some(&profiles))?;
    write_config_file(&resolved.path, &content)
}

#[tauri::command]
fn read_console_log(offset: u64, max_bytes: u64, app: AppHandle) -> Result<LogChunk, String> {
    let path = console_log_path(&app)?;
    if !path.exists() {
        return Ok(LogChunk {
            content: String::new(),
            next_offset: 0,
        });
    }
    let mut file = OpenOptions::new()
        .read(true)
        .open(&path)
        .map_err(|err| err.to_string())?;
    let len = file.metadata().map_err(|err| err.to_string())?.len();
    let start = if offset > len { 0 } else { offset };
    file.seek(SeekFrom::Start(start))
        .map_err(|err| err.to_string())?;
    if max_bytes == 0 {
        return Ok(LogChunk {
            content: String::new(),
            next_offset: len,
        });
    }
    let capped = max_bytes.min(256 * 1024) as usize;
    let mut buffer = vec![0u8; capped];
    let read = file.read(&mut buffer).map_err(|err| err.to_string())?;
    buffer.truncate(read);
    Ok(LogChunk {
        content: String::from_utf8_lossy(&buffer).to_string(),
        next_offset: start + read as u64,
    })
}

#[tauri::command]
fn restart_console(
    app: AppHandle,
    state: State<ProxyConfigState>,
    log_state: State<AppLogState>,
) -> Result<(), String> {
    let console_log = console_log_path(&app)?;
    let _ = append_log_line(&console_log, "console restart requested");
    stop_console(&app);
    let status = state.0.lock().unwrap().clone();
    if !status.present {
        return Err("proxy config missing".to_string());
    }
    match start_console(&app, Path::new(&status.path), &log_state.app_log) {
        Ok(_) => {
            let _ = append_log_line(&console_log, "console restart started");
        }
        Err(err) => {
            let _ = append_log_line(&console_log, &format!("console restart failed: {err}"));
            return Err(err);
        }
    }
    Ok(())
}

struct HttpResponse {
    status: u16,
    body: String,
}

async fn console_get(path: &str, log_path: &Path) -> Result<Value, String> {
    let response =
        console_http_request_with_timeout("GET", path, None, log_path, HTTP_IO_TIMEOUT).await?;
    if response.status / 100 != 2 {
        return Err(format!(
            "console http GET status {} for {}",
            response.status, path
        ));
    }
    serde_json::from_str(&response.body).map_err(|err| {
        let _ = append_log_line(log_path, &format!("console http GET parse error: {err}"));
        err.to_string()
    })
}

async fn console_post(path: &str, payload: Value, log_path: &Path) -> Result<(), String> {
    console_post_with_timeout(path, payload, log_path, HTTP_IO_TIMEOUT).await
}

async fn console_post_with_timeout(
    path: &str,
    payload: Value,
    log_path: &Path,
    io_timeout: Duration,
) -> Result<(), String> {
    let payload = payload.to_string();
    let _ = append_log_line(log_path, &format!("console http POST payload: {}", payload));
    let response =
        console_http_request_with_timeout("POST", path, Some(&payload), log_path, io_timeout)
            .await?;
    if response.status / 100 != 2 {
        return Err(format!(
            "console http POST status {} for {}",
            response.status, path
        ));
    }
    Ok(())
}

async fn console_http_request_with_timeout(
    method: &str,
    path: &str,
    body: Option<&str>,
    log_path: &Path,
    io_timeout: Duration,
) -> Result<HttpResponse, String> {
    let request_id = HTTP_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let body_len = body.map(|value| value.len()).unwrap_or(0);
    let _ = append_log_line(
        log_path,
        &format!("console http {method}#{request_id} start path={path} body_len={body_len}"),
    );
    let mut stream = timeout(HTTP_CONNECT_TIMEOUT, TcpStream::connect(CONSOLE_HTTP_HOST))
        .await
        .map_err(|_| "console http connect timed out".to_string())?
        .map_err(|err| err.to_string())?;
    let mut request = format!(
    "{method} {path} HTTP/1.1\r\nHost: {CONSOLE_HTTP_HOST}\r\nAccept: application/json\r\nConnection: close\r\n"
  );
    if let Some(body) = body {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
        request.push_str("\r\n");
        request.push_str(body);
    } else {
        request.push_str("\r\n");
    }
    timeout(io_timeout, stream.write_all(request.as_bytes()))
        .await
        .map_err(|_| "console http write timed out".to_string())?
        .map_err(|err| err.to_string())?;
    let mut buffer = Vec::new();
    timeout(io_timeout, stream.read_to_end(&mut buffer))
        .await
        .map_err(|_| "console http read timed out".to_string())?
        .map_err(|err| err.to_string())?;
    let (status, headers, body) = parse_http_response(&buffer)?;
    let content_type = headers
        .get("content-type")
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());
    let _ = append_log_line(
        log_path,
        &format!(
            "console http {method}#{request_id} status={} content-type={}",
            status, content_type
        ),
    );
    let _ = append_log_line(
        log_path,
        &format!("console http {method}#{request_id} body_len={}", body.len()),
    );
    let _ = append_log_line(
        log_path,
        &format!(
            "console http {method}#{request_id} body: {}",
            escape_log_body(&body)
        ),
    );
    Ok(HttpResponse { status, body })
}

fn parse_http_response(bytes: &[u8]) -> Result<(u16, HashMap<String, String>, String), String> {
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "console http response missing header".to_string())?;
    let head = String::from_utf8_lossy(&bytes[..header_end]);
    let body_bytes = &bytes[(header_end + 4)..];
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_lowercase(), value.trim().to_string());
        }
    }
    let body = if headers
        .get("transfer-encoding")
        .map(|value| value.to_lowercase().contains("chunked"))
        .unwrap_or(false)
    {
        decode_chunked_body(body_bytes)?
    } else {
        body_bytes.to_vec()
    };
    Ok((status, headers, String::from_utf8_lossy(&body).to_string()))
}

fn decode_chunked_body(body: &[u8]) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut index = 0usize;
    while index < body.len() {
        let line_end = find_crlf(body, index)
            .ok_or_else(|| "console http chunked response missing size line".to_string())?;
        let line = String::from_utf8_lossy(&body[index..line_end]);
        let size_str = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_str, 16)
            .map_err(|_| "console http chunked size parse failed".to_string())?;
        index = line_end + 2;
        if size == 0 {
            break;
        }
        if index + size > body.len() {
            return Err("console http chunked body truncated".to_string());
        }
        output.extend_from_slice(&body[index..index + size]);
        index += size;
        if index + 2 > body.len() || &body[index..index + 2] != b"\r\n" {
            return Err("console http chunked body missing terminator".to_string());
        }
        index += 2;
    }
    Ok(output)
}

fn find_crlf(body: &[u8], start: usize) -> Option<usize> {
    body[start..]
        .windows(2)
        .position(|window| window == b"\r\n")
        .map(|offset| start + offset)
}

#[tauri::command]
async fn proxy_fetch_targets(log_state: State<'_, AppLogState>) -> Result<Value, String> {
    let targets = console_get("/targets", &log_state.app_log).await?;
    let count = targets.as_array().map(|value| value.len()).unwrap_or(0);
    let _ = append_log_line(
        &log_state.app_log,
        &format!("fetch targets ok count={count}"),
    );
    Ok(targets)
}

#[tauri::command]
async fn proxy_fetch_snapshot(
    name: String,
    log_state: State<'_, AppLogState>,
) -> Result<Value, String> {
    let path = format!("/targets/{name}/snapshot");
    let snapshot = console_get(&path, &log_state.app_log).await?;
    let queue_len = snapshot
        .get("queue")
        .and_then(|value| value.as_array())
        .map(|value| value.len())
        .unwrap_or(0);
    let history_len = snapshot
        .get("history")
        .and_then(|value| value.as_array())
        .map(|value| value.len())
        .unwrap_or(0);
    let _ = append_log_line(
        &log_state.app_log,
        &format!("fetch snapshot ok target={name} queue_len={queue_len} history_len={history_len}"),
    );
    Ok(snapshot)
}

#[tauri::command]
async fn proxy_approve(
    name: String,
    id: String,
    log_state: State<'_, AppLogState>,
) -> Result<(), String> {
    let path = format!("/targets/{name}/approve");
    console_post(&path, json!({ "id": id }), &log_state.app_log).await
}

#[tauri::command]
async fn proxy_deny(
    name: String,
    id: String,
    log_state: State<'_, AppLogState>,
) -> Result<(), String> {
    let path = format!("/targets/{name}/deny");
    console_post(&path, json!({ "id": id }), &log_state.app_log).await
}

#[tauri::command]
async fn proxy_reload_remote_brokers(log_state: State<'_, AppLogState>) -> Result<(), String> {
    console_post_with_timeout(
        "/targets/reload-brokers",
        json!({}),
        &log_state.app_log,
        HTTP_RELOAD_TIMEOUT,
    )
    .await
}

#[tauri::command]
async fn ai_risk_assess(request: AiRiskRequest) -> Result<AiRiskResponse, String> {
    if request.api_key.trim().is_empty() {
        return Err("missing api key".to_string());
    }
    let url = join_base_path(&request.base_url, &request.chat_path)?;
    let timeout_ms = request.timeout_ms.unwrap_or(10000);
    let client = Client::new();
    let payload = json!({
        "model": request.model,
        "messages": [
            { "role": "user", "content": request.prompt }
        ],
        "temperature": 0.2,
    });

    let response = client
        .post(url)
        .bearer_auth(request.api_key)
        .timeout(Duration::from_millis(timeout_ms))
        .json(&payload)
        .send()
        .await
        .map_err(|err| err.to_string())?;

    let status = response.status();
    let body = response.text().await.map_err(|err| err.to_string())?;
    if !status.is_success() {
        return Err(format!("ai request failed status={} body={}", status, body));
    }

    let value: Value = serde_json::from_str(&body).map_err(|err| err.to_string())?;
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(|val| val.as_str())
        .or_else(|| {
            value
                .pointer("/choices/0/text")
                .and_then(|val| val.as_str())
        })
        .unwrap_or("")
        .trim();
    if content.is_empty() {
        return Err("ai response missing content".to_string());
    }
    parse_ai_risk_content(content)
}

fn emit_ws_status(app: &AppHandle, log_path: &Path, status: &str) {
    let _ = app.emit("console_ws_status", status.to_string());
    let _ = append_log_line(log_path, &format!("ws {status}"));
}

fn log_ws_event(log_path: &Path, payload: &Value) {
    let Some(kind) = payload.get("type").and_then(|value| value.as_str()) else {
        return;
    };
    match kind {
        "targets_snapshot" => {
            let count = payload
                .get("targets")
                .and_then(|value| value.as_array())
                .map(|value| value.len())
                .unwrap_or(0);
            let _ = append_log_line(
                log_path,
                &format!("ws event targets_snapshot count={count}"),
            );
            let _ = append_log_line(
                log_path,
                &format!("ws event targets_snapshot payload={}", payload.to_string()),
            );
        }
        "target_updated" => {
            let name = payload
                .get("target")
                .and_then(|value| value.get("name"))
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let status = payload
                .get("target")
                .and_then(|value| value.get("status"))
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            let pending = payload
                .get("target")
                .and_then(|value| value.get("pending_count"))
                .and_then(|value| value.as_i64())
                .unwrap_or(-1);
            let _ = append_log_line(
                log_path,
                &format!("ws event target_updated name={name} status={status} pending={pending}"),
            );
            let _ = append_log_line(
                log_path,
                &format!("ws event target_updated payload={}", payload.to_string()),
            );
        }
        _ => {}
    }
}

#[tauri::command]
async fn start_console_stream(
    app: AppHandle,
    stream_state: State<'_, ConsoleStreamState>,
    log_state: State<'_, AppLogState>,
) -> Result<(), String> {
    let mut running = stream_state.0.lock().unwrap();
    if *running {
        return Ok(());
    }
    *running = true;

    let app_handle = app.clone();
    let log_path = log_state.app_log.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            emit_ws_status(&app_handle, &log_path, "connecting");
            match tokio_tungstenite::connect_async(CONSOLE_WS_URL).await {
                Ok((mut stream, _)) => {
                    emit_ws_status(&app_handle, &log_path, "connected");
                    while let Some(message) = stream.next().await {
                        match message {
                            Ok(Message::Text(text)) => match serde_json::from_str::<Value>(&text) {
                                Ok(payload) => {
                                    log_ws_event(&log_path, &payload);
                                    let _ = app_handle.emit("console_event", payload);
                                }
                                Err(err) => {
                                    let _ = append_log_line(
                                        &log_path,
                                        &format!("ws parse error: {err}"),
                                    );
                                }
                            },
                            Ok(Message::Close(_)) => break,
                            Ok(Message::Binary(_))
                            | Ok(Message::Ping(_))
                            | Ok(Message::Pong(_))
                            | Ok(Message::Frame(_)) => {}
                            Err(err) => {
                                let _ =
                                    append_log_line(&log_path, &format!("ws stream error: {err}"));
                                break;
                            }
                        }
                    }
                }
                Err(err) => {
                    let _ = append_log_line(&log_path, &format!("ws connect failed: {err}"));
                }
            }
            emit_ws_status(&app_handle, &log_path, "disconnected");
            tokio::time::sleep(WS_RECONNECT_DELAY).await;
        }
    });
    Ok(())
}

#[tauri::command]
async fn terminal_open(
    name: String,
    cols: u16,
    rows: u16,
    term: Option<String>,
    app: AppHandle,
    sessions: State<'_, TerminalSessions>,
    log_state: State<'_, AppLogState>,
) -> Result<String, String> {
    let cols = cols.max(1);
    let rows = rows.max(1);
    let term = term
        .and_then(|value| {
            if value.trim().is_empty() {
                None
            } else {
                Some(value)
            }
        })
        .unwrap_or_else(|| DEFAULT_TERM.to_string());
    let url = console_terminal_url(&name, cols, rows, &term);
    let (stream, _) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|err| err.to_string())?;
    let (mut ws_tx, mut ws_rx) = stream.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let session_id = Uuid::new_v4().to_string();

    {
        let mut guard = sessions.0.lock().unwrap();
        guard.insert(session_id.clone(), TerminalSession { tx });
    }

    let log_path = log_state.app_log.clone();
    let app_handle = app.clone();
    let session_id_for_write = session_id.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(message) = rx.recv().await {
            if ws_tx.send(Message::Text(message)).await.is_err() {
                break;
            }
        }
        let _ = append_log_line(
            &log_path,
            &format!("terminal writer closed session={session_id_for_write}"),
        );
    });

    let session_id_for_read = session_id.clone();
    let app_handle_for_read = app_handle.clone();
    let log_path = log_state.app_log.clone();
    tauri::async_runtime::spawn(async move {
        let mut closed = false;
        while let Some(message) = ws_rx.next().await {
            match message {
                Ok(Message::Text(text)) => match serde_json::from_str::<TerminalMessage>(&text) {
                    Ok(TerminalMessage::Output { data }) => {
                        let _ = app_handle_for_read.emit(
                            "terminal_output",
                            json!({ "session_id": &session_id_for_read, "data": data }),
                        );
                    }
                    Ok(TerminalMessage::Exit { code }) => {
                        let _ = app_handle_for_read.emit(
                            "terminal_exit",
                            json!({ "session_id": &session_id_for_read, "code": code }),
                        );
                        closed = true;
                        break;
                    }
                    Ok(TerminalMessage::Error { message }) => {
                        let _ = app_handle_for_read.emit(
                            "terminal_error",
                            json!({ "session_id": &session_id_for_read, "message": message }),
                        );
                        closed = true;
                        break;
                    }
                    Ok(TerminalMessage::Ready { cols, rows, term }) => {
                        let _ = append_log_line(
                            &log_path,
                            &format!("terminal ready cols={cols} rows={rows} term={term}"),
                        );
                    }
                    Err(err) => {
                        let _ = append_log_line(&log_path, &format!("terminal parse error: {err}"));
                    }
                },
                Ok(Message::Close(_)) => {
                    closed = true;
                    break;
                }
                Ok(Message::Binary(_)) | Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
                Ok(Message::Frame(_)) => {}
                Err(err) => {
                    let _ = app_handle_for_read.emit(
                        "terminal_error",
                        json!({ "session_id": &session_id_for_read, "message": err.to_string() }),
                    );
                    closed = true;
                    break;
                }
            }
        }
        if !closed {
            let _ = app_handle_for_read.emit(
                "terminal_error",
                json!({ "session_id": &session_id_for_read, "message": "terminal disconnected" }),
            );
        }
        let sessions = app_handle_for_read.state::<TerminalSessions>();
        sessions.0.lock().unwrap().remove(&session_id_for_read);
        let _ = append_log_line(
            &log_path,
            &format!("terminal session closed session={session_id_for_read}"),
        );
    });

    Ok(session_id)
}

#[tauri::command]
fn terminal_input(
    session_id: String,
    data_base64: String,
    sessions: State<'_, TerminalSessions>,
) -> Result<(), String> {
    send_terminal_message(
        &session_id,
        json!({ "type": "input", "data": data_base64 }).to_string(),
        sessions,
    )
}

#[tauri::command]
fn terminal_resize(
    session_id: String,
    cols: u16,
    rows: u16,
    sessions: State<'_, TerminalSessions>,
) -> Result<(), String> {
    send_terminal_message(
        &session_id,
        json!({ "type": "resize", "cols": cols, "rows": rows }).to_string(),
        sessions,
    )
}

#[tauri::command]
fn terminal_close(session_id: String, sessions: State<'_, TerminalSessions>) -> Result<(), String> {
    let message = json!({ "type": "close" }).to_string();
    let mut guard = sessions.0.lock().unwrap();
    if let Some(session) = guard.remove(&session_id) {
        let _ = session.tx.send(message);
        return Ok(());
    }
    Err(format!("session not found: {session_id}"))
}

#[tauri::command]
fn log_ui_event(message: String, state: State<AppLogState>) -> Result<(), String> {
    append_log_line(&state.app_log, &message)
}

fn join_base_path(base: &str, path: &str) -> Result<String, String> {
    if base.trim().is_empty() {
        return Err("base_url is empty".to_string());
    }
    let normalized_base = base.trim_end_matches('/');
    let normalized_path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    Ok(format!("{normalized_base}{normalized_path}"))
}

fn parse_ai_risk_content(content: &str) -> Result<AiRiskResponse, String> {
    let payload = extract_json_block(content).unwrap_or(content);
    let parsed: AiRiskModelResponse =
        serde_json::from_str(payload).map_err(|err| err.to_string())?;
    let risk = normalize_ai_risk(&parsed.risk)
        .ok_or_else(|| "risk must be low|medium|high".to_string())?;
    Ok(AiRiskResponse {
        risk,
        reason: parsed.reason.unwrap_or_default(),
        key_points: parsed.key_points.unwrap_or_default(),
    })
}

fn extract_json_block(input: &str) -> Option<&str> {
    let start = input.find('{')?;
    let end = input.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(&input[start..=end])
}

fn normalize_ai_risk(value: &str) -> Option<String> {
    let normalized = value.trim().to_lowercase();
    match normalized.as_str() {
        "low" | "medium" | "high" => Some(normalized),
        _ => None,
    }
}

fn append_log_line(path: &Path, message: &str) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| err.to_string())?;
    let ts = humantime::format_rfc3339(SystemTime::now()).to_string();
    writeln!(file, "[{ts}] {message}").map_err(|err| err.to_string())?;
    Ok(())
}

fn escape_log_body(body: &str) -> String {
    if body.is_empty() {
        return "<empty>".to_string();
    }
    body.replace('\n', "\\n").replace('\r', "\\r")
}

fn console_terminal_url(name: &str, cols: u16, rows: u16, term: &str) -> String {
    let encoded_name = urlencoding::encode(name);
    let encoded_term = urlencoding::encode(term);
    format!(
    "ws://{CONSOLE_HTTP_HOST}/targets/{encoded_name}/terminal?cols={cols}&rows={rows}&term={encoded_term}"
  )
}

fn send_terminal_message(
    session_id: &str,
    payload: String,
    sessions: State<'_, TerminalSessions>,
) -> Result<(), String> {
    let guard = sessions.0.lock().unwrap();
    let Some(session) = guard.get(session_id) else {
        return Err(format!("session not found: {session_id}"));
    };
    session
        .tx
        .send(payload)
        .map_err(|_| "terminal session unavailable".to_string())
}
