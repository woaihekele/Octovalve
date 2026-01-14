use serde::Serialize;
use serde_json::Value;
use tauri::State;

use crate::clients::McpClientState;
use crate::paths::resolve_octovalve_proxy_bin;
use crate::services::console_sidecar::DEFAULT_COMMAND_ADDR;
use crate::services::mcp_config::{build_octovalve_server, parse_mcp_config_json};
use crate::state::ProxyConfigState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolInfo {
    pub server: String,
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

pub async fn mcp_set_config(
    proxy_state: State<'_, ProxyConfigState>,
    mcp_state: State<'_, McpClientState>,
    config_json: Option<String>,
) -> Result<(), String> {
    let raw = config_json.unwrap_or_default();
    let mut parsed = parse_mcp_config_json(&raw)?;

    if !parsed.has_octovalve {
        let status = proxy_state.0.lock().unwrap().clone();
        if status.present {
            let proxy_config = std::path::PathBuf::from(status.path);
            let proxy_bin = resolve_octovalve_proxy_bin()?;
            let (spec, value) =
                build_octovalve_server(&proxy_bin, &proxy_config, DEFAULT_COMMAND_ADDR);
            parsed.servers.push(value);
            parsed.stdio_servers.push(spec);
        }
    }

    mcp_state.set_servers(parsed.stdio_servers).await
}

pub async fn mcp_list_tools(
    mcp_state: State<'_, McpClientState>,
) -> Result<Vec<McpToolInfo>, String> {
    let tools = mcp_state.list_tools().await?;
    let mut result = Vec::new();
    for (server, tool) in tools {
        let input_schema = Value::Object(tool.input_schema.as_ref().clone());
        result.push(McpToolInfo {
            server,
            name: tool.name.to_string(),
            description: tool.description.map(|value| value.to_string()),
            input_schema,
        });
    }
    Ok(result)
}

pub async fn mcp_call_tool(
    mcp_state: State<'_, McpClientState>,
    server: Option<String>,
    name: String,
    arguments: Value,
) -> Result<Value, String> {
    let server = server.unwrap_or_else(|| "octovalve".to_string());
    mcp_state.call_tool(&server, &name, arguments).await
}
