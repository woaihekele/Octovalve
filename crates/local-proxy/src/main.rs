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
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};
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
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long, default_value = "local-proxy")]
    client_id: String,
    #[arg(long, default_value_t = 30_000)]
    timeout_ms: u64,
    #[arg(long, default_value_t = 1024 * 1024)]
    max_output_bytes: u64,
}

const DEFAULT_REMOTE_ADDR: &str = "127.0.0.1:19307";
const DEFAULT_BIND_HOST: &str = "127.0.0.1";

#[derive(Debug, Deserialize)]
struct ProxyConfig {
    default_target: Option<String>,
    defaults: Option<ProxyDefaults>,
    targets: Vec<TargetConfig>,
}

#[derive(Debug, Deserialize)]
struct ProxyDefaults {
    timeout_ms: Option<u64>,
    max_output_bytes: Option<u64>,
    local_bind: Option<String>,
    remote_addr: Option<String>,
    ssh_args: Option<Vec<String>>,
    ssh_password: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TargetConfig {
    name: String,
    desc: String,
    ssh: String,
    remote_addr: Option<String>,
    local_port: u16,
    local_bind: Option<String>,
    ssh_args: Option<Vec<String>>,
    ssh_password: Option<String>,
}

impl Default for ProxyDefaults {
    fn default() -> Self {
        Self {
            timeout_ms: None,
            max_output_bytes: None,
            local_bind: None,
            remote_addr: None,
            ssh_args: None,
            ssh_password: None,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let args = Args::parse();
    let (state, defaults) = build_proxy_state(&args)?;

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
            "Use run_command to execute commands on a target after approval. target is required. Use list_targets to see available targets."
                .to_string(),
        ),
        protocol_version: LATEST_PROTOCOL_VERSION.to_string(),
    };

    let transport = StdioTransport::new(TransportOptions::default())
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let handler = ProxyHandler::new(state, args.client_id, defaults);
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

struct ProxyRuntimeDefaults {
    timeout_ms: u64,
    max_output_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TargetStatus {
    Ready,
    Down,
}

impl TargetStatus {
    fn as_str(&self) -> &'static str {
        match self {
            TargetStatus::Ready => "ready",
            TargetStatus::Down => "down",
        }
    }
}

struct TargetRuntime {
    name: String,
    desc: String,
    ssh: Option<String>,
    ssh_args: Vec<String>,
    ssh_password: Option<String>,
    remote_addr: String,
    local_bind: Option<String>,
    local_port: Option<u16>,
    local_addr: String,
    status: TargetStatus,
    last_seen: Option<SystemTime>,
    last_error: Option<String>,
    tunnel: Option<tokio::process::Child>,
}

struct ProxyState {
    targets: HashMap<String, TargetRuntime>,
    target_order: Vec<String>,
    default_target: Option<String>,
}

#[derive(Serialize)]
struct TargetListEntry {
    name: String,
    desc: String,
    status: String,
    last_seen: Option<String>,
    ssh: Option<String>,
    remote_addr: String,
    local_addr: String,
}

impl TargetRuntime {
    fn refresh_status(&mut self) {
        if let Some(child) = self.tunnel.as_mut() {
            match child.try_wait() {
                Ok(None) => {
                    self.status = TargetStatus::Ready;
                }
                Ok(Some(status)) => {
                    self.status = TargetStatus::Down;
                    self.tunnel = None;
                    self.last_error = Some(format!("ssh exited: {status}"));
                }
                Err(err) => {
                    self.status = TargetStatus::Down;
                    self.last_error = Some(format!("ssh status check failed: {err}"));
                }
            }
        } else if self.ssh.is_some() {
            self.status = TargetStatus::Down;
        } else {
            self.status = TargetStatus::Ready;
        }
    }
}

impl ProxyState {
    fn target_names(&self) -> Vec<String> {
        self.target_order.clone()
    }

    fn refresh_statuses(&mut self) {
        for name in &self.target_order {
            if let Some(target) = self.targets.get_mut(name) {
                target.refresh_status();
            }
        }
    }

    fn list_targets(&mut self) -> Vec<TargetListEntry> {
        self.refresh_statuses();
        self.target_order
            .iter()
            .filter_map(|name| self.targets.get(name))
            .map(|target| TargetListEntry {
                name: target.name.clone(),
                desc: target.desc.clone(),
                status: target.status.as_str().to_string(),
                last_seen: target.last_seen.map(format_time),
                ssh: target.ssh.clone(),
                remote_addr: target.remote_addr.clone(),
                local_addr: target.local_addr.clone(),
            })
            .collect()
    }

