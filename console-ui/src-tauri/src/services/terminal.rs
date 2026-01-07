use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use crate::services::console_http::CONSOLE_HTTP_HOST;
use crate::services::logging::append_log_line;
use crate::state::TerminalSessions;
use crate::types::terminal::TerminalMessage;

pub const DEFAULT_TERM: &str = "xterm-256color";

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

pub async fn terminal_open(
    name: String,
    cols: u16,
    rows: u16,
    term: Option<String>,
    app: AppHandle,
    sessions: State<'_, TerminalSessions>,
    log_state: State<'_, crate::state::AppLogState>,
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
        guard.insert(session_id.clone(), crate::state::TerminalSession { tx });
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

pub fn terminal_input(
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

pub fn terminal_resize(
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

pub fn terminal_close(
    session_id: String,
    sessions: State<'_, TerminalSessions>,
) -> Result<(), String> {
    let message = json!({ "type": "close" }).to_string();
    let mut guard = sessions.0.lock().unwrap();
    if let Some(session) = guard.remove(&session_id) {
        let _ = session.tx.send(message);
        return Ok(());
    }
    Err(format!("session not found: {session_id}"))
}
