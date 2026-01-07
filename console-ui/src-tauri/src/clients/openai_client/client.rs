use futures_util::StreamExt;
use reqwest::redirect::Policy;
use reqwest::Client;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use tokio::sync::{watch, Mutex};

use crate::services::http_utils::join_base_path;
use crate::services::logging::append_log_line;

use super::types::{
    ChatMessage, ChatMessageContent, ChatMessageContentPart, ChatStreamEvent, FunctionCall,
    OpenAiConfig, Tool, ToolCall,
};

pub struct OpenAiClient {
    config: OpenAiConfig,
    http_client: Client,
    messages: Arc<Mutex<Vec<ChatMessage>>>,
    tools: Arc<Mutex<Vec<Tool>>>,
    cancel_tx: watch::Sender<u64>,
    log_path: PathBuf,
    runtime_system_prompt: Option<String>,
}

impl OpenAiClient {
    pub fn new(
        config: OpenAiConfig,
        log_path: PathBuf,
        runtime_system_prompt: Option<String>,
    ) -> Self {
        let (cancel_tx, _cancel_rx) = watch::channel(0u64);
        let http_client = match build_http_client() {
            Ok(client) => {
                let _ = append_log_line(
                    &log_path,
                    "[openai_client] reqwest configured http1_only=true redirect=none pool_idle=0",
                );
                client
            }
            Err(err) => {
                let _ = append_log_line(
                    &log_path,
                    &format!(
                        "[openai_client] reqwest build failed: {}; falling back to default",
                        err
                    ),
                );
                Client::new()
            }
        };
        Self {
            config,
            http_client,
            messages: Arc::new(Mutex::new(Vec::new())),
            tools: Arc::new(Mutex::new(Vec::new())),
            cancel_tx,
            log_path,
            runtime_system_prompt,
        }
    }

    pub fn update_config(&mut self, config: OpenAiConfig) {
        self.config = config;
    }

    pub async fn set_tools(&self, tools: Vec<Tool>) {
        let mut guard = self.tools.lock().await;
        *guard = tools;
    }

    pub async fn add_message(&self, message: ChatMessage) {
        let mut guard = self.messages.lock().await;
        guard.push(message);
    }

    pub async fn clear_messages(&self) {
        let mut guard = self.messages.lock().await;
        guard.clear();
    }

    pub fn cancel(&self) {
        let next = *self.cancel_tx.borrow() + 1;
        let _ = self.cancel_tx.send(next);
    }

    pub async fn send_stream(&self, app_handle: &AppHandle) -> Result<(), String> {
        let mut cancel_rx = self.cancel_tx.subscribe();
        let start_seq = *cancel_rx.borrow();

        let messages = self.messages.lock().await.clone();
        let messages = self.inject_runtime_system_prompt(messages);
        let tools = self.tools.lock().await.clone();

        let url = join_base_path(&self.config.base_url, &self.config.chat_path).map_err(|err| {
            self.log_line(&format!("[openai_send] invalid url: {err}"));
            err
        })?;
        self.log_line(&format!(
            "[openai_send] url={} model={} messages={} tools={}",
            url,
            self.config.model,
            messages.len(),
            tools.len()
        ));
        for (idx, msg) in messages.iter().enumerate() {
            let (content_kind, content_parts, content_len) = match &msg.content {
                ChatMessageContent::Text(text) => ("text", 0usize, text.len()),
                ChatMessageContent::Parts(parts) => {
                    let mut total_len = 0usize;
                    for part in parts {
                        match part {
                            ChatMessageContentPart::Text { text } => total_len += text.len(),
                            ChatMessageContentPart::ImageUrl { image_url } => {
                                total_len += image_url.url.len()
                            }
                        }
                    }
                    ("parts", parts.len(), total_len)
                }
            };
            let tool_calls_len = msg.tool_calls.as_ref().map(|v| v.len()).unwrap_or(0);
            self.log_line(&format!(
                "[openai_send] msg[{}] role={} content_kind={} content_parts={} content_len={} tool_calls={}",
                idx, msg.role, content_kind, content_parts, content_len, tool_calls_len
            ));
        }

        let mut body = json!({
            "model": self.config.model,
            "messages": messages,
            "stream": true,
        });

        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }

        let body_json = serde_json::to_string(&body).unwrap_or_default();
        self.log_line(&format!("[openai_send] body_len={}", body_json.len()));

