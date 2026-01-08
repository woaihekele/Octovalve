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
};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::app_server::AppServerClient;
use crate::cli::CliConfig;
use crate::protocol::{
    AuthenticateParamsInput, CancelParamsInput, ContentBlock, InitializeParamsInput,
    JsonRpcErrorOut, JsonRpcErrorOutPayload, JsonRpcIncomingRequest, JsonRpcResponseOut,
    LoadSessionParamsInput, NewSessionParamsInput, PromptParamsInput,
};
use crate::state::AcpState;
use crate::utils::{
    SessionHandler, build_new_conversation_params, insert_dual, load_rollout_history,
    normalize_cwd, update_with_type, write_temp_image,
};
use crate::writer::AcpWriter;

pub(crate) async fn handle_codex_event(
    event: EventMsg,
    writer: &AcpWriter,
    state: &Arc<Mutex<AcpState>>,
) -> Result<()> {
    match event {
        EventMsg::SessionConfigured(payload) => {
            let session_id_value = payload.session_id.to_string();
            let mut guard = state.lock().await;
            guard.session_id = Some(session_id_value.clone());
            guard.saw_message_delta = false;
            guard.saw_reasoning_delta = false;
            for waiter in guard.session_id_waiters.drain(..) {
                let _ = waiter.send(session_id_value.clone());
            }
            return Ok(());
        }
        _ => {}
    }

    let session_id = {
        let guard = state.lock().await;
        guard.session_id.clone()
    };
    let Some(session_id) = session_id else {
        return Ok(());
    };

    match event {
        EventMsg::AgentMessageDelta(AgentMessageDeltaEvent { delta }) => {
            {
                let mut guard = state.lock().await;
                guard.saw_message_delta = true;
            }
            let mut update = update_with_type("agent_message_chunk");
            update.insert("content".to_string(), json!({ "text": delta }));
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::AgentReasoningDelta(AgentReasoningDeltaEvent { delta }) => {
            {
                let mut guard = state.lock().await;
                guard.saw_reasoning_delta = true;
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
        EventMsg::McpToolCallBegin(McpToolCallBeginEvent { call_id, invocation }) => {
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
        EventMsg::McpToolCallEnd(McpToolCallEndEvent { call_id, result, .. }) => {
            let output = match result {
                Ok(value) => serde_json::to_string(&value).unwrap_or_default(),
                Err(err) => err,
            };
            let mut update = update_with_type("tool_call_update");
            insert_dual(
                &mut update,
                "tool_call_id",
                "toolCallId",
                Value::String(call_id.to_string()),
            );
            insert_dual(
                &mut update,
                "status",
                "status",
                Value::String("completed".to_string()),
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
        EventMsg::PatchApplyBegin(PatchApplyBeginEvent { call_id, changes, .. }) => {
            let mut update = update_with_type("tool_call");
            insert_dual(
                &mut update,
                "tool_call_id",
                "toolCallId",
                Value::String(call_id.to_string()),
            );
            insert_dual(&mut update, "name", "name", Value::String("edit".to_string()));
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
        EventMsg::PatchApplyEnd(PatchApplyEndEvent { call_id, success, .. }) => {
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
            let mut update = update_with_type("tool_call_update");
            insert_dual(
                &mut update,
                "tool_call_id",
                "toolCallId",
                Value::String(call_id.to_string()),
            );
            insert_dual(
                &mut update,
                "status",
                "status",
                Value::String("completed".to_string()),
            );
            let content = vec![json!({
                "type": "content",
                "content": { "text": query }
            })];
            update.insert("content".to_string(), Value::Array(content));
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::StreamError(StreamErrorEvent { message, .. }) => {
            let mut update = update_with_type("error");
            update.insert("error".to_string(), json!({ "message": message }));
            send_session_update(writer, &session_id, Value::Object(update)).await?;
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
        }
        EventMsg::Error(ErrorEvent { message, .. }) => {
            let mut update = update_with_type("error");
            update.insert("error".to_string(), json!({ "message": message }));
            send_session_update(writer, &session_id, Value::Object(update)).await?;
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
        }
        EventMsg::TaskComplete(_) => {
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
            {
                let mut guard = state.lock().await;
                guard.session_id = None;
                guard.pending_prompt_ids.clear();
                guard.conversation_id = None;
                guard.saw_message_delta = false;
                guard.saw_reasoning_delta = false;
            }
            let conversation_params = build_new_conversation_params(config, &cwd)?;
            let response = app_server.new_conversation(conversation_params).await?;
            let conversation_id = response.conversation_id;
            app_server
                .add_conversation_listener(conversation_id.clone())
                .await?;

            let session_id = {
                let mut guard = state.lock().await;
                guard.conversation_id = Some(conversation_id);
                let session_id = guard
                    .session_id
                    .clone()
                    .unwrap_or_else(|| conversation_id.to_string());
                guard.session_id = Some(session_id.clone());
                session_id
            };

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
        "session/load" => {
            let params: LoadSessionParamsInput = request
                .params
                .as_ref()
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()?
                .ok_or_else(|| anyhow!("session/load 缺少参数"))?;

            {
                let mut guard = state.lock().await;
                guard.session_id = None;
                guard.pending_prompt_ids.clear();
                guard.conversation_id = None;
                guard.saw_message_delta = false;
                guard.saw_reasoning_delta = false;
            }

            let rollout_path = SessionHandler::find_rollout_file_path(&params.session_id)?;
            let cwd = normalize_cwd(".");
            let conversation_params = build_new_conversation_params(config, &cwd)?;

            let response = app_server
                .resume_conversation(rollout_path.clone(), conversation_params)
                .await?;
            let conversation_id = response.conversation_id;
            app_server
                .add_conversation_listener(conversation_id.clone())
                .await?;

            let history = load_rollout_history(&rollout_path).await.unwrap_or_default();

            {
                let mut guard = state.lock().await;
                guard.session_id = Some(params.session_id.clone());
                guard.conversation_id = Some(conversation_id);
            }

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

async fn send_session_update(
    writer: &AcpWriter,
    session_id: &str,
    update: Value,
) -> Result<()> {
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
