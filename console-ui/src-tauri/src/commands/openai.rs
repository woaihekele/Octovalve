use tauri::State;

use crate::clients::openai_client::{ChatMessage, OpenAiConfig, Tool};
use crate::clients::OpenAiClientState;
use crate::services::openai;
use crate::state::AppLogState;

#[tauri::command]
pub async fn openai_init(
    state: State<'_, OpenAiClientState>,
    log_state: State<'_, AppLogState>,
    config: OpenAiConfig,
) -> Result<(), String> {
    openai::openai_init(state, log_state, config).await
}

#[tauri::command]
pub async fn openai_add_message(
    state: State<'_, OpenAiClientState>,
    message: ChatMessage,
) -> Result<(), String> {
    openai::openai_add_message(state, message).await
}

#[tauri::command]
pub async fn openai_set_tools(
    state: State<'_, OpenAiClientState>,
    tools: Vec<Tool>,
) -> Result<(), String> {
    openai::openai_set_tools(state, tools).await
}

#[tauri::command]
pub async fn openai_clear_messages(state: State<'_, OpenAiClientState>) -> Result<(), String> {
    openai::openai_clear_messages(state).await
}

#[tauri::command]
pub async fn openai_cancel(state: State<'_, OpenAiClientState>) -> Result<(), String> {
    openai::openai_cancel(state).await
}

#[tauri::command]
pub async fn openai_send(
    app: tauri::AppHandle,
    state: State<'_, OpenAiClientState>,
) -> Result<(), String> {
    openai::openai_send(app, state).await
}