        self.log_line("[openai_send] reqwest request start http1_only=true redirect=none");
        let request = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .header("Accept", "*/*")
            .header("Accept-Encoding", "identity")
            .header("Connection", "close")
            .header("User-Agent", "octovalve-console")
            .body(body_json.clone())
            .build()
            .map_err(|e| format!("Request build failed: {}", e))?;
        let started = Instant::now();
        let response = match self.http_client.execute(request).await {
            Ok(response) => response,
            Err(err) => {
                self.log_line(&format!(
                    "[openai_send] reqwest error is_timeout={} is_connect={} status={}",
                    err.is_timeout(),
                    err.is_connect(),
                    err.status()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "none".to_string())
                ));
                self.log_line(&format!("[openai_send] reqwest error={:?}", err));
                return Err(format!("Request failed: {}", err));
            }
        };
        let elapsed_ms = started.elapsed().as_millis();
        let version = format!("{:?}", response.version());
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("unknown");
        let content_encoding = response
            .headers()
            .get(reqwest::header::CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("none");
        let transfer_encoding = response
            .headers()
            .get(reqwest::header::TRANSFER_ENCODING)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("none");
        let server = response
            .headers()
            .get("server")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("unknown");
        let via = response
            .headers()
            .get("via")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("none");
        self.log_line(&format!(
            "[openai_send] reqwest response status={} elapsed_ms={} version={} content_type={} content_encoding={} transfer_encoding={} server={} via={}",
            response.status(),
            elapsed_ms,
            version,
            content_type,
            content_encoding,
            transfer_encoding,
            server,
            via
        ));

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            let body_preview: String = body_json.chars().take(1024).collect();
            self.log_line(&format!(
                "[openai_send] status={} body_len={} body_preview={}",
                status,
                text.len(),
                body_preview
            ));
            self.log_line(&format!("[openai_send] response_body={}", text));
            return Err(format!("API error {}: {}", status, text));
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut full_content = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        loop {
            tokio::select! {
                _ = cancel_rx.changed() => {
                    if *cancel_rx.borrow() != start_seq {
                        emit_cancelled_event(app_handle);
                        return Ok(());
                    }
                }
                maybe_chunk = stream.next() => {
                    let Some(chunk) = maybe_chunk else {
                        break;
                    };
                    let chunk = chunk.map_err(|e| format!("Stream error: {}", e))?;
                    buffer.push_str(&String::from_utf8_lossy(&chunk));

                    // Process SSE lines
                    while let Some(newline_pos) = buffer.find('\n') {
                        let line = buffer[..newline_pos].trim().to_string();
                        buffer = buffer[newline_pos + 1..].to_string();

                        if line.is_empty() || line.starts_with(':') {
                            continue;
                        }

                        if let Some(data) = strip_sse_data_prefix(&line) {
                            let done = self
                                .handle_sse_data(
                                    app_handle,
                                    data,
                                    &mut full_content,
                                    &mut tool_calls,
                                )
                                .await?;
                            if done {
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn inject_runtime_system_prompt(&self, mut messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
        let Some(prompt) = self.runtime_system_prompt.as_ref() else {
            return messages;
        };
        let trimmed = prompt.trim();
        if trimmed.is_empty() {
            return messages;
        }
        let already_present = messages.first().map_or(false, |message| {
            if message.role != "system" {
                return false;
            }
            match &message.content {
                ChatMessageContent::Text(content) => content.trim() == trimmed,
                ChatMessageContent::Parts(_) => false,
            }
        });
        if !already_present {
            messages.insert(
                0,
                ChatMessage {
                    role: "system".to_string(),
                    content: ChatMessageContent::Text(trimmed.to_string()),
                    tool_calls: None,
                    tool_call_id: None,
                },
            );
        }
        messages
    }

    async fn handle_sse_data(
        &self,
        app_handle: &AppHandle,
        data: &str,
        full_content: &mut String,
        tool_calls: &mut Vec<ToolCall>,
    ) -> Result<bool, String> {
        if data == "[DONE]"
            || data.contains("\"finish_reason\":\"stop\"")
            || data.contains("\"finish_reason\": \"stop\"")
        {
            emit_complete_event(app_handle, tool_calls);
            self.maybe_store_assistant_message(full_content, tool_calls)
                .await;
            return Ok(true);
        }

        if let Ok(chunk_data) = serde_json::from_str::<Value>(data) {
            if let Some(choices) = chunk_data.get("choices").and_then(|c| c.as_array()) {
                if let Some(choice) = choices.first() {
                    if let Some(delta) = choice.get("delta") {
                        if let Some(reasoning) =
                            delta.get("reasoning_content").and_then(|c| c.as_str())
                        {
                            let event = ChatStreamEvent {
                                event_type: "reasoning".to_string(),
                                content: Some(reasoning.to_string()),
                                tool_calls: None,
                                finish_reason: None,
                                error: None,
                            };
                            let _ = app_handle.emit("openai-stream", &event);
                        }
                        if let Some(reasoning) = delta.get("reasoning").and_then(|c| c.as_str()) {
                            let event = ChatStreamEvent {
                                event_type: "reasoning".to_string(),
                                content: Some(reasoning.to_string()),
                                tool_calls: None,
                                finish_reason: None,
                                error: None,
                            };
                            let _ = app_handle.emit("openai-stream", &event);
                        }

                        // Content delta
                        if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                            full_content.push_str(content);
                            let event = ChatStreamEvent {
                                event_type: "content".to_string(),
                                content: Some(content.to_string()),
                                tool_calls: None,
                                finish_reason: None,
                                error: None,
                            };
                            let _ = app_handle.emit("openai-stream", &event);
                        }

                        // Tool calls delta
                        if let Some(tc) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                            for tc_delta in tc {
                                let index =
                                    tc_delta.get("index").and_then(|i| i.as_u64()).unwrap_or(0)
                                        as usize;

                                // Ensure tool_calls has enough elements
                                while tool_calls.len() <= index {
                                    tool_calls.push(ToolCall {
                                        id: String::new(),
                                        call_type: "function".to_string(),
                                        function: FunctionCall {
                                            name: String::new(),
                                            arguments: String::new(),
                                        },
                                    });
                                }

                                if let Some(id) = tc_delta.get("id").and_then(|i| i.as_str()) {
                                    tool_calls[index].id = id.to_string();
                                }
                                if let Some(func) = tc_delta.get("function") {
                                    if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                                        tool_calls[index].function.name = name.to_string();
                                    }
                                    if let Some(args) =
                                        func.get("arguments").and_then(|a| a.as_str())
                                    {
                                        tool_calls[index].function.arguments.push_str(args);
                                    }
                                }
                            }
                        }
                    }

                    // Check finish reason
                    if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                        if reason == "tool_calls" {
                            self.maybe_store_assistant_message(full_content, tool_calls)
                                .await;
                            emit_tool_calls_event(app_handle, tool_calls, reason);
                            return Ok(true);
                        }
                    }
                }
            }
        }
        Ok(false)
    }

    async fn maybe_store_assistant_message(&self, full_content: &str, tool_calls: &[ToolCall]) {
        if full_content.is_empty() && tool_calls.is_empty() {
            return;
        }
        let mut msgs = self.messages.lock().await;
        msgs.push(ChatMessage {
            role: "assistant".to_string(),
            content: ChatMessageContent::Text(full_content.to_string()),
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls.to_vec())
            },
            tool_call_id: None,
        });
    }

    fn log_line(&self, message: &str) {
        let _ = append_log_line(&self.log_path, message);
    }
}

