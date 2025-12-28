#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use futures_util::StreamExt;
use serde_json::{json, Value};
use tauri::api::path::home_dir;
use tauri::api::process::{Command, CommandChild, CommandEvent};
use tauri::{AppHandle, Manager, RunEvent, State};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
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

const CONSOLE_HTTP_HOST: &str = "127.0.0.1:19309";
const CONSOLE_WS_URL: &str = "ws://127.0.0.1:19309/ws";
const WS_RECONNECT_DELAY: Duration = Duration::from_secs(3);
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const HTTP_IO_TIMEOUT: Duration = Duration::from_secs(5);
static HTTP_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

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
        if let Err(err) = start_console(&app.handle(), &proxy_path, &app_log) {
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

fn start_console(app: &AppHandle, proxy_config: &Path, app_log: &Path) -> Result<(), String> {
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
  let _ = append_log_line(
    app_log,
    &format!("console log path: {}", console_log.display()),
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
  let tunnel_bin = sidecar_path("tunnel-daemon")?;
  let mut envs = HashMap::new();
  envs.insert(
    "OCTOVALVE_TUNNEL_DAEMON_BIN".to_string(),
    tunnel_bin.to_string_lossy().to_string(),
  );
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

  let (mut rx, child) = Command::new_sidecar("console")
    .map_err(|err| err.to_string())?
    .args(console_args)
    .envs(envs)
    .spawn()
    .map_err(|err| err.to_string())?;
  let _ = append_log_line(
    app_log,
    &format!("console sidecar started pid={}", child.pid()),
  );

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

fn resolve_linux_broker(app: &AppHandle, override_name: &str, resource_path: &str) -> Option<PathBuf> {
  if let Ok(dir) = octovalve_dir() {
    let candidate = dir.join(override_name);
    if candidate.exists() {
      return Some(candidate);
    }
  }
  app.path_resolver().resolve_resource(resource_path)
}

fn build_console_path() -> String {
  let base = std::env::var("PATH").unwrap_or_default();
  if base.is_empty() {
    "/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin".to_string()
  } else {
    format!("/usr/local/bin:/opt/homebrew/bin:{base}")
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

struct HttpResponse {
  status: u16,
  body: String,
}

async fn console_get(path: &str, log_path: &Path) -> Result<Value, String> {
  let response = console_http_request("GET", path, None, log_path).await?;
  if response.status / 100 != 2 {
    return Err(format!("console http GET status {} for {}", response.status, path));
  }
  serde_json::from_str(&response.body).map_err(|err| {
    let _ = append_log_line(
      log_path,
      &format!("console http GET parse error: {err}"),
    );
    err.to_string()
  })
}

async fn console_post(path: &str, payload: Value, log_path: &Path) -> Result<(), String> {
  let payload = payload.to_string();
  let _ = append_log_line(
    log_path,
    &format!("console http POST payload: {}", payload),
  );
  let response =
    console_http_request("POST", path, Some(&payload), log_path).await?;
  if response.status / 100 != 2 {
    return Err(format!(
      "console http POST status {} for {}",
      response.status,
      path
    ));
  }
  Ok(())
}

async fn console_http_request(
  method: &str,
  path: &str,
  body: Option<&str>,
  log_path: &Path,
) -> Result<HttpResponse, String> {
  let request_id = HTTP_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
  let body_len = body.map(|value| value.len()).unwrap_or(0);
  let _ = append_log_line(
    log_path,
    &format!(
      "console http {method}#{request_id} start path={path} body_len={body_len}"
    ),
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
  timeout(HTTP_IO_TIMEOUT, stream.write_all(request.as_bytes()))
    .await
    .map_err(|_| "console http write timed out".to_string())?
    .map_err(|err| err.to_string())?;
  let mut buffer = Vec::new();
  timeout(HTTP_IO_TIMEOUT, stream.read_to_end(&mut buffer))
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
    &format!(
      "console http {method}#{request_id} body_len={}",
      body.len()
    ),
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

fn parse_http_response(
  bytes: &[u8],
) -> Result<(u16, HashMap<String, String>, String), String> {
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
  Ok((
    status,
    headers,
    String::from_utf8_lossy(&body).to_string(),
  ))
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
    &format!(
      "fetch snapshot ok target={name} queue_len={queue_len} history_len={history_len}"
    ),
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

fn emit_ws_status(app: &AppHandle, log_path: &Path, status: &str) {
  let _ = app.emit_all("console_ws_status", status.to_string());
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
