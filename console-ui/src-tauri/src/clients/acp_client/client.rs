//! ACP client for managing embedded acp-codex and protocol communication.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use acp_codex::CliConfig;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{oneshot, Mutex};
use tokio::task::AbortHandle;
use tokio::time::timeout;

use crate::clients::acp_types::*;
use crate::services::logging::append_log_line;

use super::error::AcpError;

type PendingRequests = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, AcpError>>>>>;

/// ACP client managing the embedded acp-codex loopback.
pub struct AcpClient {
    writer: Arc<Mutex<tokio::io::WriteHalf<tokio::io::DuplexStream>>>,
    pending_requests: PendingRequests,
    request_id: AtomicU64,
    session_id: Mutex<Option<String>>,
    init_result: Mutex<Option<InitializeResult>>,
    mcp_servers: Vec<Value>,
    log_path: PathBuf,
    reader_abort: StdMutex<Option<AbortHandle>>,
    server_abort: StdMutex<Option<AbortHandle>>,
}

impl AcpClient {
    /// Start an embedded acp-codex instance using in-memory transport.
    pub async fn start(
        app_handle: AppHandle,
        log_path: PathBuf,
        config: CliConfig,
        mcp_servers: Vec<Value>,
    ) -> Result<Self, AcpError> {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client_io);
        let (server_read, server_write) = tokio::io::split(server_io);

        let writer = Arc::new(Mutex::new(client_write));
        let pending_requests: PendingRequests = Arc::new(Mutex::new(HashMap::new()));

        let log_path_clone = log_path.clone();
        let server_task = tokio::spawn(async move {
            if let Err(err) = acp_codex::run_with_io(config, server_read, server_write).await {
                log_line(
                    &log_path_clone,
                    &format!("[acp-codex] server exited: {err}"),
                );
            }
        });
        let server_abort = server_task.abort_handle();

        let pending_clone = pending_requests.clone();
        let app_handle_clone = app_handle.clone();
        let log_path_reader = log_path.clone();
        let log_path_reader_err = log_path.clone();
        let writer_clone = writer.clone();
        let reader_task = tokio::spawn(async move {
            if let Err(err) = Self::read_loop(
                client_read,
                writer_clone,
                pending_clone,
                app_handle_clone,
                log_path_reader,
            )
            .await
            {
                log_line(&log_path_reader_err, &format!("[ACP reader] exited: {err}"));
            }
        });
        let reader_abort = reader_task.abort_handle();

