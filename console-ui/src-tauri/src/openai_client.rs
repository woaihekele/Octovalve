use futures_util::StreamExt;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{watch, Mutex};
use tokio::time::{timeout, Duration};

const OPENAI_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const OPENAI_HTTP_IO_TIMEOUT: Duration = Duration::from_secs(120);
const OPENAI_RAW_CANCELLED: &str = "openai_http_cancelled";

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
    cancel_tx: watch::Sender<u64>,
}

impl OpenAiClient {
    pub fn new(config: OpenAiConfig) -> Self {
        let (cancel_tx, _cancel_rx) = watch::channel(0u64);
        Self {
            config,
            http_client: Client::new(),
            messages: Arc::new(Mutex::new(Vec::new())),
            tools: Arc::new(Mutex::new(Vec::new())),
            cancel_tx,
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
        let tools = self.tools.lock().await.clone();

        let url = format!(
            "{}{}",
            self.config.base_url.trim_end_matches('/'),
            self.config.chat_path
        );
        let parsed_url = Url::parse(&url).map_err(|e| format!("Invalid url: {}", e))?;
        let use_raw_http = should_use_raw_http(&parsed_url);

        eprintln!(
            "[openai_send] url={} model={} messages={} tools={}",
            url,
            self.config.model,
            messages.len(),
            tools.len()
        );
        for (idx, msg) in messages.iter().enumerate() {
            let content_len = msg.content.len();
            let tool_calls_len = msg.tool_calls.as_ref().map(|v| v.len()).unwrap_or(0);
            eprintln!(
                "[openai_send] msg[{}] role={} content_len={} tool_calls={}",
                idx,
                msg.role,
                content_len,
                tool_calls_len
            );
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
        eprintln!("[openai_send] body_len={}", body_json.len());

        if use_raw_http {
            if let Some(host) = parsed_url.host_str() {
                eprintln!("[openai_send] raw_http host={} port={}", host, parsed_url.port_or_known_default().unwrap_or(0));
            }
            let response = match raw_http_request_with_timeout(
                "POST",
                &parsed_url,
                Some(&body_json),
                &self.config.api_key,
                &mut cancel_rx,
                start_seq,
            )
            .await
            {
                Ok(response) => response,
                Err(err) if err == OPENAI_RAW_CANCELLED => {
                    emit_cancelled_event(app_handle);
                    return Ok(());
                }
                Err(err) => return Err(err),
            };

            if response.status / 100 != 2 {
                let text = String::from_utf8_lossy(&response.body).to_string();
                let body_preview: String = body_json.chars().take(1024).collect();
                eprintln!(
                    "[openai_send] status={} body_len={} body_preview={}",
                    response.status,
                    text.len(),
                    body_preview
                );
                eprintln!("[openai_send] response_body={}", text);
                return Err(format!("API error {}: {}", response.status, text));
            }

            let text = String::from_utf8_lossy(&response.body).to_string();
            return self.process_sse_body(app_handle, &text).await;
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
            let body_preview: String = body_json.chars().take(1024).collect();
            eprintln!(
                "[openai_send] status={} body_len={} body_preview={}",
                status,
                text.len(),
                body_preview
            );
            eprintln!("[openai_send] response_body={}", text);
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
                            let done = self.handle_sse_data(app_handle, data, &mut full_content, &mut tool_calls).await?;
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

    async fn process_sse_body(&self, app_handle: &AppHandle, body: &str) -> Result<(), String> {
        let mut full_content = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut saw_data = false;

        for raw_line in body.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with(':') {
                continue;
            }
            if let Some(data) = strip_sse_data_prefix(line) {
                saw_data = true;
                let done = self.handle_sse_data(app_handle, data, &mut full_content, &mut tool_calls).await?;
                if done {
                    return Ok(());
                }
            }
        }

        if !saw_data {
            return Err("OpenAI stream response missing data".to_string());
        }
        Ok(())
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
            self.maybe_store_assistant_message(full_content, tool_calls).await;
            return Ok(true);
        }

        if let Ok(chunk_data) = serde_json::from_str::<Value>(data) {
            if let Some(choices) = chunk_data.get("choices").and_then(|c| c.as_array()) {
                if let Some(choice) = choices.first() {
                    if let Some(delta) = choice.get("delta") {
                        if let Some(reasoning) = delta.get("reasoning_content").and_then(|c| c.as_str()) {
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
                                let index = tc_delta
                                    .get("index")
                                    .and_then(|i| i.as_u64())
                                    .unwrap_or(0) as usize;

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
                            emit_tool_calls_event(app_handle, tool_calls, reason);
                            self.maybe_store_assistant_message(full_content, tool_calls).await;
                            return Ok(true);
                        }
                    }
                }
            }
        }
        Ok(false)
    }

    async fn maybe_store_assistant_message(
        &self,
        full_content: &str,
        tool_calls: &[ToolCall],
    ) {
        if full_content.is_empty() && tool_calls.is_empty() {
            return;
        }
        let mut msgs = self.messages.lock().await;
        msgs.push(ChatMessage {
            role: "assistant".to_string(),
            content: full_content.to_string(),
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls.to_vec())
            },
            tool_call_id: None,
        });
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
    line.strip_prefix("data:")
        .map(|value| value.trim_start())
}

fn should_use_raw_http(url: &Url) -> bool {
    if url.scheme() != "http" {
        return false;
    }
    matches!(url.host_str(), Some("localhost") | Some("127.0.0.1") | Some("::1"))
}

struct RawHttpResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

async fn raw_http_request_with_timeout(
    method: &str,
    url: &Url,
    body: Option<&str>,
    api_key: &str,
    cancel_rx: &mut watch::Receiver<u64>,
    start_seq: u64,
) -> Result<RawHttpResponse, String> {
    let host = url
        .host_str()
        .ok_or_else(|| "openai http missing host".to_string())?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| "openai http missing port".to_string())?;
    let addr = format!("{}:{}", host, port);
    let path = match url.query() {
        Some(query) => format!("{}?{}", url.path(), query),
        None => url.path().to_string(),
    };

