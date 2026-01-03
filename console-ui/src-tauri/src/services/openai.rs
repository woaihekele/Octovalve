use tauri::State;

use crate::clients::openai_client::{ChatMessage, OpenAiClient, OpenAiClientState, OpenAiConfig, Tool};
use crate::services::logging::append_log_line;
use crate::state::AppLogState;

pub async fn openai_init(
    state: State<'_, OpenAiClientState>,
    log_state: State<'_, AppLogState>,
    config: OpenAiConfig,
) -> Result<(), String> {
    let http_proxy = env_flag("HTTP_PROXY");
    let https_proxy = env_flag("HTTPS_PROXY");
    let all_proxy = env_flag("ALL_PROXY");
    let no_proxy = std::env::var("NO_PROXY").unwrap_or_default();
    let no_proxy_has_localhost = no_proxy
        .split(',')
        .any(|value| matches!(value.trim(), "localhost" | "127.0.0.1" | "::1"));
    let no_proxy_status = if no_proxy.is_empty() {
        "unset".to_string()
    } else {
        format!("set(len={})", no_proxy.len())
    };
    let http_proxy_lower = env_flag("http_proxy");
    let https_proxy_lower = env_flag("https_proxy");
    let all_proxy_lower = env_flag("all_proxy");
    let no_proxy_lower = std::env::var("no_proxy").unwrap_or_default();
    let no_proxy_lower_has_localhost = no_proxy_lower
        .split(',')
        .any(|value| matches!(value.trim(), "localhost" | "127.0.0.1" | "::1"));
    let no_proxy_lower_status = if no_proxy_lower.is_empty() {
        "unset".to_string()
    } else {
        format!("set(len={})", no_proxy_lower.len())
    };
    let _ = append_log_line(
        &log_state.app_log,
        &format!(
            "[openai_init] base_url={} chat_path={} model={} api_key_len={}",
            config.base_url,
            config.chat_path,
            config.model,
            config.api_key.len()
        ),
    );
    let _ = append_log_line(
        &log_state.app_log,
        &format!(
            "[openai_init] env HTTP_PROXY={} HTTPS_PROXY={} ALL_PROXY={} NO_PROXY={} NO_PROXY_has_localhost={}",
            http_proxy,
            https_proxy,
            all_proxy,
            no_proxy_status,
            no_proxy_has_localhost
        ),
    );
    let _ = append_log_line(
        &log_state.app_log,
        &format!(
            "[openai_init] env http_proxy={} https_proxy={} all_proxy={} no_proxy={} no_proxy_has_localhost={}",
            http_proxy_lower,
            https_proxy_lower,
            all_proxy_lower,
            no_proxy_lower_status,
            no_proxy_lower_has_localhost
        ),
    );
    let mut guard = state.0.lock().await;
    *guard = Some(std::sync::Arc::new(OpenAiClient::new(
        config,
        log_state.app_log.clone(),
    )));
    Ok(())
}

fn env_flag(key: &str) -> String {
    match std::env::var(key) {
        Ok(value) if value.is_empty() => "empty".to_string(),
        Ok(value) => format!("set(len={})", value.len()),
        Err(_) => "unset".to_string(),
    }
}

pub async fn openai_add_message(
    state: State<'_, OpenAiClientState>,
    message: ChatMessage,
) -> Result<(), String> {
    let client = {
        let guard = state.0.lock().await;
        guard
            .as_ref()
            .ok_or("OpenAI client not initialized")?
            .clone()
    };
    client.add_message(message).await;
    Ok(())
}

pub async fn openai_set_tools(
    state: State<'_, OpenAiClientState>,
    tools: Vec<Tool>,
) -> Result<(), String> {
    let client = {
        let guard = state.0.lock().await;
        guard
            .as_ref()
            .ok_or("OpenAI client not initialized")?
            .clone()
    };
    client.set_tools(tools).await;
    Ok(())
}

pub async fn openai_clear_messages(state: State<'_, OpenAiClientState>) -> Result<(), String> {
    let client = {
        let guard = state.0.lock().await;
        guard
            .as_ref()
            .ok_or("OpenAI client not initialized")?
            .clone()
    };
    client.clear_messages().await;
    Ok(())
}

pub async fn openai_cancel(state: State<'_, OpenAiClientState>) -> Result<(), String> {
    let client = {
        let guard = state.0.lock().await;
        guard
            .as_ref()
            .ok_or("OpenAI client not initialized")?
            .clone()
    };
    client.cancel();
    Ok(())
}

pub async fn openai_send(
    app: tauri::AppHandle,
    state: State<'_, OpenAiClientState>,
) -> Result<(), String> {
    let client = {
        let guard = state.0.lock().await;
        guard
            .as_ref()
            .ok_or("OpenAI client not initialized")?
            .clone()
    };
    client.send_stream(&app).await
}
