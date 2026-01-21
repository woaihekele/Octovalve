use crate::state::{ProxyRuntimeDefaults, ProxyState, TargetListEntry};
use anyhow::Context;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use protocol::{CommandMode, CommandRequest, CommandResponse, CommandStage, CommandStatus};
use rmcp::{
    model::{
        CallToolRequestParam, CallToolResult, Content, JsonObject, ListToolsResult,
        PaginatedRequestParam, ServerInfo, Tool, ToolAnnotations,
    },
    ErrorData as McpError, RoleServer, ServerHandler,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
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
    server_info: ServerInfo,
}

impl ProxyHandler {
    pub(crate) fn new(
        state: Arc<RwLock<ProxyState>>,
        client_id: String,
        defaults: ProxyRuntimeDefaults,
        server_info: ServerInfo,
    ) -> Self {
        Self {
            state,
            client_id,
            default_timeout_ms: defaults.timeout_ms,
            default_max_output_bytes: defaults.max_output_bytes,
            server_info,
        }
    }

    fn tool_definition(&self, targets: &[String], default_target: Option<&String>) -> Tool {
        let mut properties = Map::new();
        properties.insert(
            "command".to_string(),
            json!({
                "type": "string",
                "description": "Shell-like command line. Default mode executes via /bin/bash -lc."
            }),
        );
        let mut target_schema = json!({
            "type": "string",
            "enum": targets,
            "description": "Target name defined in octovalve-proxy config."
        });
        if let Some(default) = default_target {
            target_schema["default"] = json!(default);
        }
        properties.insert("target".to_string(), target_schema);
        properties.insert(
            "intent".to_string(),
            json!({
                "type": "string",
                "description": "Why this command is needed (required for audit)."
            }),
        );
        properties.insert(
            "mode".to_string(),
            json!({
                "type": "string",
                "enum": ["shell"],
                "default": "shell",
                "description": "Execution mode: shell uses /bin/bash -lc."
            }),
        );
        properties.insert(
            "cwd".to_string(),
            json!({
                "type": "string",
                "description": "Working directory for the command."
            }),
        );
        properties.insert(
            "timeout_ms".to_string(),
            json!({
                "type": "integer",
                "minimum": 0,
                "description": "Override command timeout in milliseconds."
            }),
        );
        properties.insert(
            "max_output_bytes".to_string(),
            json!({
                "type": "integer",
                "minimum": 0,
                "description": "Override output size limit in bytes."
            }),
        );
        properties.insert(
            "env".to_string(),
            json!({
                "type": "object",
                "additionalProperties": { "type": "string" },
                "description": "Extra environment variables."
            }),
        );

        let mut input_schema = Map::new();
        input_schema.insert("type".to_string(), Value::String("object".to_string()));
        // When there's a default target, target is not required
        let required = if default_target.is_some() {
            json!(["command", "intent"])
        } else {
            json!(["command", "intent", "target"])
        };
        input_schema.insert("required".to_string(), required);
        input_schema.insert("properties".to_string(), Value::Object(properties));

        Tool {
            name: "run_command".into(),
            description: Some(
                "Forward command execution to the console executor with manual approval. When searching for text or files, prefer using `rg` or `rg --files` respectively because `rg` is much faster than alternatives like `grep`. (If the `rg` command is not found, then use alternatives.)".into(),
            ),
            input_schema: Arc::new(input_schema),
            output_schema: None,
            title: Some("Run Command".to_string()),
            annotations: Some(ToolAnnotations {
                read_only_hint: Some(false),
                destructive_hint: Some(true),
                open_world_hint: Some(false),
                idempotent_hint: Some(false),
                title: Some("Run Command".to_string()),
            }),
            icons: None,
        }
    }

