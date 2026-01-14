use rmcp::{
    model::{
        CallToolRequestParam, CallToolResult, ClientCapabilities, ClientInfo, Implementation,
        ListToolsResult, ProtocolVersion, Tool,
    },
    service::{RoleClient, RunningService, ServiceExt},
    transport::TokioChildProcess,
    ServiceError,
};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::process::Command as TokioCommand;
use tokio::sync::Mutex;

#[derive(Clone, Debug)]
pub struct McpServerSpec {
    pub name: String,
    pub command: PathBuf,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: Option<PathBuf>,
}

impl McpServerSpec {
    pub fn signature(&self) -> String {
        let mut env_pairs = self.env.iter().collect::<Vec<_>>();
        env_pairs.sort_by(|a, b| a.0.cmp(b.0));
        let env_text = env_pairs
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(";");
        let cwd = self
            .cwd
            .as_ref()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default();
        format!(
            "{}|{}|{}|{}",
            self.command.to_string_lossy(),
            self.args.join("|"),
            env_text,
            cwd
        )
    }
}

#[derive(Default)]
struct McpClientRegistry {
    servers: HashMap<String, McpServerSpec>,
    clients: HashMap<String, Arc<McpClient>>,
}

pub struct McpClientState(pub Mutex<McpClientRegistry>);

impl Default for McpClientState {
    fn default() -> Self {
        Self(Mutex::new(McpClientRegistry::default()))
    }
}

impl McpClientState {
    pub async fn set_servers(&self, servers: Vec<McpServerSpec>) -> Result<(), String> {
        let mut next = HashMap::new();
        for server in servers {
            next.insert(server.name.clone(), server);
        }

        let removed_clients = {
            let mut guard = self.0.lock().await;
            let mut removed = Vec::new();
            let existing_names = guard.clients.keys().cloned().collect::<Vec<_>>();
            for name in existing_names {
                if let Some(spec) = next.get(&name) {
                    let should_remove = guard
                        .clients
                        .get(&name)
                        .map(|client| !client.is_usable_for(spec))
                        .unwrap_or(false);
                    if should_remove {
                        if let Some(client) = guard.clients.remove(&name) {
                            removed.push(client);
                        }
                    }
                } else if let Some(client) = guard.clients.remove(&name) {
                    removed.push(client);
                }
            }
            guard.servers = next;
            removed
        };

        for client in removed_clients {
            client.shutdown().await;
        }

        Ok(())
    }

    pub async fn list_tools(&self) -> Result<Vec<(String, Tool)>, String> {
        let server_names = {
            let guard = self.0.lock().await;
            guard.servers.keys().cloned().collect::<Vec<_>>()
        };
        let mut tools = Vec::new();
        for server in server_names {
            let client = match self.get_or_start_client(&server).await {
                Ok(client) => client,
                Err(err) => {
                    eprintln!("[mcp] start client failed: {server} err={err}");
                    continue;
                }
            };
            let result = match client.list_tools().await {
                Ok(result) => result,
                Err(err) => {
                    eprintln!("[mcp] list_tools failed: {server} err={err}");
                    continue;
                }
            };
            for tool in result.tools {
                tools.push((server.clone(), tool));
            }
        }
        Ok(tools)
    }

    pub async fn call_tool(
        &self,
        server: &str,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value, String> {
        let client = self.get_or_start_client(server).await?;
        let result = client.call_tool(tool_name, arguments).await?;
        serde_json::to_value(&result).map_err(|err| err.to_string())
    }

    async fn get_or_start_client(&self, server: &str) -> Result<Arc<McpClient>, String> {
        let spec = {
            let guard = self.0.lock().await;
            guard
                .servers
                .get(server)
                .cloned()
                .ok_or_else(|| format!("mcp server not configured: {server}"))?
        };

        let existing = {
            let mut guard = self.0.lock().await;
            if let Some(client) = guard.clients.get(server) {
                if client.is_usable_for(&spec) {
                    return Ok(client.clone());
                }
            }
            guard.clients.remove(server)
        };

        if let Some(old) = existing {
            old.shutdown().await;
        }

        let client = Arc::new(McpClient::start_with_spec(&spec).await?);
        let mut guard = self.0.lock().await;
        guard.clients.insert(server.to_string(), Arc::clone(&client));
        Ok(client)
    }
}

pub struct McpClient {
    service: RunningService<RoleClient, ClientInfo>,
    spec_signature: String,
}

impl McpClient {
    pub async fn start_with_spec(spec: &McpServerSpec) -> Result<Self, String> {
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
        let mut command = TokioCommand::new(&spec.command);
        command.args(&spec.args);
        if !spec.env.is_empty() {
            command.envs(&spec.env);
        }
        if let Some(cwd) = &spec.cwd {
            command.current_dir(cwd);
        }
        let transport = TokioChildProcess::new(command).map_err(|err| err.to_string())?;
        let service = client_info
            .serve(transport)
            .await
            .map_err(|err| err.to_string())?;
        Ok(Self {
            service,
            spec_signature: spec.signature(),
        })
    }

    pub fn is_usable_for(&self, spec: &McpServerSpec) -> bool {
        self.spec_signature == spec.signature() && !self.service.is_transport_closed()
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

    pub async fn shutdown(&self) {
        self.service.cancellation_token().cancel();
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
