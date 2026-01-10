use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{oneshot, Mutex};
use tokio::time::timeout;

use octovalve_console::clients::acp_types::{
    AcpMessage, AuthenticateParams, ContentBlock, InitializeParams, InitializeResult,
    JsonRpcRequest, NewSessionParams, NewSessionResult, PromptParams, SessionUpdate,
};

#[derive(Debug, Deserialize)]
struct AcpEventPayload {
    #[serde(rename = "type")]
    event_type: String,
    payload: serde_json::Value,
}

fn parse_args() -> (Option<PathBuf>, String, Option<String>, Option<String>) {
    let mut args = std::env::args().skip(1);
    let mut acp_codex_path: Option<PathBuf> = None;
    let mut cwd: Option<String> = None;
    let mut auth_method: Option<String> = None;
    let mut image_path: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--acp-codex-path" => {
                acp_codex_path = args.next().map(PathBuf::from);
            }
            "--cwd" => {
                cwd = args.next();
            }
            "--auth-method" => {
                auth_method = args.next();
            }
            "--image" => {
                image_path = args.next();
            }
            _ => {}
        }
    }

    let acp_codex_path =
        acp_codex_path.or_else(|| std::env::var("ACP_CODEX_PATH").ok().map(PathBuf::from));
    let cwd = cwd.unwrap_or_else(|| {
        std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "/".to_string())
    });
    (acp_codex_path, cwd, auth_method, image_path)
}