    fn list_targets_definition(&self) -> Tool {
        let mut input_schema = Map::new();
        input_schema.insert("type".to_string(), Value::String("object".to_string()));
        input_schema.insert("properties".to_string(), Value::Object(Map::new()));
        Tool {
            name: "list_targets".into(),
            description: Some("List available targets configured in octovalve-proxy.".into()),
            input_schema: Arc::new(input_schema),
            output_schema: None,
            title: Some("List Targets".to_string()),
            annotations: Some(ToolAnnotations {
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                open_world_hint: Some(false),
                idempotent_hint: Some(true),
                title: Some("List Targets".to_string()),
            }),
            icons: None,
        }
    }
}

impl ServerHandler for ProxyHandler {
    fn get_info(&self) -> ServerInfo {
        self.server_info.clone()
    }

    fn list_tools(
        &self,
        _: Option<PaginatedRequestParam>,
        _: rmcp::service::RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        async move {
            let (targets, default_target) = {
                let state = self.state.read().await;
                (state.target_names(), state.default_target())
            };
            Ok(ListToolsResult::with_all_items(vec![
                self.tool_definition(&targets, default_target.as_ref()),
                self.list_targets_definition(),
            ]))
        }
    }

    fn call_tool(
        &self,
        request: CallToolRequestParam,
        _: rmcp::service::RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, McpError>> + Send + '_ {
        async move {
            match request.name.as_ref() {
                "run_command" => {
                    let args = parse_arguments(request.arguments)
                        .map_err(|err| McpError::invalid_params(err, None))?;
                    let pipeline = parse_pipeline(&args.command)
                        .map_err(|err| McpError::invalid_params(err, None))?;

                    let (target, addr) = {
                        let state = self.state.read().await;
                        let target = args
                            .target
                            .or_else(|| state.default_target())
                            .ok_or_else(|| McpError::invalid_params("target is required", None))?;
                        let addr = state
                            .target_addr(&target)
                            .map_err(|err| McpError::invalid_params(err.to_string(), None))?;
                        (target, addr)
                    };

                    let mode = args.mode.unwrap_or(CommandMode::Shell);
                    let request = CommandRequest {
                        id: Uuid::new_v4().to_string(),
                        client: self.client_id.clone(),
                        target: target.clone(),
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
                            | CommandStatus::Approved
                            | CommandStatus::Cancelled => {
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
                _ => Err(McpError::invalid_params(
                    format!("unknown tool: {}", request.name),
                    None,
                )),
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct RunCommandArgs {
    command: String,
    intent: String,
    target: Option<String>,
    mode: Option<CommandMode>,
    cwd: Option<String>,
    timeout_ms: Option<u64>,
    max_output_bytes: Option<u64>,
    env: Option<BTreeMap<String, String>>,
}

fn parse_arguments(args: Option<JsonObject>) -> Result<RunCommandArgs, String> {
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
                let codec = LengthDelimitedCodec::builder()
                    .max_frame_length(protocol::framing::MAX_FRAME_LENGTH)
                    .new_codec();
                let mut framed = Framed::new(stream, codec);
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
    let id = format!("id: {}", response.id);
    let status = format!("status: {:?}", response.status);
    let mut message = vec![id, status];
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
    let mut structured = serde_json::to_value(&response).ok();

    if matches!(
        response.status,
        CommandStatus::Error | CommandStatus::Denied | CommandStatus::Cancelled
    ) {
        if let Some(Value::Object(map)) = structured.as_mut() {
            map.insert("is_error".to_string(), Value::Bool(true));
        }
    }

    CallToolResult {
        content: vec![Content::text(text)],
        is_error: Some(matches!(
            response.status,
            CommandStatus::Denied | CommandStatus::Error | CommandStatus::Cancelled
        )),
        meta: None,
        structured_content: structured,
    }
}

fn targets_to_tool_result(targets: Vec<TargetListEntry>) -> CallToolResult {
    let payload = json!({ "targets": targets });
    let text = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string());
    CallToolResult {
        content: vec![Content::text(text)],
        is_error: Some(false),
        meta: None,
        structured_content: Some(payload),
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