fn emit_cancelled_event(app_handle: &AppHandle) {
    let event = ChatStreamEvent {
        event_type: "cancelled".to_string(),
        content: None,
        tool_calls: None,
        finish_reason: Some("cancelled".to_string()),
        error: None,
    };
    let _ = app_handle.emit("openai-stream", &event);
}

fn emit_complete_event(app_handle: &AppHandle, tool_calls: &[ToolCall]) {
    let event = ChatStreamEvent {
        event_type: "complete".to_string(),
        content: None,
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls.to_vec())
        },
        finish_reason: Some("stop".to_string()),
        error: None,
    };
    let _ = app_handle.emit("openai-stream", &event);
}

fn emit_tool_calls_event(app_handle: &AppHandle, tool_calls: &[ToolCall], reason: &str) {
    let event = ChatStreamEvent {
        event_type: "tool_calls".to_string(),
        content: None,
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls.to_vec())
        },
        finish_reason: Some(reason.to_string()),
        error: None,
    };
    let _ = app_handle.emit("openai-stream", &event);
}

fn strip_sse_data_prefix(line: &str) -> Option<&str> {
    if let Some(data) = line.strip_prefix("data: ") {
        return Some(data);
    }
    line.strip_prefix("data:").map(|value| value.trim_start())
}

fn build_http_client() -> Result<Client, reqwest::Error> {
    Client::builder()
        .http1_only()
        .redirect(Policy::none())
        .pool_max_idle_per_host(0)
        .build()
}

// Global client state
pub struct OpenAiClientState(pub Mutex<Option<Arc<OpenAiClient>>>);
