use crate::state::{ProxyRuntimeDefaults, ProxyState, TargetListEntry};
use anyhow::Context;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use protocol::{CommandMode, CommandRequest, CommandResponse, CommandStage, CommandStatus};
use rust_mcp_sdk::mcp_server::ServerHandler;
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use rust_mcp_sdk::schema::{
    CallToolRequest, CallToolResult, ContentBlock, ListToolsRequest, ListToolsResult, TextContent,
    Tool, ToolAnnotations, ToolInputSchema,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use uuid::Uuid;

pub(crate) struct ProxyHandler {
    state: Arc<RwLock<ProxyState>>,
    client_id: String,
    default_timeout_ms: u64,
    default_max_output_bytes: u64,
}

impl ProxyHandler {
    pub(crate) fn new(
        state: Arc<RwLock<ProxyState>>,
        client_id: String,
        defaults: ProxyRuntimeDefaults,
    ) -> Self {
        Self {
            state,
            client_id,
            default_timeout_ms: defaults.timeout_ms,
            default_max_output_bytes: defaults.max_output_bytes,
        }
    }

    fn tool_definition(&self, targets: &[String], default_target: Option<&String>) -> Tool {
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
        let mut target_schema = json!({
            "type": "string",
            "enum": targets,
            "description": "Target name defined in local-proxy config."
        });
        if let Some(default) = default_target {
            target_schema["default"] = json!(default);
        }
        properties.insert(
            "target".to_string(),
            target_schema.as_object().cloned().unwrap_or_default(),
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
            vec![
                "command".to_string(),
                "intent".to_string(),
                "target".to_string(),
            ],
            Some(properties),
        );

        Tool {
            name: "run_command".to_string(),
            description: Some(
                "Forward command execution to a remote broker with manual approval. When searching for text or files, prefer using `rg` or `rg --files` respectively because `rg` is much faster than alternatives like `grep`. (If the `rg` command is not found, then use alternatives.)".to_string(),
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

    fn list_targets_definition(&self) -> Tool {
        let input_schema = ToolInputSchema::new(Vec::new(), Some(HashMap::new()));
        Tool {
            name: "list_targets".to_string(),
            description: Some("List available targets configured in local-proxy.".to_string()),
            input_schema,
            output_schema: None,
            meta: None,
            title: Some("List Targets".to_string()),
            annotations: Some(ToolAnnotations {
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                open_world_hint: Some(false),
                idempotent_hint: Some(true),
                title: Some("List Targets".to_string()),
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
        let (targets, default_target) = {
            let state = self.state.read().await;
            (state.target_names(), state.default_target())
        };
        Ok(ListToolsResult {
            tools: vec![
                self.tool_definition(&targets, default_target.as_ref()),
                self.list_targets_definition(),
            ],
            next_cursor: None,
            meta: None,
        })
    }

    async fn handle_call_tool_request(
        &self,
        request: CallToolRequest,
        _: std::sync::Arc<dyn rust_mcp_sdk::McpServer>,
    ) -> Result<CallToolResult, CallToolError> {
        match request.params.name.as_str() {
            "run_command" => {
                let args = parse_arguments(request.params.arguments)
                    .map_err(|err| CallToolError::invalid_arguments("run_command", Some(err)))?;
                let pipeline = parse_pipeline(&args.command)
                    .map_err(|err| CallToolError::invalid_arguments("run_command", Some(err)))?;

                let addr = {
                    let mut state = self.state.write().await;
                    state.ensure_tunnel(&args.target).map_err(|err| {
                        CallToolError::invalid_arguments("run_command", Some(err.to_string()))
                    })?
                };

                let mode = args.mode.unwrap_or(CommandMode::Shell);
                let request = CommandRequest {
                    id: Uuid::new_v4().to_string(),
                    client: self.client_id.clone(),
                    target: args.target.clone(),
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

                let response = match send_request(&addr, &request).await {
                    Ok(response) => response,
                    Err(err) => CommandResponse::error(request.id.clone(), err.to_string()),
                };

                {
                    let mut state = self.state.write().await;
                    match response.status {
                        CommandStatus::Completed
                        | CommandStatus::Denied
                        | CommandStatus::Approved => {
                            state.note_success(&request.target);
                        }
                        CommandStatus::Error => {
                            if let Some(error) = response.error.as_ref() {
                                state.note_failure(&request.target, error);
                            }
                        }
                    }
                }

                Ok(response_to_tool_result(response))
            }
            "list_targets" => {
                let targets = {
                    let mut state = self.state.write().await;
                    state.list_targets()
                };
                Ok(targets_to_tool_result(targets))
            }
            _ => Ok(CallToolError::unknown_tool(request.params.name).into()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RunCommandArgs {
    command: String,
    intent: String,
    target: String,
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
    let mut last_err = None;
    for attempt in 0..3 {
        match TcpStream::connect(addr).await {
            Ok(stream) => {
                let mut framed = Framed::new(stream, LengthDelimitedCodec::new());
                let payload = serde_json::to_vec(request)?;
                framed.send(Bytes::from(payload)).await?;

                let response = framed
                    .next()
                    .await
                    .context("connection closed")?
                    .context("read response")?;
                let response: CommandResponse = serde_json::from_slice(&response)?;
                return Ok(response);
            }
            Err(err) => {
                last_err = Some(err);
                if attempt < 2 {
                    sleep(Duration::from_millis(200)).await;
                }
            }
        }
    }

    let err = last_err
        .map(|err| anyhow::anyhow!(err))
        .unwrap_or_else(|| anyhow::anyhow!("failed to connect to {addr}"));
    Err(err).with_context(|| format!("failed to connect to {addr}"))
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

fn targets_to_tool_result(targets: Vec<TargetListEntry>) -> CallToolResult {
    let payload = json!({ "targets": targets });
    let text = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string());
    CallToolResult {
        content: vec![ContentBlock::from(TextContent::new(text, None, None))],
        is_error: Some(false),
        meta: None,
        structured_content: payload.as_object().cloned(),
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
