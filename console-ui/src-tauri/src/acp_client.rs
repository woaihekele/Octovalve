//! ACP client for managing codex-acp subprocess and protocol communication.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, oneshot};

use crate::acp_types::*;

/// Error type for ACP operations
#[derive(Debug)]
pub struct AcpError(pub String);

impl std::fmt::Display for AcpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for AcpError {
    fn from(s: String) -> Self {
        AcpError(s)
    }
}

impl From<&str> for AcpError {
    fn from(s: &str) -> Self {
        AcpError(s.to_string())
    }
}

impl From<std::io::Error> for AcpError {
    fn from(e: std::io::Error) -> Self {
        AcpError(e.to_string())
    }
}

impl From<serde_json::Error> for AcpError {
    fn from(e: serde_json::Error) -> Self {
        AcpError(e.to_string())
    }
}

type PendingRequests = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, AcpError>>>>>;

/// ACP client managing the codex-acp subprocess
pub struct AcpClient {
    process: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    reader_handle: Option<std::thread::JoinHandle<()>>,
    request_id: AtomicU64,
    pending_requests: PendingRequests,
    session_id: Mutex<Option<String>>,
    init_result: Mutex<Option<InitializeResult>>,
}

impl AcpClient {
    /// Start a new codex-acp process
    pub fn start(
        codex_acp_path: &PathBuf,
        app_handle: AppHandle,
    ) -> Result<Self, AcpError> {
        let mut process = Command::new(codex_acp_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| AcpError(format!("Failed to start codex-acp: {}", e)))?;

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

        // Spawn reader thread
        let pending_clone = pending_requests.clone();
        let reader_handle = std::thread::spawn(move || {
            Self::read_loop(stdout, pending_clone, app_handle);
        });
        Ok(Self {
            process,
            stdin,
            reader_handle: Some(reader_handle),
            request_id: AtomicU64::new(1),
            pending_requests,
            session_id: Mutex::new(None),
            init_result: Mutex::new(None),
        })
    }

    /// Read loop for processing messages from codex-acp
    fn read_loop(stdout: ChildStdout, pending: PendingRequests, app_handle: AppHandle) {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };

            if line.is_empty() {
                continue;
            }

            let message: AcpMessage = match serde_json::from_str(&line) {
                Ok(m) => m,
                Err(_) => continue,
            };

            match message {
                AcpMessage::Response(response) => {
                    if let Some(id) = response.id {
                        let mut pending_guard = pending.lock().unwrap();
                        if let Some(sender) = pending_guard.remove(&id) {
                            let result = if let Some(error) = response.error {
                                Err(AcpError(format!("{}: {}", error.code, error.message)))
                            } else {
                                Ok(response.result.unwrap_or(Value::Null))
                            };
                            let _ = sender.send(result);
                        }
                    }
                }
                AcpMessage::Notification(notification) => {
                    Self::handle_notification(&app_handle, &notification);
                }
            }
        }
    }

    /// Handle incoming notifications from codex-acp
    fn handle_notification(app_handle: &AppHandle, notification: &JsonRpcNotification) {
        eprintln!("[ACP] Emitting notification: {}", notification.method);
        let event = AcpEvent {
            event_type: notification.method.clone(),
            payload: notification.params.clone().unwrap_or(Value::Null),
        };
        if let Err(e) = app_handle.emit("acp-event", &event) {
            eprintln!("[ACP] Failed to emit event: {}", e);
        }
    }

    /// Send a JSON-RPC request and wait for response
    fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value, AcpError> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let request = JsonRpcRequest::new(id, method, params);
        let request_json = serde_json::to_string(&request)?;

        // Register pending request
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending_requests.lock().unwrap();
            pending.insert(id, tx);
        }

        // Send request
        {
            let mut stdin = self.stdin.lock().unwrap();
            writeln!(stdin, "{}", request_json)?;
            stdin.flush()?;
        }

        // Wait for response (blocking)
        rx.blocking_recv()
            .map_err(|_| AcpError("Request cancelled".into()))?
    }

    /// Initialize the ACP connection
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

    /// Authenticate with the agent
    pub fn authenticate(&self, method_id: &str) -> Result<(), AcpError> {
        let params = AuthenticateParams {
            method_id: method_id.to_string(),
        };
        self.send_request("authenticate", Some(serde_json::to_value(&params)?))?;
        Ok(())
    }

    /// Create a new session
    pub fn new_session(&self, cwd: &str) -> Result<NewSessionResult, AcpError> {
        // Ensure cwd is absolute
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
            mcp_servers: vec![],
        };

        let result = self.send_request("session/new", Some(serde_json::to_value(&params)?))?;
        let session_result: NewSessionResult = serde_json::from_value(result)?;

        *self.session_id.lock().unwrap() = Some(session_result.session_id.clone());
        Ok(session_result)
    }

    /// Send a prompt to the current session
    pub fn prompt(&self, content: &str, context: Option<Vec<ContextItem>>) -> Result<(), AcpError> {
        let session_id = self
            .session_id
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| AcpError("No active session".into()))?;

        let params = PromptParams {
            session_id,
            prompt: vec![ContentBlock::text(content)],
            context,
        };

        let result = self.send_request("session/prompt", Some(serde_json::to_value(&params)?))?;
        let prompt_result: PromptResult = serde_json::from_value(result)?;
        Ok(())
    }

    /// Cancel the current operation
    pub fn cancel(&self) -> Result<(), AcpError> {
        let session_id = self
            .session_id
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| AcpError("No active session".into()))?;

        let params = CancelParams { session_id };
        self.send_request("session/cancel", Some(serde_json::to_value(&params)?))?;
        Ok(())
    }

    /// Get the current session ID
    pub fn get_session_id(&self) -> Option<String> {
        self.session_id.lock().unwrap().clone()
    }

    /// Get the initialization result
    pub fn get_init_result(&self) -> Option<InitializeResult> {
        self.init_result.lock().unwrap().clone()
    }

    /// Stop the codex-acp process
    pub fn stop(&mut self) {
        let _ = self.process.kill();
        if let Some(handle) = self.reader_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for AcpClient {
    fn drop(&mut self) {
        self.stop();
    }
}

/// State wrapper for Tauri
pub struct AcpClientState(pub Mutex<Option<AcpClient>>);

impl Default for AcpClientState {
    fn default() -> Self {
        Self(Mutex::new(None))
    }
}
