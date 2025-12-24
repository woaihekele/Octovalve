use anyhow::Context;
use async_trait::async_trait;
use bytes::Bytes;
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use protocol::{CommandMode, CommandRequest, CommandResponse, CommandStage, CommandStatus};
use rust_mcp_sdk::mcp_server::server_runtime;
use rust_mcp_sdk::mcp_server::ServerHandler;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use rust_mcp_sdk::schema::{
    CallToolRequest, CallToolResult, ContentBlock, Implementation, InitializeResult,
    ListToolsRequest, ListToolsResult, ServerCapabilities, ServerCapabilitiesTools, TextContent,
    Tool, ToolAnnotations, ToolInputSchema, LATEST_PROTOCOL_VERSION,
};
use rust_mcp_sdk::{McpServer, StdioTransport, TransportOptions};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::io;
use tokio::net::TcpStream;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tracing_subscriber::prelude::*;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(
    name = "local-proxy",
    version,
    about = "MCP stdio proxy to remote command broker"
)]
struct Args {
    #[arg(long, default_value = "127.0.0.1:19306")]
    remote_addr: String,
    #[arg(long, default_value = "local-proxy")]
    client_id: String,
    #[arg(long, default_value_t = 30_000)]
    timeout_ms: u64,
    #[arg(long, default_value_t = 1024 * 1024)]
    max_output_bytes: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let args = Args::parse();

    let server_details = InitializeResult {
        server_info: Implementation {
            name: "remote_cmd_local_proxy".to_string(),
            version: "0.1.0".to_string(),
            title: Some("Remote Command Local Proxy".to_string()),
        },
        capabilities: ServerCapabilities {
            tools: Some(ServerCapabilitiesTools { list_changed: None }),
            ..Default::default()
        },
        meta: None,
        instructions: Some(
            "Use run_command to execute whitelisted commands on the remote host after approval."
                .to_string(),
        ),
        protocol_version: LATEST_PROTOCOL_VERSION.to_string(),
    };

    let transport = StdioTransport::new(TransportOptions::default())
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let handler = ProxyHandler::new(args);
    let server = server_runtime::create_server(server_details, transport, handler);
    server
        .start()
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    Ok(())
}

fn init_tracing() {
    let layer = tracing_subscriber::fmt::layer()
        .with_writer(io::stderr)
        .with_target(false);
    tracing_subscriber::registry().with(layer).init();
}

struct ProxyHandler {
    remote_addr: String,
    client_id: String,
    default_timeout_ms: u64,
    default_max_output_bytes: u64,
}

impl ProxyHandler {
    fn new(args: Args) -> Self {
        Self {
            remote_addr: args.remote_addr,
            client_id: args.client_id,
            default_timeout_ms: args.timeout_ms,
            default_max_output_bytes: args.max_output_bytes,
        }
    }

    fn tool_definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "command".to_string(),
            json!({
                "type": "string",
                "description": "Shell-like command line. Default mode executes via /bin/bash -lc."
            })
            .as_object()
            .cloned()
            .unwrap_or_default(),
        );
        properties.insert(
            "intent".to_string(),
            json!({
                "type": "string",
                "description": "Why this command is needed (required for audit)."
            })
            .as_object()
            .cloned()
            .unwrap_or_default(),
        );
        properties.insert(
            "mode".to_string(),
            json!({
                "type": "string",
                "enum": ["shell", "argv"],
                "default": "shell",
                "description": "Execution mode: shell uses /bin/bash -lc, argv uses parsed pipeline."
            })
            .as_object()
            .cloned()
            .unwrap_or_default(),
        );
        properties.insert(
            "cwd".to_string(),
            json!({
                "type": "string",
                "description": "Working directory for the command."
            })
            .as_object()
            .cloned()
            .unwrap_or_default(),
        );
        properties.insert(
            "timeout_ms".to_string(),
            json!({
                "type": "integer",
                "minimum": 0,
                "description": "Override command timeout in milliseconds."
            })
            .as_object()
            .cloned()
            .unwrap_or_default(),
        );
        properties.insert(
            "max_output_bytes".to_string(),
            json!({
                "type": "integer",
                "minimum": 0,
                "description": "Override output size limit in bytes."
            })
            .as_object()
            .cloned()
            .unwrap_or_default(),
        );
        properties.insert(
            "env".to_string(),
            json!({
                "type": "object",
                "additionalProperties": { "type": "string" },
                "description": "Extra environment variables."
            })
            .as_object()
            .cloned()
            .unwrap_or_default(),
        );

        let input_schema = ToolInputSchema::new(
            vec!["command".to_string(), "intent".to_string()],
            Some(properties),
        );

        Tool {
            name: "run_command".to_string(),
            description: Some(
                "Forward command execution to a remote broker with manual approval.".to_string(),
            ),
            input_schema,
            output_schema: None,
            meta: None,
            title: Some("Run Command".to_string()),
            annotations: Some(ToolAnnotations {
                read_only_hint: Some(false),
                destructive_hint: Some(true),
                open_world_hint: Some(false),
                idempotent_hint: Some(false),
                title: Some("Run Command".to_string()),
            }),
        }
    }
}

#[async_trait]
impl ServerHandler for ProxyHandler {
    async fn handle_list_tools_request(
        &self,
        _: ListToolsRequest,
        _: std::sync::Arc<dyn rust_mcp_sdk::McpServer>,
    ) -> Result<ListToolsResult, rust_mcp_sdk::schema::RpcError> {
        Ok(ListToolsResult {
            tools: vec![self.tool_definition()],
            next_cursor: None,
            meta: None,
        })
    }