    fn ensure_tunnel(&mut self, name: &str) -> anyhow::Result<String> {
        let target = self
            .targets
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("unknown target: {name}"))?;
        target.refresh_status();
        if target.ssh.is_some() && target.status != TargetStatus::Ready {
            spawn_tunnel(target)?;
        }
        Ok(target.local_addr.clone())
    }

    fn note_success(&mut self, name: &str) {
        if let Some(target) = self.targets.get_mut(name) {
            target.last_seen = Some(SystemTime::now());
            target.status = TargetStatus::Ready;
            target.last_error = None;
        }
    }

    fn note_failure(&mut self, name: &str, err: &str) {
        if let Some(target) = self.targets.get_mut(name) {
            target.status = TargetStatus::Down;
            target.last_error = Some(err.to_string());
        }
    }
}

fn format_time(time: SystemTime) -> String {
    humantime::format_rfc3339(time).to_string()
}

fn build_proxy_state(args: &Args) -> anyhow::Result<(ProxyState, ProxyRuntimeDefaults)> {
    if let Some(path) = &args.config {
        let config = load_proxy_config(path)?;
        build_state_from_config(args, config)
    } else {
        build_state_from_args(args)
    }
}

fn build_state_from_args(args: &Args) -> anyhow::Result<(ProxyState, ProxyRuntimeDefaults)> {
    let target = TargetRuntime {
        name: "default".to_string(),
        desc: "default remote".to_string(),
        ssh: None,
        ssh_args: Vec::new(),
        ssh_password: None,
        remote_addr: args.remote_addr.clone(),
        local_bind: None,
        local_port: None,
        local_addr: args.remote_addr.clone(),
        status: TargetStatus::Ready,
        last_seen: None,
        last_error: None,
        tunnel: None,
    };
    let mut targets = HashMap::new();
    targets.insert(target.name.clone(), target);
    let state = ProxyState {
        targets,
        target_order: vec!["default".to_string()],
        default_target: Some("default".to_string()),
    };
    let defaults = ProxyRuntimeDefaults {
        timeout_ms: args.timeout_ms,
        max_output_bytes: args.max_output_bytes,
    };
    Ok((state, defaults))
}

fn load_proxy_config(path: &PathBuf) -> anyhow::Result<ProxyConfig> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let config: ProxyConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse config {}", path.display()))?;
    if config.targets.is_empty() {
        anyhow::bail!("config must include at least one target");
    }
    Ok(config)
}

fn build_state_from_config(
    args: &Args,
    config: ProxyConfig,
) -> anyhow::Result<(ProxyState, ProxyRuntimeDefaults)> {
    let defaults = config.defaults.unwrap_or_default();
    let default_remote = defaults
        .remote_addr
        .unwrap_or_else(|| DEFAULT_REMOTE_ADDR.to_string());
    let default_bind = defaults
        .local_bind
        .unwrap_or_else(|| DEFAULT_BIND_HOST.to_string());
    let default_ssh_args = defaults.ssh_args.unwrap_or_default();
    let default_ssh_password = defaults.ssh_password.clone();

    let timeout_ms = defaults.timeout_ms.unwrap_or(args.timeout_ms);
    let max_output_bytes = defaults.max_output_bytes.unwrap_or(args.max_output_bytes);

    let mut targets = HashMap::new();
    let mut order = Vec::new();

    for target in config.targets {
        if target.name.trim().is_empty() {
            anyhow::bail!("target name cannot be empty");
        }
        if targets.contains_key(&target.name) {
            anyhow::bail!("duplicate target name: {}", target.name);
        }
        if target.ssh.trim().is_empty() {
            anyhow::bail!("target {} ssh cannot be empty", target.name);
        }
        if target.ssh.split_whitespace().count() > 1 {
            anyhow::bail!(
                "target {} ssh must be a single destination; use ssh_args for options",
                target.name
            );
        }
        let remote_addr = target
            .remote_addr
            .unwrap_or_else(|| default_remote.clone());
        let local_bind = target
            .local_bind
            .unwrap_or_else(|| default_bind.clone());
        let local_addr = format!("{local_bind}:{}", target.local_port);

        let mut ssh_args = default_ssh_args.clone();
        if let Some(extra) = target.ssh_args {
            ssh_args.extend(extra);
        }
        let ssh_password = target
            .ssh_password
            .or_else(|| default_ssh_password.clone());

        let mut runtime = TargetRuntime {
            name: target.name.clone(),
            desc: target.desc,
            ssh: Some(target.ssh),
            ssh_args,
            ssh_password,
            remote_addr,
            local_bind: Some(local_bind),
            local_port: Some(target.local_port),
            local_addr,
            status: TargetStatus::Down,
            last_seen: None,
            last_error: None,
            tunnel: None,
        };

        if let Err(err) = spawn_tunnel(&mut runtime) {
            runtime.status = TargetStatus::Down;
            runtime.last_error = Some(err.to_string());
        }

        order.push(runtime.name.clone());
        targets.insert(runtime.name.clone(), runtime);
    }

    if let Some(default_target) = config.default_target.as_ref() {
        if !targets.contains_key(default_target) {
            anyhow::bail!("default_target {} not found in targets", default_target);
        }
    }

    let state = ProxyState {
        targets,
        target_order: order,
        default_target: config.default_target,
    };

    let defaults = ProxyRuntimeDefaults {
        timeout_ms,
        max_output_bytes,
    };
    Ok((state, defaults))
}

