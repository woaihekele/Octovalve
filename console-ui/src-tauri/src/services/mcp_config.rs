use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::clients::mcp_client::McpServerSpec;

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