    async fn handle_call_tool_request(
        &self,
        request: CallToolRequest,
        _: std::sync::Arc<dyn rust_mcp_sdk::McpServer>,
    ) -> Result<CallToolResult, CallToolError> {
        if request.params.name != "run_command" {
            return Ok(CallToolError::unknown_tool(request.params.name).into());
        }

        let args = parse_arguments(request.params.arguments)
            .map_err(|err| CallToolError::invalid_arguments("run_command", Some(err)))?;
        let pipeline = parse_pipeline(&args.command)
            .map_err(|err| CallToolError::invalid_arguments("run_command", Some(err)))?;

        let mode = args.mode.unwrap_or(CommandMode::Shell);
        let request = CommandRequest {
            id: Uuid::new_v4().to_string(),
            client: self.client_id.clone(),
            intent: args.intent,
            mode,
            raw_command: args.command.clone(),
            cwd: args.cwd,
            env: args.env,
            timeout_ms: Some(args.timeout_ms.unwrap_or(self.default_timeout_ms)),
            max_output_bytes: Some(
                args.max_output_bytes
                    .unwrap_or(self.default_max_output_bytes),
            ),
            pipeline,
        };

        let response = match send_request(&self.remote_addr, &request).await {
            Ok(response) => response,
            Err(err) => CommandResponse::error(request.id.clone(), err.to_string()),
        };

        Ok(response_to_tool_result(response))
    }
}

#[derive(Debug, Deserialize)]
struct RunCommandArgs {
    command: String,
    intent: String,
    mode: Option<CommandMode>,
    cwd: Option<String>,
    timeout_ms: Option<u64>,
    max_output_bytes: Option<u64>,
    env: Option<BTreeMap<String, String>>,
}

fn parse_arguments(args: Option<Map<String, Value>>) -> Result<RunCommandArgs, String> {
    let map = args.ok_or_else(|| "missing arguments".to_string())?;
    serde_json::from_value(Value::Object(map)).map_err(|err| err.to_string())
}

fn parse_pipeline(command: &str) -> Result<Vec<CommandStage>, String> {
    let tokens = shell_words::split(command).map_err(|err| err.to_string())?;
    if tokens.is_empty() {
        return Err("command is empty".to_string());
    }
    let mut pipeline = Vec::new();
    let mut current = Vec::new();
    for token in tokens {
        if token == "|" {
            if current.is_empty() {
                return Err("empty pipeline segment".to_string());
            }
            pipeline.push(CommandStage { argv: current });
            current = Vec::new();
        } else {
            current.push(token);
        }
    }
    if current.is_empty() {
        return Err("trailing pipe".to_string());
    }
    pipeline.push(CommandStage { argv: current });
    Ok(pipeline)
}

async fn send_request(addr: &str, request: &CommandRequest) -> anyhow::Result<CommandResponse> {
    let stream = TcpStream::connect(addr)
        .await
        .with_context(|| format!("failed to connect to {addr}"))?;
    let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
    let payload = serde_json::to_vec(request)?;
    framed.send(Bytes::from(payload)).await?;

    let response = framed
        .next()
        .await
        .context("connection closed")?
        .context("read response")?;
    let response: CommandResponse = serde_json::from_slice(&response)?;
    Ok(response)
}

fn response_to_tool_result(response: CommandResponse) -> CallToolResult {
    let status = format!("status: {:?}", response.status);
    let mut message = vec![status];
    if let Some(code) = response.exit_code {
        message.push(format!("exit_code: {code}"));
    }
    if let Some(stdout) = response.stdout.as_ref() {
        message.push(format!("stdout: {stdout}"));
    }
    if let Some(stderr) = response.stderr.as_ref() {
        message.push(format!("stderr: {stderr}"));
    }
    if let Some(error) = response.error.as_ref() {
        message.push(format!("error: {error}"));
    }

    let text = message.join("\n");
    let mut structured = serde_json::to_value(&response)
        .ok()
        .and_then(|value| value.as_object().cloned());

    if matches!(
        response.status,
        CommandStatus::Error | CommandStatus::Denied
    ) {
        structured
            .get_or_insert_with(Map::new)
            .insert("is_error".to_string(), Value::Bool(true));
    }

    CallToolResult {
        content: vec![ContentBlock::from(TextContent::new(text, None, None))],
        is_error: Some(matches!(
            response.status,
            CommandStatus::Denied | CommandStatus::Error
        )),
        meta: None,
        structured_content: structured,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_command() {
        let pipeline = parse_pipeline("ls -l").expect("parse");
        assert_eq!(pipeline.len(), 1);
        assert_eq!(pipeline[0].argv, vec!["ls".to_string(), "-l".to_string()]);
    }

    #[test]
    fn parse_pipeline_command() {
        let pipeline = parse_pipeline("ls | grep foo").expect("parse");
        assert_eq!(pipeline.len(), 2);
        assert_eq!(pipeline[0].argv, vec!["ls".to_string()]);
        assert_eq!(
            pipeline[1].argv,
            vec!["grep".to_string(), "foo".to_string()]
        );
    }

    #[test]
    fn parse_rejects_empty_segment() {
        let err = parse_pipeline("ls | | grep foo").unwrap_err();
        assert!(err.contains("empty pipeline segment"));
    }
}
