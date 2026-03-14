use std::sync::Arc;

use anyhow::{anyhow, Result};
use codex_app_server_protocol::{
    CommandExecutionStatus, McpToolCallStatus, PatchApplyStatus, ServerNotification, ThreadItem,
    TurnPlanStepStatus, TurnStartParams, TurnStatus, UserInput,
};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

use crate::app_server::AppServerClient;
use crate::cli::CliConfig;
use crate::logging::{log_fmt, LogLevel};
use crate::protocol::{
    AuthenticateParamsInput, CancelParamsInput, ContentBlock, DeleteSessionParamsInput,
    InitializeParamsInput, JsonRpcErrorOut, JsonRpcErrorOutPayload, JsonRpcIncomingRequest,
    JsonRpcResponseOut, ListSessionsParamsInput, LoadSessionParamsInput, NewSessionParamsInput,
    PromptParamsInput,
};
use crate::sessions::{delete_workspace_session, list_workspace_sessions};
use crate::state::AcpState;
use crate::utils::{
    build_mcp_overrides, build_thread_resume_params, build_thread_start_params, insert_dual,
    load_mcp_servers, load_rollout_history, normalize_cwd, normalize_mcp_servers, save_mcp_servers,
    update_with_type, write_temp_image, SessionHandler,
};
use crate::writer::AcpWriter;

const APP_SERVER_MAX_RETRIES: u32 = 5;

fn extract_tool_result_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => return Some(text.clone()),
        Value::Object(map) => {
            if let Some(Value::Array(content)) = map.get("content") {
                for entry in content {
                    if let Some(text) = entry.get("text").and_then(Value::as_str) {
                        if !text.is_empty() {
                            return Some(text.to_string());
                        }
                    }
                    if let Some(text) = entry
                        .get("content")
                        .and_then(|value| value.get("text"))
                        .and_then(Value::as_str)
                    {
                        if !text.is_empty() {
                            return Some(text.to_string());
                        }
                    }
                }
            }
        }
        _ => {}
    }
    None
}

fn format_tool_result<T: serde::Serialize>(value: &T) -> String {
    let value = serde_json::to_value(value).unwrap_or(Value::Null);
    if let Some(text) = extract_tool_result_text(&value) {
        return text;
    }
    if let Some(structured) = value
        .get("structuredContent")
        .or_else(|| value.get("structured_content"))
    {
        if !structured.is_null() {
            if let Ok(text) = serde_json::to_string_pretty(structured) {
                return text;
            }
        }
    }
    serde_json::to_string(&value).unwrap_or_default()
}

async fn reset_session_state(
    state: &Arc<Mutex<AcpState>>,
    app_server: &Arc<AppServerClient>,
) -> Result<()> {
    let (previous_thread_id, previous_turn_id) = {
        let mut guard = state.lock().await;
        let previous = (guard.thread_id.clone(), guard.active_turn_id.clone());
        guard.session_id = None;
        guard.pending_prompt_ids.clear();
        guard.thread_id = None;
        guard.active_turn_id = None;
        guard.saw_message_delta = false;
        guard.saw_reasoning_delta = false;
        guard.retry_count = 0;
        guard.retry_exhausted = false;
        previous
    };
    if let (Some(previous_thread_id), Some(previous_turn_id)) =
        (previous_thread_id, previous_turn_id)
    {
        if let Err(err) = app_server
            .turn_interrupt_no_wait(previous_thread_id, previous_turn_id)
            .await
        {
            log_fmt(LogLevel::Warn, format_args!("turn/interrupt 失败: {err}"));
        }
    }
    Ok(())
}
async fn send_tool_call_update(
    writer: &AcpWriter,
    session_id: &str,
    call_id: String,
    status: &str,
    content: Option<String>,
) -> Result<()> {
    let mut update = update_with_type("tool_call_update");
    insert_dual(
        &mut update,
        "tool_call_id",
        "toolCallId",
        Value::String(call_id),
    );
    insert_dual(
        &mut update,
        "status",
        "status",
        Value::String(status.to_string()),
    );
    if let Some(text) = content {
        let content = vec![json!({
            "type": "content",
            "content": { "text": text }
        })];
        update.insert("content".to_string(), Value::Array(content));
    }
    send_session_update(writer, session_id, Value::Object(update)).await
}