    let mut stream = timeout(OPENAI_HTTP_CONNECT_TIMEOUT, TcpStream::connect(&addr))
        .await
        .map_err(|_| "openai http connect timed out".to_string())?
        .map_err(|err| err.to_string())?;

    let host_header = if (url.scheme() == "http" && port == 80)
        || (url.scheme() == "https" && port == 443)
    {
        host.to_string()
    } else {
        format!("{}:{}", host, port)
    };

    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host_header}\r\nAccept: */*\r\nConnection: close\r\n"
    );
    if !api_key.is_empty() {
        request.push_str(&format!("Authorization: Bearer {}\r\n", api_key));
    }
    if let Some(body) = body {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
        request.push_str("\r\n");
        request.push_str(body);
    } else {
        request.push_str("\r\n");
    }

    timeout(OPENAI_HTTP_IO_TIMEOUT, stream.write_all(request.as_bytes()))
        .await
        .map_err(|_| "openai http write timed out".to_string())?
        .map_err(|err| err.to_string())?;

    let buffer = read_to_end_with_cancel(&mut stream, cancel_rx, start_seq).await?;
    let (status, headers, body) = parse_http_response_bytes(&buffer)?;
    Ok(RawHttpResponse { status, headers, body })
}

async fn read_to_end_with_cancel(
    stream: &mut TcpStream,
    cancel_rx: &mut watch::Receiver<u64>,
    start_seq: u64,
) -> Result<Vec<u8>, String> {
    let mut buffer = Vec::new();
    let mut chunk = vec![0u8; 8192];
    loop {
        tokio::select! {
            _ = cancel_rx.changed() => {
                if *cancel_rx.borrow() != start_seq {
                    return Err(OPENAI_RAW_CANCELLED.to_string());
                }
            }
            read_res = timeout(OPENAI_HTTP_IO_TIMEOUT, stream.read(&mut chunk)) => {
                let read_res = read_res.map_err(|_| "openai http read timed out".to_string())?;
                let n = read_res.map_err(|err| err.to_string())?;
                if n == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..n]);
            }
        }
    }
    Ok(buffer)
}

fn parse_http_response_bytes(bytes: &[u8]) -> Result<(u16, HashMap<String, String>, Vec<u8>), String> {
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "openai http response missing header".to_string())?;
    let head = String::from_utf8_lossy(&bytes[..header_end]);
    let body_bytes = &bytes[(header_end + 4)..];
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_lowercase(), value.trim().to_string());
        }
    }
    let body = if headers
        .get("transfer-encoding")
        .map(|value| value.to_lowercase().contains("chunked"))
        .unwrap_or(false)
    {
        decode_chunked_body(body_bytes)?
    } else {
        body_bytes.to_vec()
    };
    Ok((status, headers, body))
}

fn decode_chunked_body(body: &[u8]) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut index = 0usize;
    while index < body.len() {
        let line_end = find_crlf(body, index)
            .ok_or_else(|| "openai http chunked response missing size line".to_string())?;
        let line = String::from_utf8_lossy(&body[index..line_end]);
        let size_str = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_str, 16)
            .map_err(|_| "openai http chunked size parse failed".to_string())?;
        index = line_end + 2;
        if size == 0 {
            break;
        }
        if index + size > body.len() {
            return Err("openai http chunked body truncated".to_string());
        }
        output.extend_from_slice(&body[index..index + size]);
        index += size;
        if index + 2 > body.len() || &body[index..index + 2] != b"\r\n" {
            return Err("openai http chunked body missing terminator".to_string());
        }
        index += 2;
    }
    Ok(output)
}

fn find_crlf(body: &[u8], start: usize) -> Option<usize> {
    body[start..]
        .windows(2)
        .position(|window| window == b"\r\n")
        .map(|offset| start + offset)
}

// Global client state
pub struct OpenAiClientState(pub Mutex<Option<Arc<OpenAiClient>>>);
