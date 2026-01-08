//! ACP client for managing codex-acp subprocess and protocol communication.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::clients::acp_types::*;
use crate::services::console_sidecar::build_console_path;
use crate::services::logging::append_log_line;

use super::error::AcpError;

type PendingRequests = Arc<Mutex<HashMap<u64, mpsc::Sender<Result<Value, AcpError>>>>>;

/// ACP client managing the codex-acp subprocess.
pub struct AcpClient {
    process: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    reader_handle: Option<std::thread::JoinHandle<()>>,
    request_id: AtomicU64,
    pending_requests: PendingRequests,
    session_id: Mutex<Option<String>>,
    init_result: Mutex<Option<InitializeResult>>,
    mcp_servers: Vec<Value>,
    log_path: PathBuf,
}

impl AcpClient {
    /// Start a new codex-acp process.
    pub fn start(
        codex_acp_path: &PathBuf,
        app_handle: AppHandle,
        log_path: PathBuf,
        acp_args: Vec<String>,
        mcp_servers: Vec<Value>,
    ) -> Result<Self, AcpError> {
        let mut command = Command::new(codex_acp_path);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .env("PATH", build_console_path());
        if !acp_args.is_empty() {
            command.args(acp_args);
        }
        if std::env::var_os("HOME").is_none() {
            if let Ok(home) = app_handle.path().home_dir() {
                command.env("HOME", home);
            }
        }
        let mut process = command
            .spawn()
            .map_err(|e| AcpError(format!("Failed to start ACP agent: {}", e)))?;

        let stdin = process
            .stdin
            .take()
            .ok_or_else(|| AcpError("Failed to get stdin".into()))?;
        let stdout = process
            .stdout
            .take()
            .ok_or_else(|| AcpError("Failed to get stdout".into()))?;

        let stdin = Arc::new(Mutex::new(stdin));
        let pending_requests: PendingRequests = Arc::new(Mutex::new(HashMap::new()));

        // Spawn reader thread.
        let pending_clone = pending_requests.clone();
        let app_handle_clone = app_handle.clone();
        let log_path_clone = log_path.clone();
        let stdin_clone = stdin.clone();
        let reader_handle = std::thread::spawn(move || {
            Self::read_loop(
                stdout,
                stdin_clone,
                pending_clone,
                app_handle_clone,
                log_path_clone,
            );
        });
        Ok(Self {
            process,
            stdin,
            reader_handle: Some(reader_handle),
            request_id: AtomicU64::new(1),
            pending_requests,
            session_id: Mutex::new(None),
            init_result: Mutex::new(None),
            mcp_servers,
            log_path,
        })
    }

    /// Read loop for processing messages from codex-acp.
    fn read_loop(
        stdout: ChildStdout,
        stdin: Arc<Mutex<ChildStdin>>,
        pending: PendingRequests,
        app_handle: AppHandle,
        log_path: PathBuf,
    ) {
        log_line(&log_path, "[ACP reader] starting read loop");
        let reader = BufReader::new(stdout);
        let mut exit_reason: Option<String> = None;
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(e) => {
                    let reason = format!("read error: {}", e);
                    log_line(&log_path, &format!("[ACP reader] {}", reason));
                    exit_reason = Some(reason);
                    break;
                }
            };

            if line.is_empty() {
                continue;
            }

            log_line(
                &log_path,
                &format!(
                    "[ACP reader] received line: {}",
                    Self::truncate_log_line(&line, 200)
                ),
            );

