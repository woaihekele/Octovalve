use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use base64::Engine;
use codex_protocol::{
    config_types::SandboxMode as CodexSandboxMode,
    protocol::AskForApproval as CodexAskForApproval,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::cli::CliConfig;

pub(crate) struct SessionHandler;

impl SessionHandler {
    pub(crate) fn sessions_root() -> Result<PathBuf> {
        let home_dir = dirs::home_dir().ok_or_else(|| anyhow!("无法定位 HOME 目录"))?;
        Ok(home_dir.join(".codex").join("sessions"))
    }

    pub(crate) fn find_rollout_file_path(session_id: &str) -> Result<PathBuf> {
        let sessions_dir = Self::sessions_root()?;
        Self::scan_directory(&sessions_dir, session_id)
    }

    fn scan_directory(dir: &Path, session_id: &str) -> Result<PathBuf> {
        if !dir.exists() {
            return Err(anyhow!("sessions 目录不存在: {}", dir.display()));
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                if let Ok(found) = Self::scan_directory(&path, session_id) {
                    return Ok(found);
                }
            } else if path.is_file() {
                let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if filename.starts_with("rollout-")
                    && filename.ends_with(".jsonl")
                    && filename.contains(session_id)
                {
                    return Ok(path);
                }
            }
        }

        Err(anyhow!("未找到 session: {session_id}"))
    }
}

pub(crate) fn normalize_cwd(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

pub(crate) fn build_new_conversation_params(
    config: &CliConfig,
    cwd: &Path,
) -> Result<codex_app_server_protocol::NewConversationParams> {
    let sandbox = match config
        .sandbox_mode
        .as_deref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty() && *v != "auto")
    {
        None => Some(CodexSandboxMode::WorkspaceWrite),
        Some("read-only") => Some(CodexSandboxMode::ReadOnly),
        Some("workspace-write") => Some(CodexSandboxMode::WorkspaceWrite),
        Some("danger-full-access") => Some(CodexSandboxMode::DangerFullAccess),
        Some(other) => return Err(anyhow!("未知 sandbox_mode: {other}")),
    };

    let mut approval_policy = match config
        .approval_policy
        .as_deref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty() && *v != "auto")
    {
        None => None,
        Some("unless-trusted") => Some(CodexAskForApproval::UnlessTrusted),
        Some("on-failure") => Some(CodexAskForApproval::OnFailure),
        Some("on-request") => Some(CodexAskForApproval::OnRequest),
        Some("never") => Some(CodexAskForApproval::Never),
        Some(other) => return Err(anyhow!("未知 approval_policy: {other}")),
    };

    if approval_policy.is_none() {
        approval_policy = Some(CodexAskForApproval::OnRequest);
    }

    Ok(codex_app_server_protocol::NewConversationParams {
        model: None,
        profile: None,
        cwd: Some(cwd.to_string_lossy().to_string()),
        approval_policy,
        sandbox,
        config: None,
        base_instructions: None,
        include_apply_patch_tool: Some(true),
        model_provider: None,
        compact_prompt: None,
        developer_instructions: None,
    })
}

pub(crate) fn build_mcp_overrides(servers: &[Value]) -> Option<HashMap<String, Value>> {
    if servers.is_empty() {
        return None;
    }

    let mut overrides = HashMap::new();
    for server in servers {
        let Value::Object(map) = server else {
            continue;
        };
        let name = map
            .get("name")
            .and_then(|value| value.as_str())
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string());
        let Some(name) = name else {
            continue;
        };
        let mut config = map.clone();
        config.remove("name");
        normalize_mcp_server_config(&mut config);
        overrides.insert(format!("mcp_servers.{name}"), Value::Object(config));
    }

    if overrides.is_empty() {
        None
    } else {
        Some(overrides)
    }
}

fn normalize_mcp_server_config(config: &mut serde_json::Map<String, Value>) {
    if let Some(value) = config.remove("envVars") {
        config.entry("env_vars".to_string()).or_insert(value);
    }

    if let Some(env_value) = config.remove("env") {
        match env_value {
            Value::Null => {}
            Value::Array(values) => {
                if !values.is_empty() && values.iter().all(|value| value.as_str().is_some()) {
                    config
                        .entry("env_vars".to_string())
                        .or_insert(Value::Array(values));
                }
            }
            other => {
                config.insert("env".to_string(), other);
            }
        }
    }
}

pub(crate) fn update_with_type(update_type: &str) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    map.insert(
        "session_update".to_string(),
        Value::String(update_type.to_string()),
    );
    map.insert(
        "sessionUpdate".to_string(),
        Value::String(update_type.to_string()),
    );
    map
}

pub(crate) fn insert_dual(
    map: &mut serde_json::Map<String, Value>,
    snake: &str,
    camel: &str,
    value: Value,
) {
    map.insert(snake.to_string(), value.clone());
    map.insert(camel.to_string(), value);
}

pub(crate) fn normalize_base64_payload(data: &str) -> String {
    let trimmed = data.trim();
    let payload = match trimmed.split_once("base64,") {
        Some((_, rest)) => rest,
        None => trimmed,
    };
    payload.chars().filter(|ch| !ch.is_whitespace()).collect()
}

pub(crate) fn image_extension_for_mime(mime_type: &str) -> &'static str {
    let lowered = mime_type
        .split(';')
        .next()
        .unwrap_or(mime_type)
        .trim()
        .to_ascii_lowercase();
    if lowered.contains("png") {
        "png"
    } else if lowered.contains("jpeg") || lowered.contains("jpg") {
        "jpg"
    } else if lowered.contains("webp") {
        "webp"
    } else if lowered.contains("gif") {
        "gif"
    } else {
        "bin"
    }
}

pub(crate) fn write_temp_image(data: &str, mime_type: &str) -> Result<PathBuf> {
    let normalized = normalize_base64_payload(data);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(normalized.as_bytes())
        .map_err(|err| anyhow!("image base64 decode failed: {err}"))?;
    let ext = image_extension_for_mime(mime_type);
    let filename = format!("acp-codex-image-{}.{}", Uuid::new_v4(), ext);
    let path = std::env::temp_dir().join(filename);
    std::fs::write(&path, bytes)?;
    Ok(path)
}

pub(crate) async fn load_rollout_history(path: &Path) -> Result<Vec<Value>> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let mut entries = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let Some(payload) = value.get("payload") else {
            continue;
        };
        let Some(kind) = payload.get("type").and_then(|v| v.as_str()) else {
            continue;
        };
        let message = payload.get("message").and_then(|v| v.as_str());
        if message.is_none() {
            continue;
        }
        let role = match kind {
            "user_message" => "user",
            "agent_message" => "assistant",
            _ => continue,
        };
        entries.push(json!({
            "role": role,
            "content": message.unwrap_or_default(),
        }));
    }

    Ok(entries)
}
