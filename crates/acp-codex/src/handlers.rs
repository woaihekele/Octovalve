use std::sync::Arc;

use anyhow::{anyhow, Result};
use codex_app_server_protocol::InputItem;
use codex_protocol::{
    plan_tool::{StepStatus, UpdatePlanArgs},
    protocol::{
        AgentMessageDeltaEvent, AgentMessageEvent, AgentReasoningDeltaEvent, AgentReasoningEvent,
        ErrorEvent, EventMsg, McpToolCallBeginEvent, McpToolCallEndEvent, PatchApplyBeginEvent,
        PatchApplyEndEvent, StreamErrorEvent, WebSearchBeginEvent, WebSearchEndEvent,
    },
    ConversationId,
};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

use crate::app_server::AppServerClient;
use crate::cli::CliConfig;
use crate::protocol::{
    AuthenticateParamsInput, CancelParamsInput, ContentBlock, DeleteSessionParamsInput,
    InitializeParamsInput, JsonRpcErrorOut, JsonRpcErrorOutPayload, JsonRpcIncomingRequest,
    JsonRpcResponseOut, ListSessionsParamsInput, LoadSessionParamsInput, NewSessionParamsInput,
    PromptParamsInput,
};
use crate::sessions::{delete_workspace_session, list_workspace_sessions};
use crate::state::AcpState;
use crate::utils::{
    build_mcp_overrides, build_new_conversation_params, insert_dual, load_mcp_servers,
    load_rollout_history, normalize_cwd, save_mcp_servers, update_with_type, write_temp_image,
    SessionHandler,
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
    let (previous_conversation_id, previous_subscription_id) = {
        let mut guard = state.lock().await;
        let previous = (guard.conversation_id, guard.conversation_subscription_id);
        guard.session_id = None;
        guard.pending_prompt_ids.clear();
        guard.conversation_id = None;
        guard.conversation_subscription_id = None;
        guard.saw_message_delta = false;
        guard.saw_reasoning_delta = false;
        guard.retry_count = 0;
        guard.retry_exhausted = false;
        previous
    };
    if let Some(previous_conversation_id) = previous_conversation_id {
        if let Err(err) = app_server
            .interrupt_conversation_no_wait(previous_conversation_id)
            .await
        {
            eprintln!("[acp-codex] interruptConversation 失败: {err}");
        }
    }
    if let Some(previous_subscription_id) = previous_subscription_id {
        if let Err(err) = app_server
            .remove_conversation_listener(previous_subscription_id)
            .await
        {
            eprintln!("[acp-codex] removeConversationListener 失败: {err}");
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
    conversation_id: ConversationId,
    event: EventMsg,
    writer: &AcpWriter,
    state: &Arc<Mutex<AcpState>>,
) -> Result<()> {
    // `addConversationListener` is sticky unless explicitly removed; without
    // filtering, events from an old conversation can be mislabeled as the new
    // session and show up in the wrong chat.
    if let EventMsg::SessionConfigured(payload) = &event {
        let mut guard = state.lock().await;
        if let Some(active_conversation_id) = guard.conversation_id {
            if active_conversation_id != conversation_id {
                return Ok(());
            }
        } else {
            guard.conversation_id = Some(conversation_id);
        }
        if guard.session_id.is_none() {
            let session_id_value = payload.session_id.to_string();
            guard.session_id = Some(session_id_value.clone());
            for waiter in guard.session_id_waiters.drain(..) {
                let _ = waiter.send(session_id_value.clone());
            }
        }
        guard.saw_message_delta = false;
        guard.saw_reasoning_delta = false;
        guard.retry_count = 0;
        guard.retry_exhausted = false;
        return Ok(());
    }

    let (active_conversation_id, session_id) = {
        let guard = state.lock().await;
        (guard.conversation_id, guard.session_id.clone())
    };
    if active_conversation_id != Some(conversation_id) {
        return Ok(());
    }

    let Some(session_id) = session_id else {
        return Ok(());
    };

    match event {
        EventMsg::AgentMessageDelta(AgentMessageDeltaEvent { delta }) => {
            {
                let mut guard = state.lock().await;
                guard.saw_message_delta = true;
                guard.retry_count = 0;
                guard.retry_exhausted = false;
            }
            let mut update = update_with_type("agent_message_chunk");
            update.insert("content".to_string(), json!({ "text": delta }));
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::AgentReasoningDelta(AgentReasoningDeltaEvent { delta }) => {
            {
                let mut guard = state.lock().await;
                guard.saw_reasoning_delta = true;
                guard.retry_count = 0;
                guard.retry_exhausted = false;
            }
            let mut update = update_with_type("agent_thought_chunk");
            update.insert("content".to_string(), json!({ "text": delta }));
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::AgentMessage(AgentMessageEvent { message }) => {
            let should_send = {
                let mut guard = state.lock().await;
                let should_send = !guard.saw_message_delta;
                guard.saw_message_delta = true;
                guard.retry_count = 0;
                guard.retry_exhausted = false;
                should_send
            };
            if should_send {
                let mut update = update_with_type("agent_message_chunk");
                update.insert("content".to_string(), json!({ "text": message }));
                send_session_update(writer, &session_id, Value::Object(update)).await?;
            }
        }
        EventMsg::AgentReasoning(AgentReasoningEvent { text }) => {
            let should_send = {
                let mut guard = state.lock().await;
                let should_send = !guard.saw_reasoning_delta;
                guard.saw_reasoning_delta = true;
                guard.retry_count = 0;
                guard.retry_exhausted = false;
                should_send
            };
            if should_send {
                let mut update = update_with_type("agent_thought_chunk");
                update.insert("content".to_string(), json!({ "text": text }));
                send_session_update(writer, &session_id, Value::Object(update)).await?;
            }
        }
        EventMsg::PlanUpdate(UpdatePlanArgs { plan, explanation }) => {
            let entries: Vec<Value> = plan
                .into_iter()
                .map(|item| {
                    let status = match item.status {
                        StepStatus::Pending => "pending",
                        StepStatus::InProgress => "in_progress",
                        StepStatus::Completed => "completed",
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
            if let Some(explanation) = explanation {
                if !explanation.trim().is_empty() {
                    update.insert("explanation".to_string(), Value::String(explanation));
                }
            }
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::ExecCommandBegin(event) => {
            let command_text = event.command.join(" ");
            if !command_text.is_empty() {
                let mut update = update_with_type("tool_call");
                insert_dual(
                    &mut update,
                    "tool_call_id",
                    "toolCallId",
                    Value::String(event.call_id.to_string()),
                );
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
                    Value::String(command_text.clone()),
                );
                insert_dual(
                    &mut update,
                    "status",
                    "status",
                    Value::String("in_progress".to_string()),
                );
                let raw_input = json!({
                    "command": command_text
                });
                insert_dual(&mut update, "raw_input", "rawInput", raw_input);
                send_session_update(writer, &session_id, Value::Object(update)).await?;
            }
        }
        EventMsg::ExecCommandEnd(event) => {
            let output = event.formatted_output;
            let mut update = update_with_type("tool_call_update");
            insert_dual(
                &mut update,
                "tool_call_id",
                "toolCallId",
                Value::String(event.call_id.to_string()),
            );
            let status = if event.exit_code == 0 {
                "completed"
            } else {
                "failed"
            };
            insert_dual(
                &mut update,
                "status",
                "status",
                Value::String(status.to_string()),
            );
            if !output.is_empty() {
                let content = vec![json!({
                    "type": "content",
                    "content": { "text": output }
                })];
                update.insert("content".to_string(), Value::Array(content));
            }
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::McpToolCallBegin(McpToolCallBeginEvent {
            call_id,
            invocation,
        }) => {
            let mut update = update_with_type("tool_call");
            insert_dual(
                &mut update,
                "tool_call_id",
                "toolCallId",
                Value::String(call_id.to_string()),
            );
            let name = format!("mcp:{}:{}", invocation.server, invocation.tool);
            insert_dual(&mut update, "name", "name", Value::String(name));
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
                invocation.arguments.unwrap_or(Value::Null),
            );
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::McpToolCallEnd(McpToolCallEndEvent {
            call_id, result, ..
        }) => {
            let output = match result {
                Ok(value) => format_tool_result(&value),
                Err(err) => err,
            };
            let content = if output.is_empty() {
                None
            } else {
                Some(output)
            };
            send_tool_call_update(
                writer,
                &session_id,
                call_id.to_string(),
                "completed",
                content,
            )
            .await?;
        }
        EventMsg::PatchApplyBegin(PatchApplyBeginEvent {
            call_id, changes, ..
        }) => {
            let mut update = update_with_type("tool_call");
            insert_dual(
                &mut update,
                "tool_call_id",
                "toolCallId",
                Value::String(call_id.to_string()),
            );
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
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::PatchApplyEnd(PatchApplyEndEvent {
            call_id, success, ..
        }) => {
            let mut update = update_with_type("tool_call_update");
            insert_dual(
                &mut update,
                "tool_call_id",
                "toolCallId",
                Value::String(call_id.to_string()),
            );
            let status = if success { "completed" } else { "failed" };
            insert_dual(
                &mut update,
                "status",
                "status",
                Value::String(status.to_string()),
            );
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::WebSearchBegin(WebSearchBeginEvent { call_id }) => {
            let mut update = update_with_type("tool_call");
            insert_dual(
                &mut update,
                "tool_call_id",
                "toolCallId",
                Value::String(call_id.to_string()),
            );
            insert_dual(
                &mut update,
                "name",
                "name",
                Value::String("web_search".to_string()),
            );
            insert_dual(
                &mut update,
                "status",
                "status",
                Value::String("in_progress".to_string()),
            );
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::WebSearchEnd(WebSearchEndEvent { call_id, query }) => {
            send_tool_call_update(
                writer,
                &session_id,
                call_id.to_string(),
                "completed",
                Some(query),
            )
            .await?;
        }
        EventMsg::StreamError(StreamErrorEvent { message, .. }) => {
            handle_error_message(&session_id, message, writer, state).await?;
        }
        EventMsg::Error(ErrorEvent { message, .. }) => {
            handle_error_message(&session_id, message, writer, state).await?;
        }
        EventMsg::TaskComplete(_) => {
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
                .ok_or_else(|| anyhow!("session/new 缺少参数"))?;
            let cwd = normalize_cwd(&params.cwd);
            reset_session_state(state, app_server).await?;
            let mut conversation_params = build_new_conversation_params(config, &cwd)?;
            if let Some(overrides) = build_mcp_overrides(&params.mcp_servers) {
                conversation_params.config = Some(overrides);
            }
            let response = app_server.new_conversation(conversation_params).await?;
            if !params.mcp_servers.is_empty() {
                if let Err(err) = save_mcp_servers(&response.rollout_path, &params.mcp_servers) {
                    eprintln!("[acp-codex] 写入 MCP 会话配置失败: {err}");
                }
            }
            let conversation_id = response.conversation_id;
            let session_id = conversation_id.to_string();
            {
                let mut guard = state.lock().await;
                guard.conversation_id = Some(conversation_id);
                guard.session_id = Some(session_id.clone());
            }
            let subscription = app_server
                .add_conversation_listener(conversation_id)
                .await?;
            {
                let mut guard = state.lock().await;
                guard.conversation_subscription_id = Some(subscription.subscription_id);
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
                .ok_or_else(|| anyhow!("session/load 缺少参数"))?;

            reset_session_state(state, app_server).await?;

            let rollout_path = SessionHandler::find_rollout_file_path(&params.session_id)?;
            let cwd = normalize_cwd(".");
            let mut conversation_params = build_new_conversation_params(config, &cwd)?;
            let stored_mcp_servers = match load_mcp_servers(&rollout_path) {
                Ok(servers) => servers,
                Err(err) => {
                    eprintln!("[acp-codex] 读取 MCP 会话配置失败: {err}");
                    None
                }
            };
            let mcp_servers = if let Some(servers) = stored_mcp_servers {
                servers
            } else if !params.mcp_servers.is_empty() {
                if let Err(err) = save_mcp_servers(&rollout_path, &params.mcp_servers) {
                    eprintln!("[acp-codex] 写入 MCP 会话配置失败: {err}");
                }
                params.mcp_servers.clone()
            } else {
                Vec::new()
            };
            if let Some(overrides) = build_mcp_overrides(&mcp_servers) {
                conversation_params.config = Some(overrides);
            }

            let response = app_server
                .resume_conversation(rollout_path.clone(), conversation_params)
                .await?;
            let conversation_id = response.conversation_id;
            {
                let mut guard = state.lock().await;
                guard.session_id = Some(params.session_id.clone());
                guard.conversation_id = Some(conversation_id);
            }
            let subscription = app_server
                .add_conversation_listener(conversation_id)
                .await?;
            {
                let mut guard = state.lock().await;
                guard.conversation_subscription_id = Some(subscription.subscription_id);
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
                .ok_or_else(|| anyhow!("session/delete 缺少参数"))?;
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
                .ok_or_else(|| anyhow!("session/prompt 缺少参数"))?;

            let (conversation_id, session_id) = {
                let guard = state.lock().await;
                (guard.conversation_id.clone(), guard.session_id.clone())
            };
            let conversation_id = conversation_id.ok_or_else(|| anyhow!("尚未初始化会话"))?;
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

            let mut items = Vec::new();
            for block in params.prompt {
                match block {
                    ContentBlock::Text { text } => {
                        if !text.trim().is_empty() {
                            items.push(InputItem::Text { text });
                        }
                    }
                    ContentBlock::Image { data, mime_type } => {
                        match write_temp_image(&data, &mime_type) {
                            Ok(path) => {
                                items.push(InputItem::LocalImage { path });
                            }
                            Err(err) => {
                                eprintln!("[acp-codex] 无法处理 image block: {err}");
                            }
                        }
                    }
                }
            }
            if items.is_empty() {
                let response = JsonRpcResponseOut {
                    jsonrpc: "2.0",
                    id: request.id,
                    result: json!({ "stopReason": "empty" }),
                };
                writer.send_json(&response).await?;
                return Ok(());
            }

            app_server.send_user_message(conversation_id, items).await?;

            {
                let mut guard = state.lock().await;
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
            let conversation_id = {
                let guard = state.lock().await;
                guard.conversation_id
            };
            if let Some(conversation_id) = conversation_id {
                if let Err(err) = app_server
                    .interrupt_conversation_no_wait(conversation_id)
                    .await
                {
                    eprintln!("[acp-codex] interruptConversation 失败: {err}");
                }
            }
            if let Some(prompt_id) = {
                let mut guard = state.lock().await;
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
                    message: format!("未知方法: {}", request.method),
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
