use serde_json::Value;
use tauri::State;

use crate::clients::McpClientState;
use crate::services::mcp;
use crate::state::ProxyConfigState;

#[tauri::command]
pub async fn mcp_call_tool(
    app: tauri::AppHandle,
    proxy_state: State<'_, ProxyConfigState>,
    mcp_state: State<'_, McpClientState>,
    name: String,
    arguments: Value,
) -> Result<Value, String> {
    mcp::mcp_call_tool(app, proxy_state, mcp_state, name, arguments).await
}