async fn handle_error_message(
    session_id: &str,
    message: String,
    writer: &AcpWriter,
    state: &Arc<Mutex<AcpState>>,
) -> Result<()> {
    if is_retry_related_message(&message) {
        handle_retry_signal(session_id, message, false, writer, state).await?;
        return Ok(());
    }
    let mut update = update_with_type("error");
    update.insert("error".to_string(), json!({ "message": message }));
    send_session_update(writer, session_id, Value::Object(update)).await?;
    if let Some(prompt_id) = {
        let mut guard = state.lock().await;
        guard.pending_prompt_ids.pop_front()
    } {
        send_prompt_complete(writer, prompt_id, "error").await?;
    }
    {
        let mut guard = state.lock().await;
        guard.saw_message_delta = false;
        guard.saw_reasoning_delta = false;
        guard.retry_count = 0;
        guard.retry_exhausted = false;
    }
    Ok(())
}

pub(crate) async fn handle_codex_event(
    notification: ServerNotification,
    writer: &AcpWriter,
    state: &Arc<Mutex<AcpState>>,
) -> Result<()> {
    if let ServerNotification::SessionConfigured(payload) = &notification {
        let mut guard = state.lock().await;
        if guard.session_id.is_none() {
            guard.session_id = Some(payload.session_id.to_string());
        }
        guard.saw_message_delta = false;
        guard.saw_reasoning_delta = false;
        guard.retry_count = 0;
        guard.retry_exhausted = false;
        return Ok(());
    }

    let (active_thread_id, session_id) = {
        let guard = state.lock().await;
        (guard.thread_id.clone(), guard.session_id.clone())
    };
    let Some(active_thread_id) = active_thread_id else {
        return Ok(());
    };
    let Some(session_id) = session_id else {
        return Ok(());
    };

    let notification_thread_id = match &notification {
        ServerNotification::ThreadStarted(payload) => Some(payload.thread.id.clone()),
        ServerNotification::TurnStarted(payload) => Some(payload.thread_id.clone()),
        ServerNotification::TurnCompleted(payload) => Some(payload.thread_id.clone()),
        ServerNotification::TurnPlanUpdated(payload) => Some(payload.thread_id.clone()),
        ServerNotification::ItemStarted(payload) => Some(payload.thread_id.clone()),
        ServerNotification::ItemCompleted(payload) => Some(payload.thread_id.clone()),
        ServerNotification::AgentMessageDelta(payload) => Some(payload.thread_id.clone()),
        ServerNotification::ReasoningSummaryTextDelta(payload) => Some(payload.thread_id.clone()),
        ServerNotification::ReasoningTextDelta(payload) => Some(payload.thread_id.clone()),
        ServerNotification::CommandExecutionOutputDelta(payload) => Some(payload.thread_id.clone()),
        ServerNotification::FileChangeOutputDelta(payload) => Some(payload.thread_id.clone()),
        ServerNotification::McpToolCallProgress(payload) => Some(payload.thread_id.clone()),
        ServerNotification::Error(payload) => Some(payload.thread_id.clone()),
        _ => None,
    };
    if let Some(notification_thread_id) = notification_thread_id {
        if notification_thread_id != active_thread_id {
            return Ok(());
        }
    } else {
        return Ok(());
    }

    match notification {
        ServerNotification::ThreadStarted(payload) => {
            let mut guard = state.lock().await;
            if guard.session_id.is_none() {
                guard.session_id = Some(payload.thread.id.clone());
            }
        }
        ServerNotification::TurnStarted(payload) => {
            let mut guard = state.lock().await;
            guard.active_turn_id = Some(payload.turn.id);
            guard.retry_count = 0;
            guard.retry_exhausted = false;
        }
        ServerNotification::AgentMessageDelta(payload) => {
            {
                let mut guard = state.lock().await;
                guard.saw_message_delta = true;
                guard.retry_count = 0;
                guard.retry_exhausted = false;
            }
            let mut update = update_with_type("agent_message_chunk");
            update.insert("content".to_string(), json!({ "text": payload.delta }));
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        ServerNotification::ReasoningSummaryTextDelta(payload) => {
            {
                let mut guard = state.lock().await;
                guard.saw_reasoning_delta = true;
                guard.retry_count = 0;
                guard.retry_exhausted = false;
            }
            let mut update = update_with_type("agent_thought_chunk");
            update.insert("content".to_string(), json!({ "text": payload.delta }));
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        ServerNotification::ReasoningTextDelta(payload) => {
            {
                let mut guard = state.lock().await;
                guard.saw_reasoning_delta = true;
                guard.retry_count = 0;
                guard.retry_exhausted = false;
            }
            let mut update = update_with_type("agent_thought_chunk");
            update.insert("content".to_string(), json!({ "text": payload.delta }));
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        ServerNotification::TurnPlanUpdated(payload) => {
            let entries: Vec<Value> = payload
                .plan
                .into_iter()
                .map(|item| {
                    let status = match item.status {
                        TurnPlanStepStatus::Pending => "pending",
                        TurnPlanStepStatus::InProgress => "in_progress",
                        TurnPlanStepStatus::Completed => "completed",
                    };
                    json!({
                        "step": item.step,
                        "status": status,
                        "priority": "medium",
                    })
                })
                .collect();
            let mut update = update_with_type("plan");
            update.insert("entries".to_string(), Value::Array(entries));
            if let Some(explanation) = payload.explanation {
                if !explanation.trim().is_empty() {
                    update.insert("explanation".to_string(), Value::String(explanation));
                }
            }
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        ServerNotification::ItemStarted(payload) => {
            handle_thread_item_started(&session_id, payload.item, writer).await?;
        }
        ServerNotification::ItemCompleted(payload) => {
            handle_thread_item_completed(&session_id, payload.item, writer).await?;
        }
        ServerNotification::CommandExecutionOutputDelta(payload) => {
            send_tool_call_update(
                writer,
                &session_id,
                payload.item_id,
                "in_progress",
                Some(payload.delta),
            )
            .await?;
        }
        ServerNotification::FileChangeOutputDelta(payload) => {
            send_tool_call_update(
                writer,
                &session_id,
                payload.item_id,
                "in_progress",
                Some(payload.delta),
            )
            .await?;
        }
        ServerNotification::McpToolCallProgress(payload) => {
            send_tool_call_update(
                writer,
                &session_id,
                payload.item_id,
                "in_progress",
                Some(payload.message),
            )
            .await?;
        }
        ServerNotification::Error(payload) => {
            if payload.will_retry || is_retry_related_message(&payload.error.message) {
                handle_retry_signal(&session_id, payload.error.message, true, writer, state)
                    .await?;
            } else {
                handle_error_message(&session_id, payload.error.message, writer, state).await?;
            }
        }
        ServerNotification::TurnCompleted(payload) => {
            {
                let mut guard = state.lock().await;
                if guard.active_turn_id.as_deref() == Some(payload.turn.id.as_str()) {
                    guard.active_turn_id = None;
                }
            }
            match payload.turn.status {
                TurnStatus::Completed => {
                    let retry_active = {
                        let guard = state.lock().await;
                        guard.retry_count > 0 && !guard.retry_exhausted
                    };
                    if retry_active {
                        return Ok(());
                    }

                    let should_delay = {
                        let guard = state.lock().await;
                        !guard.saw_message_delta && !guard.saw_reasoning_delta
                    };
                    if should_delay {
                        sleep(Duration::from_millis(200)).await;
                        let retry_active = {
                            let guard = state.lock().await;
                            guard.retry_count > 0 && !guard.retry_exhausted
                        };
                        if retry_active {
                            return Ok(());
                        }
                    }

                    let mut update = update_with_type("task_complete");
                    update.insert(
                        "stop_reason".to_string(),
                        Value::String("end_turn".to_string()),
                    );
                    send_session_update(writer, &session_id, Value::Object(update)).await?;
                    if let Some(prompt_id) = {
                        let mut guard = state.lock().await;
                        guard.pending_prompt_ids.pop_front()
                    } {
                        send_prompt_complete(writer, prompt_id, "end_turn").await?;
                    }
                    {
                        let mut guard = state.lock().await;
                        guard.saw_message_delta = false;
                        guard.saw_reasoning_delta = false;
                        guard.retry_count = 0;
                        guard.retry_exhausted = false;
                    }
                }
                TurnStatus::Interrupted => {
                    let mut update = update_with_type("task_complete");
                    update.insert(
                        "stop_reason".to_string(),
                        Value::String("cancelled".to_string()),
                    );
                    send_session_update(writer, &session_id, Value::Object(update)).await?;
                    {
                        let mut guard = state.lock().await;
                        guard.saw_message_delta = false;
                        guard.saw_reasoning_delta = false;
                        guard.retry_count = 0;
                        guard.retry_exhausted = false;
                    }
                }
                TurnStatus::Failed => {
                    let message = payload
                        .turn
                        .error
                        .map(|error| error.message)
                        .unwrap_or_else(|| "Turn failed".to_string());
                    handle_error_message(&session_id, message, writer, state).await?;
                }
                TurnStatus::InProgress => {}
            }
        }
        _ => {}
    }

    Ok(())
}

async fn handle_thread_item_started(
    session_id: &str,
    item: ThreadItem,
    writer: &AcpWriter,
) -> Result<()> {
    match item {
        ThreadItem::CommandExecution { id, command, .. } => {
            let mut update = update_with_type("tool_call");
            insert_dual(&mut update, "tool_call_id", "toolCallId", Value::String(id));
            insert_dual(
                &mut update,
                "name",
                "name",
                Value::String("bash".to_string()),
            );
            insert_dual(
                &mut update,
                "title",
                "title",
                Value::String(command.clone()),
            );
            insert_dual(
                &mut update,
                "status",
                "status",
                Value::String("in_progress".to_string()),
            );
            insert_dual(
                &mut update,
                "raw_input",
                "rawInput",
                json!({ "command": command }),
            );
            send_session_update(writer, session_id, Value::Object(update)).await?;
        }
        ThreadItem::McpToolCall {
            id,
            server,
            tool,
            arguments,
            ..
        } => {
            let mut update = update_with_type("tool_call");
            insert_dual(&mut update, "tool_call_id", "toolCallId", Value::String(id));
            insert_dual(
                &mut update,
                "name",
                "name",
                Value::String(format!("mcp:{server}:{tool}")),
            );
            insert_dual(
                &mut update,
                "status",
                "status",
                Value::String("in_progress".to_string()),
            );
            insert_dual(&mut update, "raw_input", "rawInput", arguments);
            send_session_update(writer, session_id, Value::Object(update)).await?;
        }
        ThreadItem::FileChange { id, changes, .. } => {
            let mut update = update_with_type("tool_call");
            insert_dual(&mut update, "tool_call_id", "toolCallId", Value::String(id));
            insert_dual(
                &mut update,
                "name",
                "name",
                Value::String("edit".to_string()),
            );
            insert_dual(
                &mut update,
                "status",
                "status",
                Value::String("in_progress".to_string()),
            );
            insert_dual(
                &mut update,
                "raw_input",
                "rawInput",
                serde_json::to_value(changes).unwrap_or(Value::Null),
            );
            send_session_update(writer, session_id, Value::Object(update)).await?;
        }
        ThreadItem::WebSearch { id, query } => {
            let mut update = update_with_type("tool_call");
            insert_dual(&mut update, "tool_call_id", "toolCallId", Value::String(id));
            insert_dual(
                &mut update,
                "name",
                "name",
                Value::String("web_search".to_string()),
            );
            insert_dual(&mut update, "title", "title", Value::String(query.clone()));
            insert_dual(
                &mut update,
                "status",
                "status",
                Value::String("in_progress".to_string()),
            );
            send_session_update(writer, session_id, Value::Object(update)).await?;
        }
        _ => {}
    }

    Ok(())
}

async fn handle_thread_item_completed(
    session_id: &str,
    item: ThreadItem,
    writer: &AcpWriter,
) -> Result<()> {
    match item {
        ThreadItem::CommandExecution {
            id,
            status,
            aggregated_output,
            exit_code,
            ..
        } => {
            let status = match status {
                CommandExecutionStatus::Completed => "completed",
                CommandExecutionStatus::InProgress => "in_progress",
                CommandExecutionStatus::Failed | CommandExecutionStatus::Declined => "failed",
            };
            let content = aggregated_output.or_else(|| {
                exit_code.map(|value| {
                    if value == 0 {
                        String::new()
                    } else {
                        format!("Exit code: {value}")
                    }
                })
            });
            send_tool_call_update(
                writer,
                session_id,
                id,
                status,
                content.filter(|value| !value.is_empty()),
            )
            .await?;
        }
        ThreadItem::McpToolCall {
            id,
            status,
            result,
            error,
            ..
        } => {
            let status = match status {
                McpToolCallStatus::Completed => "completed",
                McpToolCallStatus::InProgress => "in_progress",
                McpToolCallStatus::Failed => "failed",
            };
            let content = result
                .map(|value| format_tool_result(&value))
                .or_else(|| error.map(|value| value.message))
                .filter(|value| !value.is_empty());
            send_tool_call_update(writer, session_id, id, status, content).await?;
        }
        ThreadItem::FileChange {
            id,
            status,
            changes,
        } => {
            let status = match status {
                PatchApplyStatus::Completed => "completed",
                PatchApplyStatus::InProgress => "in_progress",
                PatchApplyStatus::Failed | PatchApplyStatus::Declined => "failed",
            };
            let content = changes
                .iter()
                .map(|change| change.diff.clone())
                .filter(|diff| !diff.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            let content = if content.is_empty() {
                None
            } else {
                Some(content)
            };
            send_tool_call_update(writer, session_id, id, status, content).await?;
        }
        ThreadItem::WebSearch { id, query } => {
            send_tool_call_update(writer, session_id, id, "completed", Some(query)).await?;
        }
        _ => {}
    }

    Ok(())
}
pub(crate) async fn handle_acp_request(
    request: JsonRpcIncomingRequest,
    writer: &AcpWriter,
    state: &Arc<Mutex<AcpState>>,
    app_server: &Arc<AppServerClient>,
    config: &CliConfig,
) -> Result<()> {
    let request_id = request.id;
    if let Err(err) = handle_acp_request_inner(request, writer, state, app_server, config).await {
        let response = JsonRpcErrorOut {
            jsonrpc: "2.0",
            id: request_id,
            error: JsonRpcErrorOutPayload {
                code: -32000,
                message: err.to_string(),
                data: None,
            },
        };
        writer.send_json(&response).await?;
    }
    Ok(())
}

async fn handle_acp_request_inner(
    request: JsonRpcIncomingRequest,
    writer: &AcpWriter,
    state: &Arc<Mutex<AcpState>>,
    app_server: &Arc<AppServerClient>,
    config: &CliConfig,
) -> Result<()> {
    match request.method.as_str() {
        "initialize" => {
            let _params: InitializeParamsInput = request
                .params
                .as_ref()
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()?
                .unwrap_or(InitializeParamsInput {
                    protocol_version: "1".to_string(),
                    client_capabilities: Value::Null,
                    client_info: Value::Null,
                });

            let mut guard = state.lock().await;
            if !guard.app_server_initialized {
                guard.app_server_initialized = true;
                drop(guard);
                app_server.initialize().await?;
            }

            let mut result = serde_json::Map::new();
            insert_dual(
                &mut result,
                "protocol_version",
                "protocolVersion",
                Value::String("1".to_string()),
            );
            let capabilities = json!({
                "promptCapabilities": {
                    "embeddedContext": true,
                    "image": true
                },
                "loadSession": true
            });
            insert_dual(
                &mut result,
                "agent_capabilities",
                "agentCapabilities",
                capabilities,
            );
            let info = json!({
                "name": "acp-codex",
                "version": env!("CARGO_PKG_VERSION"),
                "title": "Codex"
            });
            insert_dual(&mut result, "agent_info", "agentInfo", info);
            insert_dual(
                &mut result,
                "auth_methods",
                "authMethods",
                Value::Array(Vec::new()),
            );
            let response = JsonRpcResponseOut {
                jsonrpc: "2.0",
                id: request.id,
                result: Value::Object(result),
            };
            writer.send_json(&response).await?;
        }
        "authenticate" => {
            let _params: AuthenticateParamsInput = request
                .params
                .as_ref()
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()?
                .unwrap_or(AuthenticateParamsInput {
                    method_id: "".to_string(),
                });
            let response = JsonRpcResponseOut {
                jsonrpc: "2.0",
                id: request.id,
                result: Value::Null,
            };
            writer.send_json(&response).await?;
        }
        "session/new" => {
            let params: NewSessionParamsInput = request
                .params
                .as_ref()
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()?
                .ok_or_else(|| anyhow!("session/new missing params"))?;
            let cwd = normalize_cwd(&params.cwd);
            reset_session_state(state, app_server).await?;
            let mut thread_params = build_thread_start_params(config, &cwd)?;
            if let Some(overrides) = build_mcp_overrides(&params.mcp_servers) {
                thread_params.config = Some(overrides);
            }
            let response = app_server.thread_start(thread_params).await?;
            if !params.mcp_servers.is_empty() {
                if let Err(err) = save_mcp_servers(&response.thread.path, &params.mcp_servers) {
                    log_fmt(LogLevel::Warn, format_args!("写入 MCP 会话配置失败: {err}"));
                }
            }
            let session_id = response.thread.id;
            {
                let mut guard = state.lock().await;
                guard.thread_id = Some(session_id.clone());
                guard.session_id = Some(session_id.clone());
                guard.active_turn_id = None;
            }

            let mut result = serde_json::Map::new();
            insert_dual(
                &mut result,
                "session_id",
                "sessionId",
                Value::String(session_id),
            );
            result.insert("modes".to_string(), Value::Array(Vec::new()));
            result.insert("models".to_string(), Value::Array(Vec::new()));
            insert_dual(
                &mut result,
                "config_options",
                "configOptions",
                Value::Array(Vec::new()),
            );
            let response = JsonRpcResponseOut {
                jsonrpc: "2.0",
                id: request.id,
                result: Value::Object(result),
            };
            writer.send_json(&response).await?;
        }
        "session/list" => {
            let _params: ListSessionsParamsInput = request
                .params
                .as_ref()
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()?
                .unwrap_or(ListSessionsParamsInput { cwd: None });
            let sessions = list_workspace_sessions()?;
            let result = json!({ "sessions": sessions });
            let response = JsonRpcResponseOut {
                jsonrpc: "2.0",
                id: request.id,
                result,
            };
            writer.send_json(&response).await?;
        }
        "session/load" => {
            let params: LoadSessionParamsInput = request
                .params
                .as_ref()
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()?
                .ok_or_else(|| anyhow!("session/load missing params"))?;

            reset_session_state(state, app_server).await?;

            let rollout_path = SessionHandler::find_rollout_file_path(&params.session_id)?;
            let cwd = normalize_cwd(".");
            let mut resume_params = build_thread_resume_params(
                config,
                &cwd,
                params.session_id.clone(),
                rollout_path.clone(),
            )?;
            let stored_mcp_servers = match load_mcp_servers(&rollout_path) {
                Ok(servers) => servers,
                Err(err) => {
                    log_fmt(LogLevel::Warn, format_args!("读取 MCP 会话配置失败: {err}"));
                    None
                }
            };
            let requested_mcp_servers = normalize_mcp_servers(&params.mcp_servers);
            let use_requested = !requested_mcp_servers.is_empty()
                && match stored_mcp_servers.as_ref() {
                    Some(stored) => stored != &requested_mcp_servers,
                    None => true,
                };
            let mcp_servers = if use_requested {
                if let Err(err) = save_mcp_servers(&rollout_path, &requested_mcp_servers) {
                    log_fmt(LogLevel::Warn, format_args!("写入 MCP 会话配置失败: {err}"));
                }
                requested_mcp_servers
            } else if let Some(stored) = stored_mcp_servers {
                stored
            } else {
                Vec::new()
            };
            if let Some(overrides) = build_mcp_overrides(&mcp_servers) {
                resume_params.config = Some(overrides);
            }

            let response = app_server.thread_resume(resume_params).await?;
            {
                let mut guard = state.lock().await;
                guard.session_id = Some(params.session_id.clone());
                guard.thread_id = Some(response.thread.id);
                guard.active_turn_id = None;
            }

            let history = load_rollout_history(&rollout_path)
                .await
                .unwrap_or_default();

            let result = json!({
                "modes": [],
                "models": [],
                "history": history,
            });
            let response = JsonRpcResponseOut {
                jsonrpc: "2.0",
                id: request.id,
                result,
            };
            writer.send_json(&response).await?;
        }
        "session/delete" => {
            let params: DeleteSessionParamsInput = request
                .params
                .as_ref()
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()?
                .ok_or_else(|| anyhow!("session/delete missing params"))?;
            delete_workspace_session(&params.session_id)?;
            let response = JsonRpcResponseOut {
                jsonrpc: "2.0",
                id: request.id,
                result: Value::Null,
            };
            writer.send_json(&response).await?;
        }
        "session/prompt" => {
            let params: PromptParamsInput = request
                .params
                .as_ref()
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()?
                .ok_or_else(|| anyhow!("session/prompt missing params"))?;

            let (thread_id, session_id) = {
                let guard = state.lock().await;
                (guard.thread_id.clone(), guard.session_id.clone())
            };
            let thread_id = thread_id.ok_or_else(|| anyhow!("尚未初始化会话"))?;
            let session_id = session_id.ok_or_else(|| anyhow!("尚未初始化会话"))?;

            if params.session_id != session_id {
                return Err(anyhow!("session_id 不匹配"));
            }

            {
                let mut guard = state.lock().await;
                guard.saw_message_delta = false;
                guard.saw_reasoning_delta = false;
                guard.retry_count = 0;
                guard.retry_exhausted = false;
            }

            let mut input = Vec::new();
            for block in params.prompt {
                match block {
                    ContentBlock::Text { text } => {
                        if !text.trim().is_empty() {
                            input.push(UserInput::Text { text });
                        }
                    }
                    ContentBlock::Image { data, mime_type } => {
                        match write_temp_image(&data, &mime_type) {
                            Ok(path) => {
                                input.push(UserInput::LocalImage { path });
                            }
                            Err(err) => {
                                log_fmt(
                                    LogLevel::Warn,
                                    format_args!("无法处理 image block: {err}"),
                                );
                            }
                        }
                    }
                }
            }
            if input.is_empty() {
                let response = JsonRpcResponseOut {
                    jsonrpc: "2.0",
                    id: request.id,
                    result: json!({ "stopReason": "empty" }),
                };
                writer.send_json(&response).await?;
                return Ok(());
            }

            let response = app_server
                .turn_start(TurnStartParams {
                    thread_id,
                    input,
                    cwd: None,
                    approval_policy: None,
                    sandbox_policy: None,
                    model: None,
                    effort: None,
                    summary: None,
                })
                .await?;

            {
                let mut guard = state.lock().await;
                guard.active_turn_id = Some(response.turn.id);
                guard.pending_prompt_ids.push_back(request.id);
            }
        }
        "session/cancel" => {
            let _params: CancelParamsInput = request
                .params
                .as_ref()
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()?
                .unwrap_or(CancelParamsInput {
                    session_id: "".to_string(),
                });
            let (thread_id, active_turn_id) = {
                let guard = state.lock().await;
                (guard.thread_id.clone(), guard.active_turn_id.clone())
            };
            if let (Some(thread_id), Some(active_turn_id)) = (thread_id, active_turn_id) {
                if let Err(err) = app_server
                    .turn_interrupt_no_wait(thread_id, active_turn_id)
                    .await
                {
                    log_fmt(LogLevel::Warn, format_args!("turn/interrupt 失败: {err}"));
                }
            }
            if let Some(prompt_id) = {
                let mut guard = state.lock().await;
                guard.active_turn_id = None;
                guard.pending_prompt_ids.pop_front()
            } {
                send_prompt_complete(writer, prompt_id, "cancelled").await?;
            }
            {
                let mut guard = state.lock().await;
                guard.saw_message_delta = false;
                guard.saw_reasoning_delta = false;
                guard.retry_count = 0;
                guard.retry_exhausted = false;
            }
            let response = JsonRpcResponseOut {
                jsonrpc: "2.0",
                id: request.id,
                result: Value::Null,
            };
            writer.send_json(&response).await?;
        }
        _ => {
            let response = JsonRpcErrorOut {
                jsonrpc: "2.0",
                id: request.id,
                error: JsonRpcErrorOutPayload {
                    code: -32601,
                    message: format!("unknown method: {}", request.method),
                    data: None,
                },
            };
            writer.send_json(&response).await?;
        }
    }

    Ok(())
}

async fn send_session_update(writer: &AcpWriter, session_id: &str, update: Value) -> Result<()> {
    let params = json!({
        "session_id": session_id,
        "sessionId": session_id,
        "update": update,
    });
    let message = json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": params,
    });
    writer.send_json(&message).await
}

async fn send_prompt_complete(writer: &AcpWriter, id: u64, stop_reason: &str) -> Result<()> {
    let response = JsonRpcResponseOut {
        jsonrpc: "2.0",
        id,
        result: json!({ "stopReason": stop_reason }),
    };
    writer.send_json(&response).await
}

pub(crate) async fn handle_app_server_stderr_line(
    line: String,
    writer: &AcpWriter,
    state: &Arc<Mutex<AcpState>>,
) -> Result<()> {
    let session_id = {
        let guard = state.lock().await;
        guard.session_id.clone()
    };
    let Some(session_id) = session_id else {
        return Ok(());
    };

    let is_rate_limit = line.contains("error=http 429")
        || line.contains("Too Many Requests")
        || line.to_lowercase().contains("rate_limit");
    if !is_rate_limit {
        return Ok(());
    }

    let message = extract_canonical_error_message(&line);
    handle_retry_signal(&session_id, message, true, writer, state).await
}

fn extract_canonical_error_message(line: &str) -> String {
    let needle = "\\\"message\\\":\\\"";
    if let Some(start) = line.find(needle) {
        let remainder = &line[start + needle.len()..];
        if let Some(end) = remainder.find("\\\"") {
            let raw = &remainder[..end];
            return raw.replace("\\n", " ");
        }
    }
    line.to_string()
}

fn is_retry_related_message(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("reconnecting")
        || lower.contains("retrying")
        || message.contains("429")
        || message.contains("Too Many Requests")
        || lower.contains("rate_limit")
        || lower.contains("spending limit")
        || lower.contains("weekly spending")
}

fn parse_retry_progress(message: &str) -> Option<u32> {
    // Extract the attempt from patterns like "1/5" or "Reconnecting... 2/5".
    // We only need attempt; max attempts is controlled by APP_SERVER_MAX_RETRIES.
    let bytes = message.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let attempt_str = &message[start..i];
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'/' {
                if let Ok(attempt) = attempt_str.parse::<u32>() {
                    return Some(attempt);
                }
            }
            continue;
        }
        i += 1;
    }
    None
}

async fn handle_retry_signal(
    session_id: &str,
    message: String,
    increment_if_missing: bool,
    writer: &AcpWriter,
    state: &Arc<Mutex<AcpState>>,
) -> Result<()> {
    let parsed_attempt = parse_retry_progress(&message);
    let (attempt, exhausted) = {
        let mut guard = state.lock().await;
        if guard.retry_exhausted {
            return Ok(());
        }
        if let Some(parsed) = parsed_attempt {
            guard.retry_count = guard.retry_count.max(parsed);
        } else if increment_if_missing {
            guard.retry_count = guard.retry_count.saturating_add(1);
        }
        if guard.retry_count == 0 {
            return Ok(());
        }
        if guard.retry_count >= APP_SERVER_MAX_RETRIES {
            guard.retry_exhausted = true;
        }
        (guard.retry_count, guard.retry_exhausted)
    };

    if !exhausted {
        let mut update = update_with_type("retry");
        insert_dual(
            &mut update,
            "attempt",
            "attempt",
            Value::Number(serde_json::Number::from(attempt)),
        );
        insert_dual(
            &mut update,
            "max_attempts",
            "maxAttempts",
            Value::Number(serde_json::Number::from(APP_SERVER_MAX_RETRIES)),
        );
        update.insert("message".to_string(), Value::String(message));
        send_session_update(writer, session_id, Value::Object(update)).await?;
        return Ok(());
    }

    // Emit the final attempt as a retry update so the UI can display [max/max]
    // before we close out the prompt with an error.
    {
        let mut update = update_with_type("retry");
        insert_dual(
            &mut update,
            "attempt",
            "attempt",
            Value::Number(serde_json::Number::from(APP_SERVER_MAX_RETRIES)),
        );
        insert_dual(
            &mut update,
            "max_attempts",
            "maxAttempts",
            Value::Number(serde_json::Number::from(APP_SERVER_MAX_RETRIES)),
        );
        update.insert("message".to_string(), Value::String(message.clone()));
        send_session_update(writer, session_id, Value::Object(update)).await?;
    }

    let error_message = format!(
        "Request failed after {} retries: {}",
        APP_SERVER_MAX_RETRIES, message
    );
    let mut update = update_with_type("error");
    update.insert("error".to_string(), json!({ "message": error_message }));
    send_session_update(writer, session_id, Value::Object(update)).await?;
    if let Some(prompt_id) = {
        let mut guard = state.lock().await;
        guard.pending_prompt_ids.pop_front()
    } {
        send_prompt_complete(writer, prompt_id, "error").await?;
    }
    {
        let mut guard = state.lock().await;
        guard.saw_message_delta = false;
        guard.saw_reasoning_delta = false;
    }

    Ok(())
}