fn parse_host_port(addr: &str) -> anyhow::Result<(String, u16)> {
    let (host, port) = addr
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("invalid address {addr}, expected host:port"))?;
    let port = port
        .parse::<u16>()
        .with_context(|| format!("invalid port in address {addr}"))?;
    Ok((host.to_string(), port))
}

fn spawn_tunnel(target: &mut TargetRuntime) -> anyhow::Result<()> {
    if target.ssh.is_none() {
        target.status = TargetStatus::Ready;
        return Ok(());
    }
    let bind = target
        .local_bind
        .as_ref()
        .context("missing local_bind")?;
    let port = target.local_port.context("missing local_port")?;
    let (remote_host, remote_port) = parse_host_port(&target.remote_addr)?;

    let mut cmd = if let Some(password) = target.ssh_password.as_ref() {
        let mut cmd = Command::new("sshpass");
        cmd.arg("-e");
        cmd.env("SSHPASS", password);
        cmd.arg("ssh");
        cmd
    } else {
        Command::new("ssh")
    };
    cmd.arg("-N")
        .arg("-T")
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-o")
        .arg("ServerAliveInterval=30")
        .arg("-o")
        .arg("ServerAliveCountMax=3")
        .arg("-L")
        .arg(format!("{bind}:{port}:{remote_host}:{remote_port}"));
    if target.ssh_password.is_none() {
        cmd.arg("-o").arg("BatchMode=yes");
    }

    if !target.ssh_args.is_empty() {
        cmd.args(&target.ssh_args);
    }

    cmd.arg(
        target
            .ssh
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing ssh target"))?,
    );
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let child = cmd
        .spawn()
        .map_err(|err| {
            if target.ssh_password.is_some() && err.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!("sshpass not found; install sshpass or remove ssh_password")
            } else {
                anyhow::anyhow!(err)
            }
        })
        .context("failed to spawn ssh tunnel")?;
    target.tunnel = Some(child);
    target.status = TargetStatus::Ready;
    Ok(())
}

struct ProxyHandler {
    state: Arc<RwLock<ProxyState>>,
    client_id: String,
    default_timeout_ms: u64,
    default_max_output_bytes: u64,
}

impl ProxyHandler {
    fn new(state: ProxyState, client_id: String, defaults: ProxyRuntimeDefaults) -> Self {
        Self {
            state: Arc::new(RwLock::new(state)),
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
            (state.target_names(), state.default_target.clone())
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
                let args = parse_arguments(request.params.arguments).map_err(|err| {
                    CallToolError::invalid_arguments("run_command", Some(err))
                })?;
                let pipeline = parse_pipeline(&args.command).map_err(|err| {
                    CallToolError::invalid_arguments("run_command", Some(err))
                })?;

                let addr = {
                    let mut state = self.state.write().await;
                    state
                        .ensure_tunnel(&args.target)
                        .map_err(|err| {
                            CallToolError::invalid_arguments(
                                "run_command",
                                Some(err.to_string()),
                            )
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
                        CommandStatus::Completed | CommandStatus::Denied | CommandStatus::Approved => {
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

    #[test]
    fn config_requires_desc() {
        let input = r#"
[[targets]]
name = "dev"
ssh = "devops@127.0.0.1"
local_port = 19311
"#;
        let parsed: Result<ProxyConfig, _> = toml::from_str(input);
        assert!(parsed.is_err());
    }

    #[test]
    fn build_state_from_args_sets_default_target() {
        let args = Args {
            remote_addr: "127.0.0.1:19306".to_string(),
            config: None,
            client_id: "local-proxy".to_string(),
            timeout_ms: 10,
            max_output_bytes: 20,
        };
        let (state, defaults) = build_state_from_args(&args).expect("state");
        assert!(state.targets.contains_key("default"));
        assert_eq!(defaults.timeout_ms, 10);
        assert_eq!(defaults.max_output_bytes, 20);
    }
}
