use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::clients::mcp_client::McpServerSpec;

// Codex（app-server）对 MCP tool call 默认 60s 超时；我们给内置 octovalve MCP server
// 一个更宽松的默认值，避免长命令（构建/测试/部署）在 60s 被 Codex 终止。
const DEFAULT_CODEX_TOOL_TIMEOUT_SEC: u64 = 60 * 60;

pub struct ParsedMcpConfig {
    pub servers: Vec<Value>,
    pub stdio_servers: Vec<McpServerSpec>,
    pub has_octovalve: bool,
}

pub fn parse_mcp_config_json(raw: &str) -> Result<ParsedMcpConfig, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(ParsedMcpConfig {
            servers: Vec::new(),
            stdio_servers: Vec::new(),
            has_octovalve: false,
        });
    }
    let value: Value = serde_json::from_str(trimmed)
        .map_err(|err| format!("MCP JSON 解析失败: {err}"))?;
    let servers_map = extract_servers_map(&value)
        .ok_or_else(|| "MCP JSON 缺少 mcpServers 对象".to_string())?;

    let mut servers = Vec::new();
    let mut stdio_servers = Vec::new();
    let mut has_octovalve = false;

    for (name, config_value) in servers_map.iter() {
        if name == "octovalve" {
            has_octovalve = true;
        }
        let Some(config) = config_value.as_object() else {
            continue;
        };
        let mut entry = config.clone();
        entry.insert("name".to_string(), Value::String(name.clone()));
        // 如果用户没有显式设置 tool_timeout_sec，则给 octovalve 默认加大超时，避免 Codex 60s 限制。
        if name == "octovalve"
            && !entry.contains_key("tool_timeout_sec")
            && !entry.contains_key("toolTimeoutSec")
        {
            entry.insert(
                "tool_timeout_sec".to_string(),
                Value::from(DEFAULT_CODEX_TOOL_TIMEOUT_SEC),
            );
        }
        servers.push(Value::Object(entry.clone()));

        let enabled = entry
            .get("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        if !enabled {
            continue;
        }
        let Some(command) = entry.get("command").and_then(|value| value.as_str()) else {
            continue;
        };
        let args = parse_args(entry.get("args"));
        let env = parse_env(entry.get("env"), entry.get("envVars"), entry.get("env_vars"));
        let cwd = entry
            .get("cwd")
            .and_then(|value| value.as_str())
            .map(PathBuf::from);
        stdio_servers.push(McpServerSpec {
            name: name.clone(),
            command: PathBuf::from(command),
            args,
            env,
            cwd,
        });
    }

    Ok(ParsedMcpConfig {
        servers,
        stdio_servers,
        has_octovalve,
    })
}

pub fn build_octovalve_server(
    proxy_bin: &Path,
    proxy_config: &Path,
    command_addr: &str,
) -> (McpServerSpec, Value) {
    let args = vec![
        "--config".to_string(),
        proxy_config.to_string_lossy().to_string(),
        "--command-addr".to_string(),
        command_addr.to_string(),
    ];
    let spec = McpServerSpec {
        name: "octovalve".to_string(),
        command: proxy_bin.to_path_buf(),
        args: args.clone(),
        env: HashMap::new(),
        cwd: None,
    };
    let value = Value::Object({
        let mut map = Map::new();
        map.insert("name".to_string(), Value::String("octovalve".to_string()));
        map.insert(
            "command".to_string(),
            Value::String(proxy_bin.to_string_lossy().to_string()),
        );
        map.insert(
            "args".to_string(),
            Value::Array(args.into_iter().map(Value::String).collect()),
        );
        map.insert(
            "tool_timeout_sec".to_string(),
            Value::from(DEFAULT_CODEX_TOOL_TIMEOUT_SEC),
        );
        map
    });
    (spec, value)
}

