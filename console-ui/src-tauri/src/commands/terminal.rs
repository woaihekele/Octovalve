use tauri::State;

use crate::services::terminal;
use crate::state::{AppLogState, TerminalSessions};

#[tauri::command]
pub async fn terminal_open(
    name: String,
    cols: u16,
    rows: u16,
    term: Option<String>,
    app: tauri::AppHandle,
    sessions: State<'_, TerminalSessions>,
    log_state: State<'_, AppLogState>,
) -> Result<String, String> {
    terminal::terminal_open(name, cols, rows, term, app, sessions, log_state).await
}

#[tauri::command]
pub fn terminal_input(
    session_id: String,
    data_base64: String,
    sessions: State<'_, TerminalSessions>,
) -> Result<(), String> {
    terminal::terminal_input(session_id, data_base64, sessions)
}

#[tauri::command]
pub fn terminal_resize(
    session_id: String,
    cols: u16,
    rows: u16,
    sessions: State<'_, TerminalSessions>,
) -> Result<(), String> {
    terminal::terminal_resize(session_id, cols, rows, sessions)
}

#[tauri::command]
pub fn terminal_close(
    session_id: String,
    sessions: State<'_, TerminalSessions>,
) -> Result<(), String> {
    terminal::terminal_close(session_id, sessions)
}
