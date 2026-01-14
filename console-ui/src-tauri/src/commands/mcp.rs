use serde_json::Value;
use tauri::State;

use crate::clients::McpClientState;
use crate::services::mcp;
use crate::state::ProxyConfigState;

#[tauri::command]
pub async fn mcp_set_config(
    proxy_state: State<'_, ProxyConfigState>,
    mcp_state: State<'_, McpClientState>,
    config_json: Option<String>,
) -> Result<(), String> {
    mcp::mcp_set_config(proxy_state, mcp_state, config_json).await
}

#[tauri::command]
pub async fn mcp_list_tools(
    mcp_state: State<'_, McpClientState>,
) -> Result<Vec<mcp::McpToolInfo>, String> {
    mcp::mcp_list_tools(mcp_state).await
}

#[tauri::command]
pub async fn mcp_call_tool(
    mcp_state: State<'_, McpClientState>,
    server: Option<String>,
    name: String,
    arguments: Value,
) -> Result<Value, String> {
    mcp::mcp_call_tool(mcp_state, server, name, arguments).await
}
