use std::time::Duration;

use futures_util::StreamExt;
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};
use tokio_tungstenite::tungstenite::Message;

use crate::services::logging::append_log_line;
use crate::state::{AppLogState, ConsoleStreamState};

const CONSOLE_WS_URL: &str = "ws://127.0.0.1:19309/ws";
const WS_RECONNECT_DELAY: Duration = Duration::from_secs(3);

fn emit_ws_status(app: &AppHandle, log_path: &std::path::Path, status: &str) {
    let _ = app.emit("console_ws_status", status.to_string());
    let _ = append_log_line(log_path, &format!("ws {status}"));
}

fn log_ws_event(log_path: &std::path::Path, payload: &Value) {
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

pub async fn start_console_stream(
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
