#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use tauri::api::process::{Command, CommandChild, CommandEvent};
use tauri::{AppHandle, Manager, RunEvent};

const DEFAULT_PROXY_CONFIG: &str = include_str!("../resources/local-proxy-config.toml");
const DEFAULT_BROKER_CONFIG: &str = include_str!("../../../config/config.toml");

struct ConsoleSidecarState(Mutex<Option<CommandChild>>);

fn main() {
  tauri::Builder::default()
    .manage(ConsoleSidecarState(Mutex::new(None)))
    .setup(|app| {
      if let Err(err) = start_console(app.handle()) {
        eprintln!("failed to start console sidecar: {err}");
      }
      Ok(())
    })
    .build(tauri::generate_context!())
    .expect("error while running tauri application")
    .run(|app_handle, event| {
      if let RunEvent::ExitRequested { .. } = event {
        stop_console(app_handle);
      }
    });
}

fn start_console(app: &AppHandle) -> Result<(), String> {
  let config_dir = app
    .path_resolver()
    .app_config_dir()
    .ok_or_else(|| "failed to resolve app config dir".to_string())?;
  fs::create_dir_all(&config_dir).map_err(|err| err.to_string())?;

  let proxy_config = config_dir.join("local-proxy-config.toml");
  let broker_config = config_dir.join("remote-broker-config.toml");
  ensure_file(&proxy_config, DEFAULT_PROXY_CONFIG)?;
  ensure_file(&broker_config, DEFAULT_BROKER_CONFIG)?;

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
    ])
    .envs(envs)
    .spawn()
    .map_err(|err| err.to_string())?;

  *app.state::<ConsoleSidecarState>().0.lock().unwrap() = Some(child);

  tauri::async_runtime::spawn(async move {
    while let Some(event) = rx.recv().await {
      match event {
        CommandEvent::Error(err) => eprintln!("console sidecar error: {err}"),
        CommandEvent::Terminated(payload) => {
          eprintln!("console sidecar terminated: {:?}", payload.code)
        }
        _ => {}
      }
    }
  });

  Ok(())
}

fn stop_console(app: &AppHandle) {
  let mut guard = app.state::<ConsoleSidecarState>().0.lock().unwrap();
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