            let message: AcpMessage = match serde_json::from_str(&line) {
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
                        if let Err(e) = send_json_response(&stdin, request.id, result) {
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
                        let mut pending_guard = pending.lock().unwrap();
                        if let Some(sender) = pending_guard.remove(&id) {
                            let result = if let Some(error) = response.error {
                                Err(AcpError(format!("{}: {}", error.code, error.message)))
                            } else {
                                Ok(response.result.unwrap_or(Value::Null))
                            };
                            let _ = sender.send(result);
                        } else if let Some(result) = &response.result {
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
        let mut pending_guard = pending.lock().unwrap();
        for (_, sender) in pending_guard.drain() {
            let _ = sender.send(Err(AcpError(format!("ACP reader exited: {}", reason))));
        }
    }

    fn truncate_log_line(value: &str, max_chars: usize) -> String {
        value.chars().take(max_chars).collect()
    }

    /// Handle incoming notifications from codex-acp.
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
    fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value, AcpError> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let request = JsonRpcRequest::new(id, method, params);
        let request_json = serde_json::to_string(&request)?;

        // Register pending request.
        let (tx, rx) = mpsc::channel();
        {
            let mut pending = self.pending_requests.lock().unwrap();
            pending.insert(id, tx);
        }

        // Send request.
        {
            let mut stdin = self.stdin.lock().unwrap();
            writeln!(stdin, "{}", request_json)?;
            stdin.flush()?;
        }

        let timeout = request_timeout();
        match rx.recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let mut pending = self.pending_requests.lock().unwrap();
                pending.remove(&id);
                log_line(
                    &self.log_path,
                    &format!(
                        "[ACP] request timeout method={} id={} timeout_secs={}",
                        method,
                        id,
                        timeout.as_secs()
                    ),
                );
                Err(AcpError(format!(
                    "Request timed out after {}s",
                    timeout.as_secs()
                )))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let mut pending = self.pending_requests.lock().unwrap();
                pending.remove(&id);
                Err(AcpError("Request channel closed".into()))
            }
        }
    }

    /// Initialize the ACP connection.
    pub fn initialize(&self) -> Result<InitializeResult, AcpError> {
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

        let result = self.send_request("initialize", Some(serde_json::to_value(&params)?))?;
        let init_result: InitializeResult = serde_json::from_value(result)?;

        *self.init_result.lock().unwrap() = Some(init_result.clone());
        Ok(init_result)
    }

    /// Authenticate with the agent.
    pub fn authenticate(&self, method_id: &str) -> Result<(), AcpError> {
        let params = AuthenticateParams {
            method_id: method_id.to_string(),
        };
        self.send_request("authenticate", Some(serde_json::to_value(&params)?))?;
        Ok(())
    }

    /// Create a new session.
    pub fn new_session(&self, cwd: &str) -> Result<NewSessionResult, AcpError> {
        // Ensure cwd is absolute.
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

        let result = self.send_request("session/new", Some(serde_json::to_value(&params)?))?;
        let session_result: NewSessionResult = serde_json::from_value(result)?;

        *self.session_id.lock().unwrap() = Some(session_result.session_id.clone());
        Ok(session_result)
    }

    /// Load an existing session.
    pub fn load_session(&self, session_id: &str) -> Result<LoadSessionResult, AcpError> {
        let params = LoadSessionParams {
            session_id: session_id.to_string(),
        };

        let result = self.send_request("session/load", Some(serde_json::to_value(&params)?))?;
        let load_result: LoadSessionResult = serde_json::from_value(result)?;

        *self.session_id.lock().unwrap() = Some(session_id.to_string());
        Ok(load_result)
    }

    /// Send a prompt to the current session (non-blocking).
    /// Content comes via notifications, completion comes via response.
    pub fn prompt(
        &self,
        prompt: Vec<ContentBlock>,
        context: Option<Vec<ContextItem>>,
    ) -> Result<(), AcpError> {
        let session_id = self
            .session_id
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| AcpError("No active session".into()))?;

        let params = PromptParams {
            session_id,
            prompt,
            context,
        };

        // Send request but don't block waiting - let reader thread handle response.
        self.send_request_async("session/prompt", Some(serde_json::to_value(&params)?))
    }

    /// Send a request without waiting for response (reader thread will handle it).
    fn send_request_async(&self, method: &str, params: Option<Value>) -> Result<(), AcpError> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let request = JsonRpcRequest::new(id, method, params);
        let request_json = serde_json::to_string(&request)?;

        let mut stdin = self.stdin.lock().unwrap();
        writeln!(stdin, "{}", request_json)?;
        stdin.flush()?;
        Ok(())
    }

    /// Cancel the current operation.
    pub fn cancel(&self) -> Result<(), AcpError> {
        let session_id = self
            .session_id
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| AcpError("No active session".into()))?;

        let params = CancelParams { session_id };
        // Use async request (don't wait for response).
        self.send_request_async("session/cancel", Some(serde_json::to_value(&params)?))
    }

    /// Get the current session ID.
    pub fn get_session_id(&self) -> Option<String> {
        self.session_id.lock().unwrap().clone()
    }

    /// Get the initialization result.
    pub fn get_init_result(&self) -> Option<InitializeResult> {
        self.init_result.lock().unwrap().clone()
    }

    /// Stop the codex-acp process.
    pub fn stop(&mut self) {
        log_line(&self.log_path, "[ACP] stop: killing process");
        let _ = self.process.kill();
        // Don't wait for reader thread - it will exit when stdout closes.
        let _ = self.reader_handle.take();
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

fn send_json_response(
    stdin: &Arc<Mutex<ChildStdin>>,
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
    let mut guard = stdin.lock().unwrap();
    writeln!(guard, "{}", response_json)?;
    guard.flush()?;
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
        self.stop();
    }
}

/// State wrapper for Tauri.
pub struct AcpClientState(pub Mutex<Option<AcpClient>>);

impl Default for AcpClientState {
    fn default() -> Self {
        Self(Mutex::new(None))
    }
}
