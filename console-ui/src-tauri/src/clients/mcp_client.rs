use rmcp::{
    model::{
        CallToolRequestParam, CallToolResult, ClientCapabilities, ClientInfo, Implementation,
        ListToolsResult, ProtocolVersion,
    },
    service::{RoleClient, RunningService, ServiceExt},
    transport::{ConfigureCommandExt, TokioChildProcess},
    ServiceError,
};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::process::Command as TokioCommand;
use tokio::sync::Mutex;

use crate::paths::resolve_octovalve_proxy_bin;

const DEFAULT_CLIENT_ID: &str = "octovalve-console-openai";

pub struct McpClientState(pub Mutex<Option<McpClient>>);

impl Default for McpClientState {
    fn default() -> Self {
        Self(Mutex::new(None))
    }
}

impl McpClientState {
    pub async fn call_tool(
        &self,
        proxy_config_path: &Path,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value, String> {
        let mut guard = self.0.lock().await;
        let needs_restart = match guard.as_ref() {
            Some(client) => !client.is_usable_for(proxy_config_path),
            None => true,
        };
        if needs_restart {
            if let Some(old) = guard.take() {
                drop(guard);
                old.shutdown().await;
                guard = self.0.lock().await;
            }
            let client = McpClient::start(proxy_config_path, DEFAULT_CLIENT_ID).await?;
            *guard = Some(client);
        }
        let client = guard
            .as_ref()
            .ok_or_else(|| "mcp client unavailable".to_string())?;
        let result = client.call_tool(tool_name, arguments).await?;
        serde_json::to_value(&result).map_err(|err| err.to_string())
    }
}

pub struct McpClient {
    service: RunningService<RoleClient, ClientInfo>,
    config_path: PathBuf,
}

impl McpClient {
    pub async fn start(proxy_config_path: &Path, client_id: &str) -> Result<Self, String> {
        let proxy_bin = resolve_octovalve_proxy_bin()?;
        let client_info = ClientInfo {
            protocol_version: ProtocolVersion::V_2025_06_18,
            capabilities: ClientCapabilities::default(),
            client_info: Implementation {
                name: "octovalve-console".to_string(),
                version: "0.1.0".to_string(),
                title: Some("Octovalve Console".to_string()),
                icons: None,
                website_url: None,
            },
        };
        let transport = TokioChildProcess::new(TokioCommand::new(proxy_bin).configure(|cmd| {
            cmd.arg("--config").arg(proxy_config_path);
            cmd.arg("--client-id").arg(client_id);
        }))
        .map_err(|err| err.to_string())?;
        let service = client_info
            .serve(transport)
            .await
            .map_err(|err| err.to_string())?;
        Ok(Self {
            service,
            config_path: proxy_config_path.to_path_buf(),
        })
    }

    pub fn is_usable_for(&self, proxy_config_path: &Path) -> bool {
        self.config_path.as_path() == proxy_config_path && !self.service.is_transport_closed()
    }

    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
    ) -> Result<CallToolResult, String> {
        let arguments = match arguments {
            Value::Null => None,
            Value::Object(map) => Some(map),
            _ => return Err("mcp tool arguments must be a JSON object".to_string()),
        };
        self.service
            .call_tool(CallToolRequestParam {
                name: tool_name.to_string().into(),
                arguments,
            })
            .await
            .map_err(format_service_error)
    }

    pub async fn list_tools(&self) -> Result<ListToolsResult, String> {
        self.service
            .list_tools(None)
            .await
            .map_err(format_service_error)
    }

    pub async fn shutdown(self) {
        let _ = self.service.cancel().await;
    }
}

fn format_service_error(err: ServiceError) -> String {
    match err {
        ServiceError::McpError(data) => match data.data {
            Some(value) => format!("mcp error: {} ({value})", data.message),
            None => format!("mcp error: {}", data.message),
        },
        _ => err.to_string(),
    }
}
