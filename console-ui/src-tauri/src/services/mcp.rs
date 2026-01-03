use std::path::PathBuf;

use serde_json::Value;
use tauri::{AppHandle, State};

use crate::clients::McpClientState;
use crate::state::ProxyConfigState;

pub async fn mcp_call_tool(
    app: AppHandle,
    proxy_state: State<'_, ProxyConfigState>,
    mcp_state: State<'_, McpClientState>,
    name: String,
    arguments: Value,
) -> Result<Value, String> {
    let status = proxy_state.0.lock().unwrap().clone();
    if !status.present {
        return Err("proxy config missing".to_string());
    }
    let proxy_path = PathBuf::from(status.path);
    let _ = app;
    mcp_state.call_tool(&proxy_path, &name, arguments).await
}
