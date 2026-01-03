use tauri::State;

use crate::clients::acp_types::{AcpInitResponse, AcpSessionInfo, ContextItem, LoadSessionResult};
use crate::services::acp;
use crate::clients::AcpClientState;
use crate::state::AppLogState;

#[tauri::command]
pub async fn acp_start(
    app: tauri::AppHandle,
    state: State<'_, AcpClientState>,
    cwd: String,
    codex_acp_path: Option<String>,
) -> Result<AcpInitResponse, String> {
    acp::acp_start(app, state, cwd, codex_acp_path).await
}

#[tauri::command]
pub fn acp_authenticate(state: State<'_, AcpClientState>, method_id: String) -> Result<(), String> {
    acp::acp_authenticate(state, method_id)
}

#[tauri::command]
pub fn acp_new_session(
    state: State<'_, AcpClientState>,
    cwd: String,
) -> Result<AcpSessionInfo, String> {
    acp::acp_new_session(state, cwd)
}

#[tauri::command]
pub fn acp_load_session(
    state: State<'_, AcpClientState>,
    session_id: String,
) -> Result<LoadSessionResult, String> {
    acp::acp_load_session(state, session_id)
}

#[tauri::command]
pub fn acp_prompt(
    state: State<'_, AcpClientState>,
    log_state: State<'_, AppLogState>,
    content: String,
    context: Option<Vec<ContextItem>>,
) -> Result<(), String> {
    acp::acp_prompt(state, log_state, content, context)
}

#[tauri::command]
pub fn acp_cancel(state: State<'_, AcpClientState>) -> Result<(), String> {
    acp::acp_cancel(state)
}

#[tauri::command]
pub async fn acp_stop(state: State<'_, AcpClientState>) -> Result<(), String> {
    acp::acp_stop(state).await
}
