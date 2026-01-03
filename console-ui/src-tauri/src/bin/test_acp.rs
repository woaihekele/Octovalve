use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tauri::Listener;

use octovalve_console::clients::acp_client::AcpClient;
use octovalve_console::clients::acp_types::SessionUpdate;

#[derive(Debug, Deserialize)]
struct AcpEventPayload {
    #[serde(rename = "type")]
    event_type: String,
    payload: serde_json::Value,
}

fn parse_args() -> (PathBuf, String, Option<String>) {
    let mut args = std::env::args().skip(1);
    let mut codex_acp_path: Option<PathBuf> = None;
    let mut cwd: Option<String> = None;
    let mut auth_method: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--codex-acp-path" => {
                codex_acp_path = args.next().map(PathBuf::from);
            }
            "--cwd" => {
                cwd = args.next();
            }
            "--auth-method" => {
                auth_method = args.next();
            }
            _ => {}
        }
    }

    let codex_acp_path = codex_acp_path
        .or_else(|| std::env::var("CODEX_ACP_PATH").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("codex-acp"));
    let cwd = cwd.unwrap_or_else(|| {
        std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "/".to_string())
    });
    (codex_acp_path, cwd, auth_method)
}

fn main() -> Result<(), String> {
    let (codex_acp_path, cwd, auth_method) = parse_args();
    eprintln!("codex_acp_path={}", codex_acp_path.display());
    eprintln!("cwd={}", cwd);

    let app = tauri::Builder::default()
        .build(tauri::generate_context!())
        .map_err(|e| format!("build app failed: {}", e))?;
    let app_handle = app.handle().clone();

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

    let mut client = AcpClient::start(&codex_acp_path, app_handle.clone())
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

    client
        .prompt("查看当前有哪些工具", None)
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