fn main() -> Result<(), String> {
    let (acp_codex_path, cwd, auth_method, image_arg) = parse_args();
    if let Some(path) = acp_codex_path {
        eprintln!(
            "acp_codex_path={} (ignored: using embedded acp-codex)",
            path.display()
        );
    }
    eprintln!("cwd={}", cwd);

    let (tx, rx) = mpsc::channel::<AcpEventPayload>();

    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| format!("create tokio runtime failed: {}", e))?;
    let (client, _server_handle) = runtime.block_on(spawn_loopback(tx))?;

    let init_params = InitializeParams {
        protocol_version: "1".to_string(),
        client_capabilities: Default::default(),
        client_info: octovalve_console::clients::acp_types::ClientInfo {
            name: "octovalve".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    };
    let init_value = runtime
        .block_on(client.send_request(
            "initialize",
            Some(serde_json::to_value(&init_params).map_err(|e| e.to_string())?),
        ))
        .map_err(|e| format!("initialize failed: {}", e))?;
    let init: InitializeResult =
        serde_json::from_value(init_value).map_err(|e| format!("parse init failed: {}", e))?;
    eprintln!(
        "agent_info={:?}, auth_methods={:?}",
        init.agent_info, init.auth_methods
    );

    let method_id = auth_method.or_else(|| {
        if init.auth_methods.iter().any(|m| m.id == "openai-api-key") {
            Some("openai-api-key".to_string())
        } else if init.auth_methods.iter().any(|m| m.id == "codex-api-key") {
            Some("codex-api-key".to_string())
        } else {
            init.auth_methods.first().map(|m| m.id.clone())
        }
    });
    if let Some(method_id) = method_id {
        eprintln!("authenticate method_id={}", method_id);
        let auth_params = AuthenticateParams { method_id };
        runtime
            .block_on(client.send_request(
                "authenticate",
                Some(serde_json::to_value(&auth_params).map_err(|e| e.to_string())?),
            ))
            .map_err(|e| format!("authenticate failed: {}", e))?;
    } else {
        eprintln!("skip authenticate: no auth method");
    }

    let session_params = NewSessionParams {
        cwd: cwd.clone(),
        mcp_servers: Vec::new(),
    };
    let session_value = runtime
        .block_on(client.send_request(
            "session/new",
            Some(serde_json::to_value(&session_params).map_err(|e| e.to_string())?),
        ))
        .map_err(|e| format!("new_session failed: {}", e))?;
    let session: NewSessionResult = serde_json::from_value(session_value)
        .map_err(|e| format!("parse new_session failed: {}", e))?;
    eprintln!("session_id={}", session.session_id);

    let image_path = resolve_image_path(image_arg)
        .ok_or_else(|| "image path not found (use --image or ACP_IMAGE_PATH)".to_string())?;
    let image_bytes = fs::read(&image_path)
        .map_err(|e| format!("read image failed: {} ({})", image_path.display(), e))?;
    let image_base64 = base64_encode(&image_bytes);
    let mime_type = infer_mime_type(&image_path);

    let prompt_blocks = vec![
        ContentBlock::Text {
            text: "请描述这张图里是什么内容。".to_string(),
        },
        ContentBlock::Image {
            data: image_base64,
            mime_type,
        },
    ];

    let prompt_params = PromptParams {
        session_id: session.session_id.clone(),
        prompt: prompt_blocks,
        context: None,
    };
    runtime
        .block_on(client.send_request_async(
            "session/prompt",
            Some(serde_json::to_value(&prompt_params).map_err(|e| e.to_string())?),
        ))
        .map_err(|e| format!("prompt failed: {}", e))?;

    let mut full_text = String::new();
    let mut prompt_complete = false;
    let mut saw_retry = false;
    let mut retry_last_attempt: Option<u64> = None;
    let mut retry_max_attempts: Option<u64> = None;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(120) {
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(event) => {
                match event.event_type.as_str() {
                    "session/update" => {
                        if let Some(update) = event.payload.get("update") {
                            if let Some(update_type) =
                                update.get("sessionUpdate").and_then(|v| v.as_str())
                            {
                                match update_type {
                                    "agent_message_chunk" => {
                                        if let Some(text) = update
                                            .get("content")
                                            .and_then(|v| v.get("text"))
                                            .and_then(|v| v.as_str())
                                        {
                                            full_text.push_str(text);
                                            eprint!("{}", text);
                                        }
                                    }
                                    "agent_thought_chunk" => {}
                                    "retry" => {
                                        let attempt = update
                                            .get("attempt")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0);
                                        let max_attempts = update
                                            .get("maxAttempts")
                                            .or_else(|| update.get("max_attempts"))
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0);
                                        let message = update
                                            .get("message")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("Retrying...");

                                        saw_retry = true;
                                        retry_max_attempts = Some(max_attempts);
                                        if let Some(prev) = retry_last_attempt {
                                            if attempt != prev.saturating_add(1) && attempt != prev
                                            {
                                                eprintln!(
                                                    "\n[retry] unexpected attempt change: prev={} now={}",
                                                    prev, attempt
                                                );
                                            }
                                        }
                                        retry_last_attempt = Some(attempt);
                                        eprintln!(
                                            "\n[retry] attempt={}/{} message={}",
                                            attempt, max_attempts, message
                                        );
                                    }
                                    "error" => {
                                        if let Some(msg) = update
                                            .get("error")
                                            .and_then(|v| v.get("message"))
                                            .and_then(|v| v.as_str())
                                        {
                                            eprintln!("\n[session_error] {}", msg);
                                        } else {
                                            eprintln!(
                                                "\n[session_error] {:?}",
                                                update.get("error")
                                            );
                                        }
                                    }
                                    "task_complete" => {
                                        eprintln!(
                                            "\n[session_complete_update] stop_reason={:?}",
                                            update
                                                .get("stopReason")
                                                .or_else(|| update.get("stop_reason"))
                                        );
                                    }
                                    "available_commands_update" => {
                                        if let Some(commands) = update.get("availableCommands") {
                                            eprintln!("\n[available_commands] {}", commands);
                                        }
                                    }
                                    other => {
                                        eprintln!("\n[session_update] type={}", other);
                                    }
                                }
                                continue;
                            }
                        }

                        match serde_json::from_value::<SessionUpdate>(event.payload) {
                            Ok(SessionUpdate::ContentDelta { content, .. }) => {
                                full_text.push_str(&content);
                                eprint!("{}", content);
                            }
                            Ok(SessionUpdate::ToolCallStart {
                                name, arguments, ..
                            }) => {
                                eprintln!("\n[tool_call_start] name={name} args={arguments:?}");
                            }
                            Ok(SessionUpdate::ToolCallEnd { result, error, .. }) => {
                                eprintln!("\n[tool_call_end] result={result:?} error={error:?}");
                            }
                            Ok(SessionUpdate::PermissionRequest { request_id, .. }) => {
                                eprintln!("\n[permission_request] id={request_id}");
                            }
                            Ok(SessionUpdate::Error { message, .. }) => {
                                eprintln!("\n[session_error] {}", message);
                            }
                            Ok(SessionUpdate::Complete { stop_reason, .. }) => {
                                eprintln!("\n[session_complete] {}", stop_reason);
                            }
                            Ok(SessionUpdate::Unknown) => {}
                            Err(err) => {
                                eprintln!("\n[session_update] parse error: {}", err);
                            }
                        }
                    }
                    "prompt/complete" => {
                        eprintln!("\n[prompt_complete]");
                        prompt_complete = true;
                    }
                    other => {
                        eprintln!("\n[event] type={}", other);
                    }
                }
                if prompt_complete {
                    if saw_retry {
                        let attempt = retry_last_attempt.unwrap_or(0);
                        let max_attempts = retry_max_attempts.unwrap_or(0);
                        if max_attempts > 0 && attempt < max_attempts {
                            eprintln!(
                                "[BUG] prompt_complete arrived before retries exhausted: attempt={}/{}",
                                attempt, max_attempts
                            );
                        }
                    }
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(err) => return Err(format!("event channel closed: {err}")),
        }
    }

    if full_text.is_empty() {
        eprintln!("[result] empty");
    } else {
        eprintln!("\n[result] {}", full_text.trim());
    }

    Ok(())
}

struct LoopbackClient {
    writer: Arc<Mutex<tokio::io::WriteHalf<tokio::io::DuplexStream>>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>,
    request_id: AtomicU64,
}