        Ok(Self {
            writer,
            pending_requests,
            request_id: AtomicU64::new(1),
            session_id: Mutex::new(None),
            init_result: Mutex::new(None),
            mcp_servers,
            log_path,
            reader_abort: StdMutex::new(Some(reader_abort)),
            server_abort: StdMutex::new(Some(server_abort)),
        })
    }

    /// Read loop for processing messages from acp-codex.
    async fn read_loop(
        reader: tokio::io::ReadHalf<tokio::io::DuplexStream>,
        writer: Arc<Mutex<tokio::io::WriteHalf<tokio::io::DuplexStream>>>,
        pending: PendingRequests,
        app_handle: AppHandle,
        log_path: PathBuf,
    ) -> Result<(), AcpError> {
        log_line(&log_path, "[ACP reader] starting read loop");
        let mut reader = BufReader::new(reader);
        let mut buffer = String::new();
        let mut exit_reason: Option<String> = None;

        loop {
            buffer.clear();
            let bytes = match reader.read_line(&mut buffer).await {
                Ok(size) => size,
                Err(e) => {
                    let reason = format!("read error: {}", e);
                    log_line(&log_path, &format!("[ACP reader] {}", reason));
                    exit_reason = Some(reason);
                    break;
                }
            };

            if bytes == 0 {
                break;
            }

            let line = buffer.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                continue;
            }

            log_line(
                &log_path,
                &format!(
                    "[ACP reader] received line: {}",
                    Self::truncate_log_line(line, 200)
                ),
            );

            let message: AcpMessage = match serde_json::from_str(line) {
                Ok(m) => m,
                Err(e) => {
                    log_line(&log_path, &format!("[ACP reader] parse error: {}", e));
                    continue;
                }
            };

            match message {
                AcpMessage::Request(request) => {
                    log_line(
                        &log_path,
                        &format!("[ACP reader] parsed as Request: {}", request.method),
                    );
                    if request.method == "session/request_permission" {
                        let option_id = pick_permission_option(&request.params);
                        let result = if let Some(option_id) = option_id {
                            json!({ "outcome": "selected", "optionId": option_id })
                        } else {
                            json!({ "outcome": "cancelled" })
                        };
                        if let Err(e) = send_json_response(&writer, request.id, result).await {
                            log_line(
                                &log_path,
                                &format!("[ACP] request_permission response failed: {}", e),
                            );
                        } else {
                            log_line(
                                &log_path,
                                &format!("[ACP] request_permission responded id={}", request.id),
                            );
                        }
                    }
                }
                AcpMessage::Notification(notification) => {
                    log_line(
                        &log_path,
                        &format!(
                            "[ACP reader] parsed as Notification: {}",
                            notification.method
                        ),
                    );
                    Self::handle_notification(&app_handle, &log_path, &notification);
                }
                AcpMessage::Response(response) => {
                    log_line(
                        &log_path,
                        &format!("[ACP reader] parsed as Response, id: {:?}", response.id),
                    );
                    if let Some(id) = response.id {
                        let mut pending_guard = pending.lock().await;
                        if let Some(sender) = pending_guard.remove(&id) {
                            let result = if let Some(error) = response.error {
                                Err(AcpError(format!("{}: {}", error.code, error.message)))
                            } else {
                                Ok(response.result.unwrap_or(Value::Null))
                            };
                            let _ = sender.send(result);
                        } else if let Some(result) = response.result {
                            if let Some(stop_reason) = result.get("stopReason") {
                                log_line(
                                    &log_path,
                                    &format!(
                                        "[ACP reader] emitting completion event: {:?}",
                                        stop_reason
                                    ),
                                );
                                let event = AcpEvent {
                                    event_type: "prompt/complete".to_string(),
                                    payload: result.clone(),
                                };
                                if let Err(e) = app_handle.emit("acp-event", &event) {
                                    log_line(
                                        &log_path,
                                        &format!("[ACP] Failed to emit completion event: {}", e),
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        let reason = exit_reason.unwrap_or_else(|| "stdout closed".to_string());
        log_line(
            &log_path,
            &format!(
                "[ACP reader] exiting; notifying pending requests: {}",
                reason
            ),
        );
        let mut pending_guard = pending.lock().await;
        for (_, sender) in pending_guard.drain() {
            let _ = sender.send(Err(AcpError(format!("ACP reader exited: {}", reason))));
        }
        Ok(())
    }

    fn truncate_log_line(value: &str, max_chars: usize) -> String {
        value.chars().take(max_chars).collect()
    }

    /// Handle incoming notifications from acp-codex.
    fn handle_notification(
        app_handle: &AppHandle,
        log_path: &Path,
        notification: &JsonRpcNotification,
    ) {
        log_line(
            log_path,
            &format!("[ACP] Emitting notification: {}", notification.method),
        );
        let event = AcpEvent {
            event_type: notification.method.clone(),
            payload: notification.params.clone().unwrap_or(Value::Null),
        };
        if let Err(e) = app_handle.emit("acp-event", &event) {
            log_line(log_path, &format!("[ACP] Failed to emit event: {}", e));
        }
    }

    /// Send a JSON-RPC request and wait for response.
    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value, AcpError> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let request = JsonRpcRequest::new(id, method, params);
        let request_json = serde_json::to_string(&request)?;

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending_requests.lock().await;
            pending.insert(id, tx);
        }

        {
            let mut writer = self.writer.lock().await;
            writer.write_all(request_json.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }

        let timeout_duration = request_timeout();
        match timeout(timeout_duration, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                let mut pending = self.pending_requests.lock().await;
                pending.remove(&id);
                Err(AcpError("Request channel closed".into()))
            }
            Err(_) => {
                let mut pending = self.pending_requests.lock().await;
                pending.remove(&id);
                log_line(
                    &self.log_path,
                    &format!(
                        "[ACP] request timeout method={} id={} timeout_secs={}",
                        method,
                        id,
                        timeout_duration.as_secs()
                    ),
                );
                Err(AcpError(format!(
                    "Request timed out after {}s",
                    timeout_duration.as_secs()
                )))
            }
        }
    }

    /// Initialize the ACP connection.
    pub async fn initialize(&self) -> Result<InitializeResult, AcpError> {
        let params = InitializeParams {
            protocol_version: "1".to_string(),
            client_capabilities: ClientCapabilities {
                prompt: Some(PromptCapabilities {
                    embedded_context: Some(true),
                    image: Some(true),
                }),
            },
            client_info: ClientInfo {
                name: "octovalve".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };

        let result = self
            .send_request("initialize", Some(serde_json::to_value(&params)?))
            .await?;
        let init_result: InitializeResult = serde_json::from_value(result)?;

        *self.init_result.lock().await = Some(init_result.clone());
        Ok(init_result)
    }

    /// Authenticate with the agent.
    pub async fn authenticate(&self, method_id: &str) -> Result<(), AcpError> {
        let params = AuthenticateParams {
            method_id: method_id.to_string(),
        };
        self.send_request("authenticate", Some(serde_json::to_value(&params)?))
            .await?;
        Ok(())
    }

    /// Create a new session.
    pub async fn new_session(&self, cwd: &str) -> Result<NewSessionResult, AcpError> {
        let cwd_path = std::path::Path::new(cwd);
        let absolute_cwd = if cwd_path.is_absolute() {
            cwd.to_string()
        } else {
            std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| "/".to_string())
        };

        let params = NewSessionParams {
            cwd: absolute_cwd,
            mcp_servers: self.mcp_servers.clone(),
        };

        let result = self
            .send_request("session/new", Some(serde_json::to_value(&params)?))
            .await?;
        let session_result: NewSessionResult = serde_json::from_value(result)?;

        *self.session_id.lock().await = Some(session_result.session_id.clone());
        Ok(session_result)
    }

    /// Load an existing session.
    pub async fn load_session(&self, session_id: &str) -> Result<LoadSessionResult, AcpError> {
        let params = LoadSessionParams {
            session_id: session_id.to_string(),
            mcp_servers: self.mcp_servers.clone(),
        };

        let result = self
            .send_request("session/load", Some(serde_json::to_value(&params)?))
            .await?;
        let load_result: LoadSessionResult = serde_json::from_value(result)?;

        *self.session_id.lock().await = Some(session_id.to_string());
        Ok(load_result)
    }

    /// List sessions scoped to the workspace.
    pub async fn list_sessions(&self) -> Result<ListSessionsResult, AcpError> {
        let params = ListSessionsParams { cwd: None };
        let result = self
            .send_request("session/list", Some(serde_json::to_value(&params)?))
            .await?;
        let list_result: ListSessionsResult = serde_json::from_value(result)?;
        Ok(list_result)
    }

    /// Delete a session by id.
    pub async fn delete_session(&self, session_id: &str) -> Result<(), AcpError> {
        let params = DeleteSessionParams {
            session_id: session_id.to_string(),
        };
        self.send_request("session/delete", Some(serde_json::to_value(&params)?))
            .await?;
        Ok(())
    }

    /// Send a prompt to the current session (non-blocking).
    /// Content comes via notifications, completion comes via response.
    pub async fn prompt(
        &self,
        prompt: Vec<ContentBlock>,
        context: Option<Vec<ContextItem>>,
    ) -> Result<(), AcpError> {
        let session_id = self
            .session_id
            .lock()
            .await
            .clone()
            .ok_or_else(|| AcpError("No active session".into()))?;

        let params = PromptParams {
            session_id,
            prompt,
            context,
        };

        self.send_request_async("session/prompt", Some(serde_json::to_value(&params)?))
            .await
    }

    /// Send a request without waiting for response (reader loop will handle it).
    async fn send_request_async(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<(), AcpError> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let request = JsonRpcRequest::new(id, method, params);
        let request_json = serde_json::to_string(&request)?;

        let mut writer = self.writer.lock().await;
        writer.write_all(request_json.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
        Ok(())
    }

    /// Cancel the current operation.
    pub async fn cancel(&self) -> Result<(), AcpError> {
        let session_id = self
            .session_id
            .lock()
            .await
            .clone()
            .ok_or_else(|| AcpError("No active session".into()))?;

        let params = CancelParams { session_id };
        self.send_request_async("session/cancel", Some(serde_json::to_value(&params)?))
            .await
    }

    /// Get the current session ID.
    pub async fn get_session_id(&self) -> Option<String> {
        self.session_id.lock().await.clone()
    }

    /// Get the initialization result.
    pub async fn get_init_result(&self) -> Option<InitializeResult> {
        self.init_result.lock().await.clone()
    }

    /// Stop the embedded acp-codex tasks.
    pub async fn stop(&self) {
        log_line(&self.log_path, "[ACP] stop: aborting tasks");

        if let Some(handle) = self.reader_abort.lock().unwrap().take() {
            handle.abort();
        }
        if let Some(handle) = self.server_abort.lock().unwrap().take() {
            handle.abort();
        }

        let mut pending = self.pending_requests.lock().await;
        for (_, sender) in pending.drain() {
            let _ = sender.send(Err(AcpError("ACP stopped".into())));
        }

        log_line(&self.log_path, "[ACP] stop: done");
    }
}

fn pick_permission_option(params: &Option<Value>) -> Option<String> {
    let options = params
        .as_ref()
        .and_then(|value| value.get("options"))
        .and_then(|value| value.as_array())?;
    let mut fallback: Option<String> = None;
    for option in options {
        let option_id = option
            .get("optionId")
            .or_else(|| option.get("option_id"))
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
        if fallback.is_none() {
            fallback = option_id.clone();
        }
        let kind = option
            .get("kind")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if matches!(kind, "allow_once" | "allow_always") {
            return option_id;
        }
    }
    fallback
}

async fn send_json_response(
    writer: &Arc<Mutex<tokio::io::WriteHalf<tokio::io::DuplexStream>>>,
    id: u64,
    result: Value,
) -> Result<(), AcpError> {
    #[derive(serde::Serialize)]
    struct JsonRpcResponseOut {
        jsonrpc: &'static str,
        id: u64,
        result: Value,
    }
    let response = JsonRpcResponseOut {
        jsonrpc: "2.0",
        id,
        result,
    };
    let response_json = serde_json::to_string(&response)?;
    let mut guard = writer.lock().await;
    guard.write_all(response_json.as_bytes()).await?;
    guard.write_all(b"\n").await?;
    guard.flush().await?;
    Ok(())
}

fn log_line(log_path: &Path, message: &str) {
    if let Some(parent) = log_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = append_log_line(log_path, message);
}

fn request_timeout() -> Duration {
    Duration::from_secs(30)
}

impl Drop for AcpClient {
    fn drop(&mut self) {
        if let Some(handle) = self.reader_abort.lock().unwrap().take() {
            handle.abort();
        }
        if let Some(handle) = self.server_abort.lock().unwrap().take() {
            handle.abort();
        }
    }
}

/// State wrapper for Tauri.
pub struct AcpClientState(pub Mutex<Option<Arc<AcpClient>>>);

impl Default for AcpClientState {
    fn default() -> Self {
        Self(Mutex::new(None))
    }
}
