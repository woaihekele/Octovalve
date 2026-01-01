use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default = "default_chat_path")]
    pub chat_path: String,
}

fn default_chat_path() -> String {
    "/v1/chat/completions".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: ToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStreamEvent {
    pub event_type: String,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub finish_reason: Option<String>,
    pub error: Option<String>,
}

pub struct OpenAiClient {
    config: OpenAiConfig,
    http_client: Client,
    messages: Arc<Mutex<Vec<ChatMessage>>>,
    tools: Arc<Mutex<Vec<Tool>>>,
}

impl OpenAiClient {
    pub fn new(config: OpenAiConfig) -> Self {
        Self {
            config,
            http_client: Client::new(),
            messages: Arc::new(Mutex::new(Vec::new())),
            tools: Arc::new(Mutex::new(Vec::new())),
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

    pub async fn send_stream(&self, app_handle: &AppHandle) -> Result<(), String> {
        let messages = self.messages.lock().await.clone();
        let tools = self.tools.lock().await.clone();

        let url = format!(
            "{}{}",
            self.config.base_url.trim_end_matches('/'),
            self.config.chat_path
        );

        let mut body = json!({
            "model": self.config.model,
            "messages": messages,
            "stream": true,
        });

        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }

        let response = self
            .http_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(format!("API error {}: {}", status, text));
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut full_content = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("Stream error: {}", e))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Process SSE lines
            while let Some(newline_pos) = buffer.find('\n') {
                let line = buffer[..newline_pos].trim().to_string();
                buffer = buffer[newline_pos + 1..].to_string();

                if line.is_empty() || line.starts_with(':') {
                    continue;
                }

                if let Some(data) = line.strip_prefix("data: ") {
                    // Check for [DONE] or finish_reason
                    if data == "[DONE]" || data.contains("\"finish_reason\":\"stop\"") || data.contains("\"finish_reason\": \"stop\"") {
                        eprintln!("[OpenAI] Stream end detected: {}", data);
                        // Stream complete
                        let event = ChatStreamEvent {
                            event_type: "complete".to_string(),
                            content: None,
                            tool_calls: if tool_calls.is_empty() {
                                None
                            } else {
                                Some(tool_calls.clone())
                            },
                            finish_reason: Some("stop".to_string()),
                            error: None,
                        };
                        let _ = app_handle.emit("openai-stream", &event);

                        // Add assistant message to history
                        if !full_content.is_empty() || !tool_calls.is_empty() {
                            let mut msgs = self.messages.lock().await;
                            msgs.push(ChatMessage {
                                role: "assistant".to_string(),
                                content: full_content.clone(),
                                tool_calls: if tool_calls.is_empty() {
                                    None
                                } else {
                                    Some(tool_calls.clone())
                                },
                                tool_call_id: None,
                            });
                        }
                        return Ok(());
                    }

                    if let Ok(chunk_data) = serde_json::from_str::<Value>(data) {
                        if let Some(choices) = chunk_data.get("choices").and_then(|c| c.as_array()) {
                            if let Some(choice) = choices.first() {
                                if let Some(delta) = choice.get("delta") {
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
                                            let index = tc_delta.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                                            
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
                                                if let Some(args) = func.get("arguments").and_then(|a| a.as_str()) {
                                                    tool_calls[index].function.arguments.push_str(args);
                                                }
                                            }
                                        }
                                    }
                                }

                                // Check finish reason
                                if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
                                    if reason == "tool_calls" {
                                        let event = ChatStreamEvent {
                                            event_type: "tool_calls".to_string(),
                                            content: None,
                                            tool_calls: Some(tool_calls.clone()),
                                            finish_reason: Some(reason.to_string()),
                                            error: None,
                                        };
                                        let _ = app_handle.emit("openai-stream", &event);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

// Global client state
pub struct OpenAiClientState(pub Mutex<Option<OpenAiClient>>);
