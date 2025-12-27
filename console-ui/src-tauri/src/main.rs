#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use tauri::api::path::home_dir;
use tauri::api::process::{Command, CommandChild, CommandEvent};
use tauri::{AppHandle, Manager, RunEvent, State};
use tokio_tungstenite::tungstenite::Message;

const DEFAULT_PROXY_EXAMPLE: &str = include_str!("../resources/local-proxy-config.toml.example");
const DEFAULT_BROKER_CONFIG: &str = include_str!("../../../config/config.toml");

struct ConsoleSidecarState(Mutex<Option<CommandChild>>);
struct ConsoleStreamState(Mutex<bool>);
struct ProxyConfigState(ProxyConfigStatus);
struct AppLogState {
  app_log: PathBuf,
}

#[derive(Clone, serde::Serialize)]
struct ProxyConfigStatus {
  present: bool,
  path: String,
  example_path: String,
}

const CONSOLE_HTTP_BASE: &str = "http://127.0.0.1:19309";
const CONSOLE_WS_URL: &str = "ws://127.0.0.1:19309/ws";
const WS_RECONNECT_DELAY: Duration = Duration::from_secs(3);

fn main() {
  tauri::Builder::default()
    .manage(ConsoleSidecarState(Mutex::new(None)))
    .manage(ConsoleStreamState(Mutex::new(false)))
    .invoke_handler(tauri::generate_handler![
      get_proxy_config_status,
      log_ui_event,
      proxy_fetch_targets,
      proxy_fetch_snapshot,
      proxy_approve,
      proxy_deny,
      start_console_stream
    ])
    .setup(|app| {
      let config_dir = app
        .path_resolver()
        .app_config_dir()
        .ok_or_else(|| "failed to resolve app config dir".to_string())?;
      fs::create_dir_all(&config_dir).map_err(|err| err.to_string())?;
      let logs_dir = config_dir.join("logs");
      fs::create_dir_all(&logs_dir).map_err(|err| err.to_string())?;
      let app_log = logs_dir.join("app.log");
      app.manage(AppLogState {
        app_log: app_log.clone(),
      });
      let (proxy_status, proxy_path) = prepare_proxy_config(&app_log)?;
      app.manage(ProxyConfigState(proxy_status.clone()));
      if proxy_status.present {
        if let Err(err) = start_console(&app.handle(), &proxy_path) {
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
    .build(tauri::generate_context!())
    .expect("error while running tauri application")
    .run(|app_handle, event| {
      match event {
        RunEvent::ExitRequested { .. } => {
          stop_console(app_handle);
        }
        RunEvent::Exit => {
          stop_console(app_handle);
          tauri::api::process::kill_children();
        }
        _ => {}
      }
    });
}

fn start_console(app: &AppHandle, proxy_config: &Path) -> Result<(), String> {
  let config_dir = app
    .path_resolver()
    .app_config_dir()
    .ok_or_else(|| "failed to resolve app config dir".to_string())?;
  fs::create_dir_all(&config_dir).map_err(|err| err.to_string())?;

  let broker_config = config_dir.join("remote-broker-config.toml");
  ensure_file(&broker_config, DEFAULT_BROKER_CONFIG)?;
  let logs_dir = config_dir.join("logs");
  fs::create_dir_all(&logs_dir).map_err(|err| err.to_string())?;
  let console_log = logs_dir.join("console.log");

  let broker_bin = sidecar_path("remote-broker")?;
  let tunnel_bin = sidecar_path("tunnel-daemon")?;
  let mut envs = HashMap::new();
  envs.insert(
    "OCTOVALVE_TUNNEL_DAEMON_BIN".to_string(),
    tunnel_bin.to_string_lossy().to_string(),
  );

  let (mut rx, child) = Command::new_sidecar("console")
    .map_err(|err| err.to_string())?
    .args([
      "--config",
      proxy_config.to_string_lossy().as_ref(),
      "--broker-bin",
      broker_bin.to_string_lossy().as_ref(),
      "--broker-config",
      broker_config.to_string_lossy().as_ref(),
      "--log-to-stderr",
    ])
    .envs(envs)
    .spawn()
    .map_err(|err| err.to_string())?;

  *app.state::<ConsoleSidecarState>().0.lock().unwrap() = Some(child);

  tauri::async_runtime::spawn(async move {
    let mut file = match OpenOptions::new().create(true).append(true).open(&console_log) {
      Ok(file) => file,
      Err(err) => {
        eprintln!("failed to open console log: {err}");
        return;
      }
    };
    while let Some(event) = rx.recv().await {
      match event {
        CommandEvent::Stdout(line) => {
          let _ = writeln!(file, "[stdout] {line}");
        }
        CommandEvent::Stderr(line) => {
          let _ = writeln!(file, "[stderr] {line}");
        }
        CommandEvent::Error(err) => {
          let _ = writeln!(file, "[error] {err}");
        }
        CommandEvent::Terminated(payload) => {
          let _ = writeln!(file, "[exit] {:?}", payload.code);
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
  let Some(child) = guard.take() else {
    return;
  };
  let pid = child.pid();
  #[cfg(unix)]
  {
    unsafe {
      libc::kill(pid as i32, libc::SIGINT);
    }
    std::thread::sleep(Duration::from_millis(300));
  }
  let _ = child.kill();
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

fn octovalve_dir() -> Result<PathBuf, String> {
  let home = home_dir().ok_or_else(|| "failed to resolve home dir".to_string())?;
  Ok(home.join(".octovalve"))
}

fn prepare_proxy_config(log_path: &Path) -> Result<(ProxyConfigStatus, PathBuf), String> {
  let config_dir = octovalve_dir()?;
  fs::create_dir_all(&config_dir).map_err(|err| err.to_string())?;
  let config_path = config_dir.join("local-proxy-config.toml");
  let example_path = config_dir.join("local-proxy-config.toml.example");
  ensure_file(&example_path, DEFAULT_PROXY_EXAMPLE)?;

  let present = config_path.exists();
  let status = ProxyConfigStatus {
    present,
    path: config_path.to_string_lossy().to_string(),
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
  Ok((status, config_path))
}

#[tauri::command]
fn get_proxy_config_status(state: State<ProxyConfigState>) -> ProxyConfigStatus {
  state.0.clone()
}

fn console_http_url(path: &str) -> String {
  if path.starts_with('/') {
    format!("{CONSOLE_HTTP_BASE}{path}")
  } else {
    format!("{CONSOLE_HTTP_BASE}/{path}")
  }
}

async fn console_get(path: &str) -> Result<Value, String> {
  let url = console_http_url(path);
  let response = Client::new()
    .get(url)
    .send()
    .await
    .map_err(|err| err.to_string())?;
  let response = response
    .error_for_status()
    .map_err(|err| err.to_string())?;
  response.json::<Value>().await.map_err(|err| err.to_string())
}

async fn console_post(path: &str, payload: Value) -> Result<(), String> {
  let url = console_http_url(path);
  let response = Client::new()
    .post(url)
    .json(&payload)
    .send()
    .await
    .map_err(|err| err.to_string())?;
  response
    .error_for_status()
    .map_err(|err| err.to_string())?;
  Ok(())
}

#[tauri::command]
async fn proxy_fetch_targets() -> Result<Value, String> {
  console_get("/targets").await
}

#[tauri::command]
async fn proxy_fetch_snapshot(name: String) -> Result<Value, String> {
  let path = format!("/targets/{name}/snapshot");
  console_get(&path).await
}

#[tauri::command]
async fn proxy_approve(name: String, id: String) -> Result<(), String> {
  let path = format!("/targets/{name}/approve");
  console_post(&path, json!({ "id": id })).await
}

#[tauri::command]
async fn proxy_deny(name: String, id: String) -> Result<(), String> {
  let path = format!("/targets/{name}/deny");
  console_post(&path, json!({ "id": id })).await
}

fn emit_ws_status(app: &AppHandle, log_path: &Path, status: &str) {
  let _ = app.emit_all("console_ws_status", status.to_string());
  let _ = append_log_line(log_path, &format!("ws {status}"));
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
                  let _ = app_handle.emit_all("console_event", payload);
                }
                Err(err) => {
                  let _ = append_log_line(&log_path, &format!("ws parse error: {err}"));
                }
              },
              Ok(Message::Close(_)) => break,
              Ok(Message::Binary(_))
              | Ok(Message::Ping(_))
              | Ok(Message::Pong(_))
              | Ok(Message::Frame(_)) => {}
              Err(err) => {
                let _ = append_log_line(&log_path, &format!("ws stream error: {err}"));
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
fn log_ui_event(message: String, state: State<AppLogState>) -> Result<(), String> {
  append_log_line(&state.app_log, &message)
}

fn append_log_line(path: &Path, message: &str) -> Result<(), String> {
  let mut file = OpenOptions::new()
    .create(true)
    .append(true)
    .open(path)
    .map_err(|err| err.to_string())?;
  let ts = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs();
  writeln!(file, "[{ts}] {message}").map_err(|err| err.to_string())?;
  Ok(())
}
