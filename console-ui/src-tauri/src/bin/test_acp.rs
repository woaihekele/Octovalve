use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tauri::{Listener, Manager};

use octovalve_console::clients::acp_client::AcpClient;
use octovalve_console::clients::acp_types::{ContentBlock, SessionUpdate};

#[derive(Debug, Deserialize)]
struct AcpEventPayload {
    #[serde(rename = "type")]
    event_type: String,
    payload: serde_json::Value,
}

fn parse_args() -> (PathBuf, String, Option<String>, Option<String>) {
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

    let acp_codex_path = acp_codex_path
        .or_else(|| std::env::var("ACP_CODEX_PATH").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("acp-codex"));
    let cwd = cwd.unwrap_or_else(|| {
        std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "/".to_string())
    });
    (acp_codex_path, cwd, auth_method, image_path)
}

fn main() -> Result<(), String> {
    let (acp_codex_path, cwd, auth_method, image_arg) = parse_args();
    eprintln!("acp_codex_path={}", acp_codex_path.display());
    eprintln!("cwd={}", cwd);

    let app = tauri::Builder::default()
        .build(tauri::generate_context!())
        .map_err(|e| format!("build app failed: {}", e))?;
    let app_handle = app.handle().clone();
    let config_dir = app_handle
        .path()
        .app_config_dir()
        .map_err(|e| format!("app_config_dir failed: {}", e))?;
    fs::create_dir_all(&config_dir).map_err(|e| format!("create_dir_all failed: {}", e))?;
    let logs_dir = config_dir.join("logs");
    fs::create_dir_all(&logs_dir).map_err(|e| format!("create logs dir failed: {}", e))?;
    let app_log = logs_dir.join("app.log");

    let (tx, rx) = mpsc::channel::<AcpEventPayload>();
    let _listener_id = app_handle.listen("acp-event", move |event| {
        match serde_json::from_str::<AcpEventPayload>(event.payload()) {
            Ok(evt) => {
                let _ = tx.send(evt);
            }
            Err(err) => {
                eprintln!("[event] parse error: {}", err);
            }
        }
    });

    let mut client = AcpClient::start(
        &acp_codex_path,
        app_handle.clone(),
        app_log,
        Vec::new(),
        Vec::new(),
    )
    .map_err(|e| format!("start failed: {}", e))?;
    let init = client
        .initialize()
        .map_err(|e| format!("initialize failed: {}", e))?;
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
        client
            .authenticate(&method_id)
            .map_err(|e| format!("authenticate failed: {}", e))?;
    } else {
        eprintln!("skip authenticate: no auth method");
    }

    let session = client
        .new_session(&cwd)
        .map_err(|e| format!("new_session failed: {}", e))?;
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

    client
        .prompt(prompt_blocks, None)
        .map_err(|e| format!("prompt failed: {}", e))?;

    let mut full_text = String::new();
    let mut prompt_complete = false;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
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

    client.stop();
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