fn extract_servers_map(value: &Value) -> Option<&Map<String, Value>> {
    if let Some(obj) = value.get("mcpServers").and_then(|v| v.as_object()) {
        return Some(obj);
    }
    if let Some(obj) = value.get("mcp_servers").and_then(|v| v.as_object()) {
        return Some(obj);
    }
    let Some(obj) = value.as_object() else {
        return None;
    };
    if obj.is_empty() {
        return Some(obj);
    }
    let mut has_server_like = false;
    for (_, entry) in obj.iter() {
        let Some(entry_obj) = entry.as_object() else {
            return None;
        };
        if entry_obj.contains_key("command")
            || entry_obj.contains_key("url")
            || entry_obj.contains_key("transport")
        {
            has_server_like = true;
        }
    }
    if has_server_like {
        Some(obj)
    } else {
        None
    }
}

fn parse_args(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(|value| value.to_string()))
            .collect(),
        Some(Value::String(value)) => vec![value.clone()],
        _ => Vec::new(),
    }
}

fn parse_env(
    env_value: Option<&Value>,
    env_vars_value: Option<&Value>,
    env_vars_snake: Option<&Value>,
) -> HashMap<String, String> {
    let mut env = HashMap::new();
    match env_value {
        Some(Value::Object(map)) => {
            for (key, value) in map.iter() {
                if let Some(val) = value.as_str() {
                    env.insert(key.clone(), val.to_string());
                }
            }
        }
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(value) = item.as_str() {
                    if let Some((key, val)) = value.split_once('=') {
                        env.insert(key.to_string(), val.to_string());
                    }
                    continue;
                }
                let Some(obj) = item.as_object() else {
                    continue;
                };
                let name = obj.get("name").and_then(|v| v.as_str());
                let value = obj.get("value").and_then(|v| v.as_str());
                if let (Some(name), Some(value)) = (name, value) {
                    env.insert(name.to_string(), value.to_string());
                }
            }
        }
        _ => {}
    }
    if env.is_empty() {
        apply_env_vars_value(&mut env, env_vars_value);
    }
    if env.is_empty() {
        apply_env_vars_value(&mut env, env_vars_snake);
    }
    env
}

fn apply_env_vars_value(env: &mut HashMap<String, String>, value: Option<&Value>) {
    let Some(value) = value else {
        return;
    };
    match value {
        Value::Object(map) => {
            for (key, value) in map.iter() {
                if let Some(val) = value.as_str() {
                    env.insert(key.clone(), val.to_string());
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                if let Some(value) = item.as_str() {
                    if let Some((key, val)) = value.split_once('=') {
                        env.insert(key.to_string(), val.to_string());
                    }
                    continue;
                }
                let Some(obj) = item.as_object() else {
                    continue;
                };
                let name = obj.get("name").and_then(|v| v.as_str());
                let value = obj.get("value").and_then(|v| v.as_str());
                if let (Some(name), Some(value)) = (name, value) {
                    env.insert(name.to_string(), value.to_string());
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mcp_config_adds_default_tool_timeout_for_octovalve() {
        let raw = r#"{
          "mcpServers": {
            "octovalve": {
              "command": "/bin/echo",
              "args": ["hello"]
            }
          }
        }"#;
        let parsed = parse_mcp_config_json(raw).expect("parse");
        assert!(parsed.has_octovalve);
        let octo = parsed
            .servers
            .iter()
            .filter_map(|v| v.as_object())
            .find(|m| m.get("name").and_then(|v| v.as_str()) == Some("octovalve"))
            .expect("octovalve server");
        assert_eq!(
            octo.get("tool_timeout_sec").and_then(|v| v.as_u64()),
            Some(DEFAULT_CODEX_TOOL_TIMEOUT_SEC)
        );
    }

    #[test]
    fn parse_mcp_config_does_not_override_explicit_tool_timeout() {
        let raw = r#"{
          "mcpServers": {
            "octovalve": {
              "command": "/bin/echo",
              "args": ["hello"],
              "tool_timeout_sec": 12
            }
          }
        }"#;
        let parsed = parse_mcp_config_json(raw).expect("parse");
        let octo = parsed
            .servers
            .iter()
            .filter_map(|v| v.as_object())
            .find(|m| m.get("name").and_then(|v| v.as_str()) == Some("octovalve"))
            .expect("octovalve server");
        assert_eq!(octo.get("tool_timeout_sec").and_then(|v| v.as_u64()), Some(12));
    }
}