impl LoopbackClient {
    async fn send_request(&self, method: &str, params: Option<Value>) -> Result<Value, String> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let request = JsonRpcRequest::new(id, method, params);
        let request_json = serde_json::to_string(&request).map_err(|e| e.to_string())?;

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        {
            let mut writer = self.writer.lock().await;
            writer
                .write_all(request_json.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            writer.write_all(b"\n").await.map_err(|e| e.to_string())?;
            writer.flush().await.map_err(|e| e.to_string())?;
        }

        let response = timeout(Duration::from_secs(30), rx)
            .await
            .map_err(|_| "request timeout".to_string())?
            .map_err(|_| "request channel closed".to_string())?;
        response
    }

    async fn send_request_async(&self, method: &str, params: Option<Value>) -> Result<(), String> {
        let id = self.request_id.fetch_add(1, Ordering::SeqCst);
        let request = JsonRpcRequest::new(id, method, params);
        let request_json = serde_json::to_string(&request).map_err(|e| e.to_string())?;
        let mut writer = self.writer.lock().await;
        writer
            .write_all(request_json.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        writer.write_all(b"\n").await.map_err(|e| e.to_string())?;
        writer.flush().await.map_err(|e| e.to_string())?;
        Ok(())
    }
}

async fn spawn_loopback(
    event_tx: mpsc::Sender<AcpEventPayload>,
) -> Result<(LoopbackClient, tokio::task::JoinHandle<()>), String> {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let (client_read, client_write) = tokio::io::split(client_io);
    let (server_read, server_write) = tokio::io::split(server_io);

    let config = acp_codex::CliConfig {
        approval_policy: None,
        sandbox_mode: None,
        app_server_args: Vec::new(),
    };

    let server_handle = tokio::spawn(async move {
        if let Err(err) = acp_codex::run_with_io(config, server_read, server_write).await {
            eprintln!("[acp-codex] server exited: {err}");
        }
    });

    let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let pending_clone = pending.clone();
    tokio::spawn(async move {
        if let Err(err) = read_loop(client_read, pending_clone, event_tx).await {
            eprintln!("[acp-codex] client read loop error: {}", err);
        }
    });

    Ok((
        LoopbackClient {
            writer: Arc::new(Mutex::new(client_write)),
            pending,
            request_id: AtomicU64::new(1),
        },
        server_handle,
    ))
}

async fn read_loop(
    reader: tokio::io::ReadHalf<tokio::io::DuplexStream>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>,
    event_tx: mpsc::Sender<AcpEventPayload>,
) -> Result<(), String> {
    let mut reader = BufReader::new(reader);
    let mut buffer = String::new();
    loop {
        buffer.clear();
        let bytes = reader
            .read_line(&mut buffer)
            .await
            .map_err(|e| e.to_string())?;
        if bytes == 0 {
            break;
        }
        let line = buffer.trim_end_matches(['\n', '\r']);
        if line.is_empty() {
            continue;
        }
        let message: AcpMessage =
            serde_json::from_str(line).map_err(|e| format!("parse error: {}", e))?;
        match message {
            AcpMessage::Notification(notification) => {
                let payload = notification.params.unwrap_or(Value::Null);
                let event = AcpEventPayload {
                    event_type: notification.method,
                    payload,
                };
                let _ = event_tx.send(event);
            }
            AcpMessage::Response(response) => {
                if let Some(id) = response.id {
                    let mut pending_guard = pending.lock().await;
                    if let Some(sender) = pending_guard.remove(&id) {
                        let result = if let Some(error) = response.error {
                            Err(format!("{}: {}", error.code, error.message))
                        } else {
                            Ok(response.result.unwrap_or(Value::Null))
                        };
                        let _ = sender.send(result);
                    } else if let Some(result) = response.result {
                        if result.get("stopReason").is_some() {
                            let event = AcpEventPayload {
                                event_type: "prompt/complete".to_string(),
                                payload: result,
                            };
                            let _ = event_tx.send(event);
                        }
                    }
                }
            }
            AcpMessage::Request(_) => {}
        }
    }
    Ok(())
}

fn resolve_image_path(cli_value: Option<String>) -> Option<PathBuf> {
    if let Some(value) = cli_value {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            if path.exists() {
                return Some(path);
            }
        }
    }
    if let Ok(value) = std::env::var("ACP_IMAGE_PATH") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            if path.exists() {
                return Some(path);
            }
        }
    }
    let local = PathBuf::from("SCR-20260106-qdmo.png");
    if local.exists() {
        return Some(local);
    }
    if let Ok(home) = std::env::var("HOME") {
        let doc = PathBuf::from(home)
            .join("Documents")
            .join("SCR-20260106-qdmo.png");
        if doc.exists() {
            return Some(doc);
        }
    }
    None
}

fn infer_mime_type(path: &PathBuf) -> String {
    let ext = path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);

        let idx0 = b0 >> 2;
        let idx1 = ((b0 & 0x03) << 4) | (b1 >> 4);
        let idx2 = ((b1 & 0x0f) << 2) | (b2 >> 6);
        let idx3 = b2 & 0x3f;

        out.push(TABLE[idx0 as usize] as char);
        out.push(TABLE[idx1 as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[idx2 as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[idx3 as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}
