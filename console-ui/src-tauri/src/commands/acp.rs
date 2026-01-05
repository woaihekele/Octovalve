use tauri::State;

use crate::clients::acp_types::{AcpInitResponse, AcpSessionInfo, ContextItem, LoadSessionResult};
use crate::services::acp;
use crate::clients::AcpClientState;
use crate::state::ProxyConfigState;

#[tauri::command]
pub async fn acp_start(
    app: tauri::AppHandle,
    state: State<'_, AcpClientState>,
    proxy_state: State<'_, ProxyConfigState>,
    cwd: String,
    codex_acp_path: Option<String>,
    acp_args: Option<String>,
) -> Result<AcpInitResponse, String> {
    acp::acp_start(app, state, proxy_state, cwd, codex_acp_path, acp_args).await
}

#[tauri::command]
pub async fn acp_authenticate(app: tauri::AppHandle, method_id: String) -> Result<(), String> {
    acp::acp_authenticate(app, method_id).await
}

#[tauri::command]
pub async fn acp_new_session(app: tauri::AppHandle, cwd: String) -> Result<AcpSessionInfo, String> {
    acp::acp_new_session(app, cwd).await
}

#[tauri::command]
pub async fn acp_load_session(
    app: tauri::AppHandle,
    session_id: String,
) -> Result<LoadSessionResult, String> {
    acp::acp_load_session(app, session_id).await
}

#[tauri::command]
pub async fn acp_prompt(
    app: tauri::AppHandle,
    content: String,
    context: Option<Vec<ContextItem>>,
) -> Result<(), String> {
    acp::acp_prompt(app, content, context).await
}

#[tauri::command]
pub async fn acp_cancel(app: tauri::AppHandle) -> Result<(), String> {
    acp::acp_cancel(app).await
}

#[tauri::command]
pub async fn acp_stop(state: State<'_, AcpClientState>) -> Result<(), String> {
    acp::acp_stop(state).await
}
