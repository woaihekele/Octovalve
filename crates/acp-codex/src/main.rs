use std::{
    collections::{HashMap, VecDeque},
    env,
    io::BufRead,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};

use anyhow::{anyhow, Result};
use base64::Engine;
use async_trait::async_trait;
use codex_app_server_protocol::{
    AddConversationListenerParams, AddConversationSubscriptionResponse, ApplyPatchApprovalResponse,
    ClientInfo, ClientNotification, ClientRequest, ExecCommandApprovalResponse, InitializeParams,
    InitializeResponse, InputItem, JSONRPCError, JSONRPCMessage, JSONRPCNotification,
    JSONRPCRequest, JSONRPCResponse, NewConversationParams, NewConversationResponse, RequestId,
    ResumeConversationParams, ResumeConversationResponse, SendUserMessageParams,
    SendUserMessageResponse, ServerRequest,
};
use codex_protocol::{
    ConversationId,
    config_types::SandboxMode as CodexSandboxMode,
    plan_tool::{StepStatus, UpdatePlanArgs},
    protocol::{
        AgentMessageDeltaEvent, AgentMessageEvent, AgentReasoningDeltaEvent, AgentReasoningEvent,
        AskForApproval as CodexAskForApproval, ErrorEvent, EventMsg, McpToolCallBeginEvent,
        McpToolCallEndEvent, PatchApplyBeginEvent, PatchApplyEndEvent, ReviewDecision,
        StreamErrorEvent, WebSearchBeginEvent, WebSearchEndEvent,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::{Mutex, mpsc, oneshot},
};
use uuid::Uuid;

const CODEX_BASE_COMMAND: &[&str] = &["npx", "-y", "@openai/codex@0.77.0", "app-server"];

#[derive(Debug, Clone)]
struct CliConfig {
    approval_policy: Option<String>,
    sandbox_mode: Option<String>,
    app_server_args: Vec<String>,
}

impl CliConfig {
    fn parse() -> Result<Self> {
        let mut approval_policy = None;
        let mut sandbox_mode = None;
        let mut app_server_args = Vec::new();
        let mut args = env::args().skip(1).peekable();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--approval-policy" | "--approval_policy" => {
                    let value = args
                        .next()
                        .ok_or_else(|| anyhow!("--approval-policy 缺少值"))?;
                    approval_policy = Some(value.replace('_', "-"));
                }
                "--sandbox-mode" | "--sandbox_mode" => {
                    let value = args
                        .next()
                        .ok_or_else(|| anyhow!("--sandbox-mode 缺少值"))?;
                    sandbox_mode = Some(value.replace('_', "-"));
                }
                "-c" | "--config" => {
                    let value = args
                        .next()
                        .ok_or_else(|| anyhow!("-c 缺少配置值"))?;
                    Self::apply_config_override(&value, &mut approval_policy, &mut sandbox_mode);
                    app_server_args.push(arg);
                    app_server_args.push(value);
                }
                _ => {
                    app_server_args.push(arg);
                }
            }
        }

        Ok(Self {
            approval_policy,
            sandbox_mode,
            app_server_args,
        })
    }

    fn apply_config_override(
        value: &str,
        approval_policy: &mut Option<String>,
        sandbox_mode: &mut Option<String>,
    ) {
        let (key, raw_value) = match value.split_once('=') {
            Some(pair) => pair,
            None => return,
        };
        let normalized_value = raw_value.trim().replace('_', "-");
        match key.trim() {
            "approval_policy" if approval_policy.is_none() => {
                *approval_policy = Some(normalized_value);
            }
            "sandbox_mode" if sandbox_mode.is_none() => {
                *sandbox_mode = Some(normalized_value);
            }
            _ => {}
        }
    }
}

#[derive(Default)]
struct AcpState {
    session_id: Option<String>,
    conversation_id: Option<ConversationId>,
    pending_prompt_ids: VecDeque<u64>,
    session_id_waiters: Vec<oneshot::Sender<String>>,
    app_server_initialized: bool,
    saw_message_delta: bool,
    saw_reasoning_delta: bool,
}

struct AcpWriter {
    stdout: Mutex<tokio::io::Stdout>,
}

impl AcpWriter {
    fn new() -> Self {
        Self {
            stdout: Mutex::new(tokio::io::stdout()),
        }
    }

    async fn send_json<T: Serialize + Sync>(&self, value: &T) -> Result<()> {
        let raw = serde_json::to_string(value)?;
        let mut guard = self.stdout.lock().await;
        guard.write_all(raw.as_bytes()).await?;
        guard.write_all(b"\n").await?;
        guard.flush().await?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct JsonRpcIncomingRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcNotification {
    jsonrpc: String,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Option<u64>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<JsonRpcErrorPayload>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcErrorPayload {
    code: i32,
    message: String,
    #[serde(default)]
    data: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AcpMessage {
    Request(JsonRpcIncomingRequest),
    Notification(JsonRpcNotification),
    Response(JsonRpcResponse),
}

#[derive(Serialize)]
struct JsonRpcResponseOut {
    jsonrpc: &'static str,
    id: u64,
    result: Value,
}

#[derive(Serialize)]
struct JsonRpcErrorOut {
    jsonrpc: &'static str,
    id: u64,
    error: JsonRpcErrorOutPayload,
}

#[derive(Serialize)]
struct JsonRpcErrorOutPayload {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitializeParamsInput {
    #[serde(alias = "protocol_version")]
    protocol_version: String,
    #[serde(default, alias = "client_capabilities")]
    client_capabilities: Value,
    #[serde(default, alias = "client_info")]
    client_info: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthenticateParamsInput {
    method_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewSessionParamsInput {
    cwd: String,
    #[serde(default)]
    mcp_servers: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoadSessionParamsInput {
    session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PromptParamsInput {
    session_id: String,
    prompt: Vec<ContentBlock>,
    #[serde(default)]
    context: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelParamsInput {
    session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlock {
    Text { text: String },
    Image { data: String, #[serde(rename = "mimeType", alias = "mime_type")] mime_type: String },
}

#[derive(Debug, Deserialize)]
struct CodexNotificationParams {
    #[serde(rename = "msg")]
    msg: EventMsg,
}

#[derive(Debug)]
enum AppServerEvent {
    SessionConfigured { session_id: String },
    CodexEvent(EventMsg),
}

struct AppServerClient {
    rpc: JsonRpcPeer,
    _child: Mutex<tokio::process::Child>,
}

impl AppServerClient {
    async fn spawn(config: &CliConfig) -> Result<(Self, mpsc::UnboundedReceiver<AppServerEvent>)> {
        let (events_tx, events_rx) = mpsc::unbounded_channel();

        let mut cmd = Command::new(CODEX_BASE_COMMAND[0]);
        cmd.args(&CODEX_BASE_COMMAND[1..]);
        cmd.args(&config.app_server_args);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .env("NODE_NO_WARNINGS", "1")
            .env("NO_COLOR", "1")
            .env("RUST_LOG", "error");

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow!("无法获取 app-server stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("无法获取 app-server stdout"))?;

        let callbacks = Arc::new(AppServerCallbacks { events_tx });
        let rpc = JsonRpcPeer::spawn(stdin, stdout, callbacks);
        Ok((Self { rpc, _child: Mutex::new(child) }, events_rx))
    }

    async fn initialize(&self) -> Result<()> {
        let request = ClientRequest::Initialize {
            request_id: self.rpc.next_request_id(),
            params: InitializeParams {
                client_info: ClientInfo {
                    name: "acp-codex".to_string(),
                    title: None,
                    version: env!("CARGO_PKG_VERSION").to_string(),
                },
            },
        };
        self.rpc
            .request::<InitializeResponse, _>(request_id(&request), &request, "initialize")
            .await?;
        let notification = ClientNotification::Initialized;
        self.rpc.send(&notification).await?;
        Ok(())
    }

    async fn new_conversation(
        &self,
        params: NewConversationParams,
    ) -> Result<NewConversationResponse> {
        let request = ClientRequest::NewConversation {
            request_id: self.rpc.next_request_id(),
            params,
        };
        self.rpc
            .request(request_id(&request), &request, "newConversation")
            .await
    }

    async fn resume_conversation(
        &self,
        rollout_path: PathBuf,
        overrides: NewConversationParams,
    ) -> Result<ResumeConversationResponse> {
        let request = ClientRequest::ResumeConversation {
            request_id: self.rpc.next_request_id(),
            params: ResumeConversationParams {
                path: Some(rollout_path),
                overrides: Some(overrides),
                conversation_id: None,
                history: None,
            },
        };
        self.rpc
            .request(request_id(&request), &request, "resumeConversation")
            .await
    }

    async fn add_conversation_listener(
        &self,
        conversation_id: ConversationId,
    ) -> Result<AddConversationSubscriptionResponse> {
        let request = ClientRequest::AddConversationListener {
            request_id: self.rpc.next_request_id(),
            params: AddConversationListenerParams {
                conversation_id,
                experimental_raw_events: false,
            },
        };
        self.rpc
            .request(request_id(&request), &request, "addConversationListener")
            .await
    }

    async fn send_user_message(
        &self,
        conversation_id: ConversationId,
        items: Vec<InputItem>,
    ) -> Result<SendUserMessageResponse> {
        let request = ClientRequest::SendUserMessage {
            request_id: self.rpc.next_request_id(),
            params: SendUserMessageParams {
                conversation_id,
                items,
            },
        };
        self.rpc
            .request(request_id(&request), &request, "sendUserMessage")
            .await
    }
}

#[async_trait]
trait JsonRpcCallbacks: Send + Sync {
    async fn on_request(
        &self,
        peer: &JsonRpcPeer,
        request: JSONRPCRequest,
    ) -> Result<()>;
    async fn on_response(
        &self,
        _peer: &JsonRpcPeer,
        _response: &JSONRPCResponse,
    ) -> Result<()>;
    async fn on_error(&self, _peer: &JsonRpcPeer, _error: &JSONRPCError) -> Result<()>;
    async fn on_notification(
        &self,
        _peer: &JsonRpcPeer,
        notification: JSONRPCNotification,
    ) -> Result<()>;
    async fn on_non_json(&self, _raw: &str) -> Result<()>;
}

#[derive(Clone)]
struct JsonRpcPeer {
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    pending: Arc<Mutex<HashMap<RequestId, oneshot::Sender<PendingResponse>>>>,
    id_counter: Arc<AtomicI64>,
}

impl JsonRpcPeer {
    fn spawn(
        stdin: tokio::process::ChildStdin,
        stdout: tokio::process::ChildStdout,
        callbacks: Arc<dyn JsonRpcCallbacks>,
    ) -> Self {
        let peer = Self {
            stdin: Arc::new(Mutex::new(stdin)),
            pending: Arc::new(Mutex::new(HashMap::new())),
            id_counter: Arc::new(AtomicI64::new(1)),
        };

        let reader_peer = peer.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut buffer = String::new();

            loop {
                buffer.clear();
                match reader.read_line(&mut buffer).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let line = buffer.trim_end_matches(['\n', '\r']);
                        if line.is_empty() {
                            continue;
                        }

                        match serde_json::from_str::<JSONRPCMessage>(line) {
                            Ok(JSONRPCMessage::Response(response)) => {
                                let request_id = response.id.clone();
                                if callbacks
                                    .on_response(&reader_peer, &response)
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                reader_peer
                                    .resolve(request_id, PendingResponse::Result(response.result))
                                    .await;
                            }
                            Ok(JSONRPCMessage::Error(error)) => {
                                let request_id = error.id.clone();
                                if callbacks
                                    .on_error(&reader_peer, &error)
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                reader_peer
                                    .resolve(request_id, PendingResponse::Error(error))
                                    .await;
                            }
                            Ok(JSONRPCMessage::Request(request)) => {
                                if callbacks
                                    .on_request(&reader_peer, request)
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Ok(JSONRPCMessage::Notification(notification)) => {
                                if callbacks
                                    .on_notification(&reader_peer, notification)
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(_) => {
                                if callbacks.on_non_json(line).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Err(_) => break,
                }
            }

            let _ = reader_peer.shutdown().await;
        });

        peer
    }

    fn next_request_id(&self) -> RequestId {
        RequestId::Integer(self.id_counter.fetch_add(1, Ordering::Relaxed))
    }

    async fn register(&self, request_id: RequestId) -> PendingReceiver {
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(request_id, sender);
        receiver
    }

    async fn resolve(&self, request_id: RequestId, response: PendingResponse) {
        if let Some(sender) = self.pending.lock().await.remove(&request_id) {
            let _ = sender.send(response);
        }
    }

    async fn shutdown(&self) -> Result<()> {
        let mut pending = self.pending.lock().await;
        for (_, sender) in pending.drain() {
            let _ = sender.send(PendingResponse::Shutdown);
        }
        Ok(())
    }

    async fn send<T>(&self, message: &T) -> Result<()>
    where
        T: Serialize + Sync,
    {
        let raw = serde_json::to_string(message)?;
        let mut guard = self.stdin.lock().await;
        guard.write_all(raw.as_bytes()).await?;
        guard.write_all(b"\n").await?;
        guard.flush().await?;
        Ok(())
    }

    async fn request<R, T>(&self, request_id: RequestId, message: &T, label: &str) -> Result<R>
    where
        R: serde::de::DeserializeOwned + std::fmt::Debug,
        T: Serialize + Sync,
    {
        let receiver = self.register(request_id).await;
        self.send(message).await?;
        await_response(receiver, label).await
    }
}

pub type PendingReceiver = oneshot::Receiver<PendingResponse>;

#[derive(Debug)]
pub enum PendingResponse {
    Result(Value),
    Error(JSONRPCError),
    Shutdown,
}

async fn await_response<R>(receiver: PendingReceiver, label: &str) -> Result<R>
where
    R: serde::de::DeserializeOwned + std::fmt::Debug,
{
    match receiver.await {
        Ok(PendingResponse::Result(value)) => {
            serde_json::from_value(value).map_err(|err| anyhow!("{label} 解码失败: {err}"))
        }
        Ok(PendingResponse::Error(error)) => Err(anyhow!(
            "{label} 失败: {}",
            error.error.message
        )),
        Ok(PendingResponse::Shutdown) => Err(anyhow!("{label} 被关闭")),
        Err(_) => Err(anyhow!("{label} 结果通道已关闭")),
    }
}

fn request_id(request: &ClientRequest) -> RequestId {
    match request {
        ClientRequest::Initialize { request_id, .. }
        | ClientRequest::NewConversation { request_id, .. }
        | ClientRequest::ResumeConversation { request_id, .. }
        | ClientRequest::AddConversationListener { request_id, .. }
        | ClientRequest::SendUserMessage { request_id, .. } => request_id.clone(),
        _ => unreachable!("unsupported request variant"),
    }
}

struct AppServerCallbacks {
    events_tx: mpsc::UnboundedSender<AppServerEvent>,
}

#[async_trait]
impl JsonRpcCallbacks for AppServerCallbacks {
    async fn on_request(&self, peer: &JsonRpcPeer, request: JSONRPCRequest) -> Result<()> {
        match ServerRequest::try_from(request.clone()) {
            Ok(ServerRequest::ExecCommandApproval { request_id, .. }) => {
                let response = ExecCommandApprovalResponse {
                    decision: ReviewDecision::ApprovedForSession,
                };
                let payload = JSONRPCResponse {
                    id: request_id,
                    result: serde_json::to_value(response)?,
                };
                peer.send(&payload).await?;
            }
            Ok(ServerRequest::ApplyPatchApproval { request_id, .. }) => {
                let response = ApplyPatchApprovalResponse {
                    decision: ReviewDecision::ApprovedForSession,
                };
                let payload = JSONRPCResponse {
                    id: request_id,
                    result: serde_json::to_value(response)?,
                };
                peer.send(&payload).await?;
            }
            Ok(_) => {
                let payload = JSONRPCResponse {
                    id: request.id,
                    result: Value::Null,
                };
                peer.send(&payload).await?;
            }
            Err(_) => {
                let payload = JSONRPCResponse {
                    id: request.id,
                    result: Value::Null,
                };
                peer.send(&payload).await?;
            }
        }
        Ok(())
    }

    async fn on_response(&self, _peer: &JsonRpcPeer, _response: &JSONRPCResponse) -> Result<()> {
        Ok(())
    }

    async fn on_error(&self, _peer: &JsonRpcPeer, _error: &JSONRPCError) -> Result<()> {
        Ok(())
    }

    async fn on_notification(
        &self,
        _peer: &JsonRpcPeer,
        notification: JSONRPCNotification,
    ) -> Result<()> {
        if notification.method == "sessionConfigured" {
            if let Some(session_id) = notification
                .params
                .as_ref()
                .and_then(|value| value.get("session_id").or_else(|| value.get("sessionId")))
                .and_then(|value| value.as_str())
            {
                let _ = self.events_tx.send(AppServerEvent::SessionConfigured {
                    session_id: session_id.to_string(),
                });
            }
            return Ok(());
        }

        let method = notification.method.as_str();
        if !method.starts_with("codex/event") {
            return Ok(());
        }

        let Some(params) = notification
            .params
            .and_then(|value| serde_json::from_value::<CodexNotificationParams>(value).ok())
        else {
            return Ok(());
        };

        let _ = self.events_tx.send(AppServerEvent::CodexEvent(params.msg));
        Ok(())
    }

    async fn on_non_json(&self, _raw: &str) -> Result<()> {
        Ok(())
    }
}

struct SessionHandler;

impl SessionHandler {
    fn sessions_root() -> Result<PathBuf> {
        let home_dir = dirs::home_dir().ok_or_else(|| anyhow!("无法定位 HOME 目录"))?;
        Ok(home_dir.join(".codex").join("sessions"))
    }

    fn find_rollout_file_path(session_id: &str) -> Result<PathBuf> {
        let sessions_dir = Self::sessions_root()?;
        Self::scan_directory(&sessions_dir, session_id)
    }

    fn scan_directory(dir: &Path, session_id: &str) -> Result<PathBuf> {
        if !dir.exists() {
            return Err(anyhow!("sessions 目录不存在: {}", dir.display()));
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                if let Ok(found) = Self::scan_directory(&path, session_id) {
                    return Ok(found);
                }
            } else if path.is_file() {
                let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if filename.starts_with("rollout-")
                    && filename.ends_with(".jsonl")
                    && filename.contains(session_id)
                {
                    return Ok(path);
                }
            }
        }

        Err(anyhow!("未找到 session: {session_id}"))
    }
}

fn normalize_cwd(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn normalize_base64_payload(data: &str) -> String {
    let trimmed = data.trim();
    let payload = match trimmed.split_once("base64,") {
        Some((_, rest)) => rest,
        None => trimmed,
    };
    payload.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn image_extension_for_mime(mime_type: &str) -> &'static str {
    let lowered = mime_type
        .split(';')
        .next()
        .unwrap_or(mime_type)
        .trim()
        .to_ascii_lowercase();
    if lowered.contains("png") {
        "png"
    } else if lowered.contains("jpeg") || lowered.contains("jpg") {
        "jpg"
    } else if lowered.contains("webp") {
        "webp"
    } else if lowered.contains("gif") {
        "gif"
    } else {
        "bin"
    }
}

fn write_temp_image(data: &str, mime_type: &str) -> Result<PathBuf> {
    let normalized = normalize_base64_payload(data);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(normalized.as_bytes())
        .map_err(|err| anyhow!("image base64 decode failed: {err}"))?;
    let ext = image_extension_for_mime(mime_type);
    let filename = format!("acp-codex-image-{}.{}", Uuid::new_v4(), ext);
    let path = std::env::temp_dir().join(filename);
    std::fs::write(&path, bytes)?;
    Ok(path)
}

fn build_new_conversation_params(config: &CliConfig, cwd: &Path) -> Result<NewConversationParams> {
    let sandbox = match config
        .sandbox_mode
        .as_deref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty() && *v != "auto")
    {
        None => Some(CodexSandboxMode::WorkspaceWrite),
        Some("read-only") => Some(CodexSandboxMode::ReadOnly),
        Some("workspace-write") => Some(CodexSandboxMode::WorkspaceWrite),
        Some("danger-full-access") => Some(CodexSandboxMode::DangerFullAccess),
        Some(other) => return Err(anyhow!("未知 sandbox_mode: {other}")),
    };

    let mut approval_policy = match config
        .approval_policy
        .as_deref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty() && *v != "auto")
    {
        None => None,
        Some("unless-trusted") => Some(CodexAskForApproval::UnlessTrusted),
        Some("on-failure") => Some(CodexAskForApproval::OnFailure),
        Some("on-request") => Some(CodexAskForApproval::OnRequest),
        Some("never") => Some(CodexAskForApproval::Never),
        Some(other) => return Err(anyhow!("未知 approval_policy: {other}")),
    };

    if approval_policy.is_none() {
        approval_policy = Some(CodexAskForApproval::OnRequest);
    }

    Ok(NewConversationParams {
        model: None,
        profile: None,
        cwd: Some(cwd.to_string_lossy().to_string()),
        approval_policy,
        sandbox,
        config: None,
        base_instructions: None,
        include_apply_patch_tool: Some(true),
        model_provider: None,
        compact_prompt: None,
        developer_instructions: None,
    })
}

async fn send_session_update(
    writer: &AcpWriter,
    session_id: &str,
    update: Value,
) -> Result<()> {
    let params = json!({
        "session_id": session_id,
        "sessionId": session_id,
        "update": update,
    });
    let message = json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": params,
    });
    writer.send_json(&message).await
}

fn update_with_type(update_type: &str) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    map.insert(
        "session_update".to_string(),
        Value::String(update_type.to_string()),
    );
    map.insert(
        "sessionUpdate".to_string(),
        Value::String(update_type.to_string()),
    );
    map
}

fn insert_dual(map: &mut serde_json::Map<String, Value>, snake: &str, camel: &str, value: Value) {
    map.insert(snake.to_string(), value.clone());
    map.insert(camel.to_string(), value);
}

async fn handle_codex_event(
    event: EventMsg,
    writer: &AcpWriter,
    state: &Arc<Mutex<AcpState>>,
) -> Result<()> {
    match event {
        EventMsg::SessionConfigured(payload) => {
            let session_id_value = payload.session_id.to_string();
            let mut guard = state.lock().await;
            guard.session_id = Some(session_id_value.clone());
            guard.saw_message_delta = false;
            guard.saw_reasoning_delta = false;
            for waiter in guard.session_id_waiters.drain(..) {
                let _ = waiter.send(session_id_value.clone());
            }
            return Ok(());
        }
        _ => {}
    }

    let session_id = {
        let guard = state.lock().await;
        guard.session_id.clone()
    };
    let Some(session_id) = session_id else {
        return Ok(());
    };

    match event {
        EventMsg::AgentMessageDelta(AgentMessageDeltaEvent { delta }) => {
            {
                let mut guard = state.lock().await;
                guard.saw_message_delta = true;
            }
            let mut update = update_with_type("agent_message_chunk");
            update.insert("content".to_string(), json!({ "text": delta }));
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::AgentReasoningDelta(AgentReasoningDeltaEvent { delta }) => {
            {
                let mut guard = state.lock().await;
                guard.saw_reasoning_delta = true;
            }
            let mut update = update_with_type("agent_thought_chunk");
            update.insert("content".to_string(), json!({ "text": delta }));
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::AgentMessage(AgentMessageEvent { message }) => {
            let should_send = {
                let mut guard = state.lock().await;
                let should_send = !guard.saw_message_delta;
                guard.saw_message_delta = true;
                should_send
            };
            if should_send {
                let mut update = update_with_type("agent_message_chunk");
                update.insert("content".to_string(), json!({ "text": message }));
                send_session_update(writer, &session_id, Value::Object(update)).await?;
            }
        }
        EventMsg::AgentReasoning(AgentReasoningEvent { text }) => {
            let should_send = {
                let mut guard = state.lock().await;
                let should_send = !guard.saw_reasoning_delta;
                guard.saw_reasoning_delta = true;
                should_send
            };
            if should_send {
                let mut update = update_with_type("agent_thought_chunk");
                update.insert("content".to_string(), json!({ "text": text }));
                send_session_update(writer, &session_id, Value::Object(update)).await?;
            }
        }
        EventMsg::PlanUpdate(UpdatePlanArgs { plan, explanation }) => {
            let entries: Vec<Value> = plan
                .into_iter()
                .map(|item| {
                    let status = match item.status {
                        StepStatus::Pending => "pending",
                        StepStatus::InProgress => "in_progress",
                        StepStatus::Completed => "completed",
                    };
                    json!({
                        "step": item.step,
                        "status": status,
                        "priority": "medium",
                    })
                })
                .collect();
            let mut update = update_with_type("plan");
            update.insert("entries".to_string(), Value::Array(entries));
            if let Some(explanation) = explanation {
                if !explanation.trim().is_empty() {
                    update.insert("explanation".to_string(), Value::String(explanation));
                }
            }
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::ExecCommandBegin(event) => {
            let command_text = event.command.join(" ");
            if !command_text.is_empty() {
                let mut update = update_with_type("tool_call");
                insert_dual(
                    &mut update,
                    "tool_call_id",
                    "toolCallId",
                    Value::String(event.call_id.to_string()),
                );
                insert_dual(
                    &mut update,
                    "name",
                    "name",
                    Value::String("bash".to_string()),
                );
                insert_dual(
                    &mut update,
                    "title",
                    "title",
                    Value::String(command_text.clone()),
                );
                insert_dual(
                    &mut update,
                    "status",
                    "status",
                    Value::String("in_progress".to_string()),
                );
                let raw_input = json!({
                    "command": command_text
                });
                insert_dual(&mut update, "raw_input", "rawInput", raw_input);
                send_session_update(writer, &session_id, Value::Object(update)).await?;
            }
        }
        EventMsg::ExecCommandEnd(event) => {
            let output = event.formatted_output;
            let mut update = update_with_type("tool_call_update");
            insert_dual(
                &mut update,
                "tool_call_id",
                "toolCallId",
                Value::String(event.call_id.to_string()),
            );
            let status = if event.exit_code == 0 {
                "completed"
            } else {
                "failed"
            };
            insert_dual(
                &mut update,
                "status",
                "status",
                Value::String(status.to_string()),
            );
            if !output.is_empty() {
                let content = vec![json!({
                    "type": "content",
                    "content": { "text": output }
                })];
                update.insert("content".to_string(), Value::Array(content));
            }
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::McpToolCallBegin(McpToolCallBeginEvent { call_id, invocation }) => {
            let mut update = update_with_type("tool_call");
            insert_dual(
                &mut update,
                "tool_call_id",
                "toolCallId",
                Value::String(call_id.to_string()),
            );
            let name = format!("mcp:{}:{}", invocation.server, invocation.tool);
            insert_dual(&mut update, "name", "name", Value::String(name));
            insert_dual(
                &mut update,
                "status",
                "status",
                Value::String("in_progress".to_string()),
            );
            insert_dual(
                &mut update,
                "raw_input",
                "rawInput",
                invocation.arguments.unwrap_or(Value::Null),
            );
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::McpToolCallEnd(McpToolCallEndEvent { call_id, result, .. }) => {
            let output = match result {
                Ok(value) => serde_json::to_string(&value).unwrap_or_default(),
                Err(err) => err,
            };
            let mut update = update_with_type("tool_call_update");
            insert_dual(
                &mut update,
                "tool_call_id",
                "toolCallId",
                Value::String(call_id.to_string()),
            );
            insert_dual(
                &mut update,
                "status",
                "status",
                Value::String("completed".to_string()),
            );
            if !output.is_empty() {
                let content = vec![json!({
                    "type": "content",
                    "content": { "text": output }
                })];
                update.insert("content".to_string(), Value::Array(content));
            }
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::PatchApplyBegin(PatchApplyBeginEvent { call_id, changes, .. }) => {
            let mut update = update_with_type("tool_call");
            insert_dual(
                &mut update,
                "tool_call_id",
                "toolCallId",
                Value::String(call_id.to_string()),
            );
            insert_dual(&mut update, "name", "name", Value::String("edit".to_string()));
            insert_dual(
                &mut update,
                "status",
                "status",
                Value::String("in_progress".to_string()),
            );
            insert_dual(
                &mut update,
                "raw_input",
                "rawInput",
                serde_json::to_value(changes).unwrap_or(Value::Null),
            );
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::PatchApplyEnd(PatchApplyEndEvent { call_id, success, .. }) => {
            let mut update = update_with_type("tool_call_update");
            insert_dual(
                &mut update,
                "tool_call_id",
                "toolCallId",
                Value::String(call_id.to_string()),
            );
            let status = if success { "completed" } else { "failed" };
            insert_dual(
                &mut update,
                "status",
                "status",
                Value::String(status.to_string()),
            );
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::WebSearchBegin(WebSearchBeginEvent { call_id }) => {
            let mut update = update_with_type("tool_call");
            insert_dual(
                &mut update,
                "tool_call_id",
                "toolCallId",
                Value::String(call_id.to_string()),
            );
            insert_dual(
                &mut update,
                "name",
                "name",
                Value::String("web_search".to_string()),
            );
            insert_dual(
                &mut update,
                "status",
                "status",
                Value::String("in_progress".to_string()),
            );
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::WebSearchEnd(WebSearchEndEvent { call_id, query }) => {
            let mut update = update_with_type("tool_call_update");
            insert_dual(
                &mut update,
                "tool_call_id",
                "toolCallId",
                Value::String(call_id.to_string()),
            );
            insert_dual(
                &mut update,
                "status",
                "status",
                Value::String("completed".to_string()),
            );
            let content = vec![json!({
                "type": "content",
                "content": { "text": query }
            })];
            update.insert("content".to_string(), Value::Array(content));
            send_session_update(writer, &session_id, Value::Object(update)).await?;
        }
        EventMsg::StreamError(StreamErrorEvent { message, .. }) => {
            let mut update = update_with_type("error");
            update.insert("error".to_string(), json!({ "message": message }));
            send_session_update(writer, &session_id, Value::Object(update)).await?;
            if let Some(prompt_id) = {
                let mut guard = state.lock().await;
                guard.pending_prompt_ids.pop_front()
            } {
                send_prompt_complete(writer, prompt_id, "error").await?;
            }
            {
                let mut guard = state.lock().await;
                guard.saw_message_delta = false;
                guard.saw_reasoning_delta = false;
            }
        }
        EventMsg::Error(ErrorEvent { message, .. }) => {
            let mut update = update_with_type("error");
            update.insert("error".to_string(), json!({ "message": message }));
            send_session_update(writer, &session_id, Value::Object(update)).await?;
            if let Some(prompt_id) = {
                let mut guard = state.lock().await;
                guard.pending_prompt_ids.pop_front()
            } {
                send_prompt_complete(writer, prompt_id, "error").await?;
            }
            {
                let mut guard = state.lock().await;
                guard.saw_message_delta = false;
                guard.saw_reasoning_delta = false;
            }
        }
        EventMsg::TaskComplete(_) => {
            let mut update = update_with_type("task_complete");
            update.insert(
                "stop_reason".to_string(),
                Value::String("end_turn".to_string()),
            );
            send_session_update(writer, &session_id, Value::Object(update)).await?;
            if let Some(prompt_id) = {
                let mut guard = state.lock().await;
                guard.pending_prompt_ids.pop_front()
            } {
                send_prompt_complete(writer, prompt_id, "end_turn").await?;
            }
            {
                let mut guard = state.lock().await;
                guard.saw_message_delta = false;
                guard.saw_reasoning_delta = false;
            }
        }
        _ => {}
    }

    Ok(())
}

async fn send_prompt_complete(writer: &AcpWriter, id: u64, stop_reason: &str) -> Result<()> {
    let response = JsonRpcResponseOut {
        jsonrpc: "2.0",
        id,
        result: json!({ "stopReason": stop_reason }),
    };
    writer.send_json(&response).await
}

async fn load_rollout_history(path: &Path) -> Result<Vec<Value>> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let mut entries = Vec::new();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let Some(entry_type) = value.get("type").and_then(|v| v.as_str()) else {
            continue;
        };
        if entry_type != "event_msg" {
            continue;
        }
        let Some(payload) = value.get("payload") else {
            continue;
        };
        let Some(kind) = payload.get("type").and_then(|v| v.as_str()) else {
            continue;
        };
        let message = payload.get("message").and_then(|v| v.as_str());
        if message.is_none() {
            continue;
        }
        let role = match kind {
            "user_message" => "user",
            "agent_message" => "assistant",
            _ => continue,
        };
        entries.push(json!({
            "role": role,
            "content": message.unwrap_or_default(),
        }));
    }

    Ok(entries)
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = CliConfig::parse()?;
    let writer = Arc::new(AcpWriter::new());
    let state = Arc::new(Mutex::new(AcpState::default()));
    let (app_server, mut app_events) = AppServerClient::spawn(&config).await?;
    let app_server = Arc::new(app_server);

    let writer_clone = writer.clone();
    let state_clone = state.clone();
    tokio::spawn(async move {
        while let Some(event) = app_events.recv().await {
            match event {
                AppServerEvent::SessionConfigured { session_id } => {
                    let mut guard = state_clone.lock().await;
                    guard.session_id = Some(session_id.clone());
                    guard.saw_message_delta = false;
                    guard.saw_reasoning_delta = false;
                    for waiter in guard.session_id_waiters.drain(..) {
                        let _ = waiter.send(session_id.clone());
                    }
                }
                AppServerEvent::CodexEvent(msg) => {
                    if let Err(err) = handle_codex_event(msg, &writer_clone, &state_clone).await {
                        eprintln!("[acp-codex] 处理 codex 事件失败: {err}");
                    }
                }
            }
        }
    });

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut buffer = String::new();

    loop {
        buffer.clear();
        let bytes = reader.read_line(&mut buffer).await?;
        if bytes == 0 {
            break;
        }
        let line = buffer.trim_end_matches(['\n', '\r']);
        if line.is_empty() {
            continue;
        }

        let message: AcpMessage = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(err) => {
                eprintln!("[acp-codex] 无法解析 ACP 消息: {err}");
                continue;
            }
        };

        if let AcpMessage::Request(request) = message {
            if let Err(err) = handle_acp_request(
                request,
                &writer,
                &state,
                &app_server,
                &config,
            )
            .await
            {
                eprintln!("[acp-codex] 处理 ACP 请求失败: {err}");
            }
        }
    }

    Ok(())
}

async fn handle_acp_request(
    request: JsonRpcIncomingRequest,
    writer: &AcpWriter,
    state: &Arc<Mutex<AcpState>>,
    app_server: &Arc<AppServerClient>,
    config: &CliConfig,
) -> Result<()> {
    let request_id = request.id;
    if let Err(err) = handle_acp_request_inner(request, writer, state, app_server, config).await {
        let response = JsonRpcErrorOut {
            jsonrpc: "2.0",
            id: request_id,
            error: JsonRpcErrorOutPayload {
                code: -32000,
                message: err.to_string(),
                data: None,
            },
        };
        writer.send_json(&response).await?;
    }
    Ok(())
}

async fn handle_acp_request_inner(
    request: JsonRpcIncomingRequest,
    writer: &AcpWriter,
    state: &Arc<Mutex<AcpState>>,
    app_server: &Arc<AppServerClient>,
    config: &CliConfig,
) -> Result<()> {
    match request.method.as_str() {
        "initialize" => {
            let _params: InitializeParamsInput = request
                .params
                .as_ref()
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()?
                .unwrap_or(InitializeParamsInput {
                    protocol_version: "1".to_string(),
                    client_capabilities: Value::Null,
                    client_info: Value::Null,
                });

            let mut guard = state.lock().await;
            if !guard.app_server_initialized {
                guard.app_server_initialized = true;
                drop(guard);
                app_server.initialize().await?;
            }

            let mut result = serde_json::Map::new();
            insert_dual(
                &mut result,
                "protocol_version",
                "protocolVersion",
                Value::String("1".to_string()),
            );
            let capabilities = json!({
                "promptCapabilities": {
                    "embeddedContext": true,
                    "image": true
                },
                "loadSession": true
            });
            insert_dual(
                &mut result,
                "agent_capabilities",
                "agentCapabilities",
                capabilities,
            );
            let info = json!({
                "name": "acp-codex",
                "version": env!("CARGO_PKG_VERSION"),
                "title": "Codex"
            });
            insert_dual(&mut result, "agent_info", "agentInfo", info);
            insert_dual(
                &mut result,
                "auth_methods",
                "authMethods",
                Value::Array(Vec::new()),
            );
            let response = JsonRpcResponseOut {
                jsonrpc: "2.0",
                id: request.id,
                result: Value::Object(result),
            };
            writer.send_json(&response).await?;
        }
        "authenticate" => {
            let _params: AuthenticateParamsInput = request
                .params
                .as_ref()
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()?
                .unwrap_or(AuthenticateParamsInput {
                    method_id: "".to_string(),
                });
            let response = JsonRpcResponseOut {
                jsonrpc: "2.0",
                id: request.id,
                result: Value::Null,
            };
            writer.send_json(&response).await?;
        }
        "session/new" => {
            let params: NewSessionParamsInput = request
                .params
                .as_ref()
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()?
                .ok_or_else(|| anyhow!("session/new 缺少参数"))?;
            let cwd = normalize_cwd(&params.cwd);
            {
                let mut guard = state.lock().await;
                guard.session_id = None;
                guard.pending_prompt_ids.clear();
                guard.conversation_id = None;
                guard.saw_message_delta = false;
                guard.saw_reasoning_delta = false;
            }
            let conversation_params = build_new_conversation_params(config, &cwd)?;
            let response = app_server.new_conversation(conversation_params).await?;
            let conversation_id = response.conversation_id;
            let conversation_id_fallback = conversation_id.to_string();
            app_server
                .add_conversation_listener(conversation_id.clone())
                .await?;

            let (waiter, session_id_ready) = {
                let mut guard = state.lock().await;
                guard.conversation_id = Some(conversation_id);
                if let Some(session_id) = guard.session_id.clone() {
                    (None, Some(session_id))
                } else {
                    let (tx, rx) = oneshot::channel();
                    guard.session_id_waiters.push(tx);
                    (Some(rx), None)
                }
            };

            let session_id = if let Some(session_id) = session_id_ready {
                session_id
            } else if let Some(waiter) = waiter {
                match tokio::time::timeout(std::time::Duration::from_secs(5), waiter).await {
                    Ok(Ok(value)) => value,
                    _ => conversation_id_fallback,
                }
            } else {
                conversation_id_fallback
            };

            {
                let mut guard = state.lock().await;
                guard.session_id = Some(session_id.clone());
            }

            let mut result = serde_json::Map::new();
            insert_dual(
                &mut result,
                "session_id",
                "sessionId",
                Value::String(session_id),
            );
            result.insert("modes".to_string(), Value::Array(Vec::new()));
            result.insert("models".to_string(), Value::Array(Vec::new()));
            insert_dual(
                &mut result,
                "config_options",
                "configOptions",
                Value::Array(Vec::new()),
            );
            let response = JsonRpcResponseOut {
                jsonrpc: "2.0",
                id: request.id,
                result: Value::Object(result),
            };
            writer.send_json(&response).await?;
        }
        "session/load" => {
            let params: LoadSessionParamsInput = request
                .params
                .as_ref()
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()?
                .ok_or_else(|| anyhow!("session/load 缺少参数"))?;

            {
                let mut guard = state.lock().await;
                guard.session_id = None;
                guard.pending_prompt_ids.clear();
                guard.conversation_id = None;
                guard.saw_message_delta = false;
                guard.saw_reasoning_delta = false;
            }

            let rollout_path = SessionHandler::find_rollout_file_path(&params.session_id)?;
            let cwd = normalize_cwd(".");
            let conversation_params = build_new_conversation_params(config, &cwd)?;

            let response = app_server
                .resume_conversation(rollout_path.clone(), conversation_params)
                .await?;
            let conversation_id = response.conversation_id;
            app_server
                .add_conversation_listener(conversation_id.clone())
                .await?;

            let history = load_rollout_history(&rollout_path).await.unwrap_or_default();

            {
                let mut guard = state.lock().await;
                guard.session_id = Some(params.session_id.clone());
                guard.conversation_id = Some(conversation_id);
            }

            let result = json!({
                "modes": [],
                "models": [],
                "history": history,
            });
            let response = JsonRpcResponseOut {
                jsonrpc: "2.0",
                id: request.id,
                result,
            };
            writer.send_json(&response).await?;
        }
        "session/prompt" => {
            let params: PromptParamsInput = request
                .params
                .as_ref()
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()?
                .ok_or_else(|| anyhow!("session/prompt 缺少参数"))?;

            let (conversation_id, session_id) = {
                let guard = state.lock().await;
                (guard.conversation_id.clone(), guard.session_id.clone())
            };
            let conversation_id = conversation_id.ok_or_else(|| anyhow!("尚未初始化会话"))?;
            let session_id = session_id.ok_or_else(|| anyhow!("尚未初始化会话"))?;

            if params.session_id != session_id {
                return Err(anyhow!("session_id 不匹配"));
            }

            {
                let mut guard = state.lock().await;
                guard.saw_message_delta = false;
                guard.saw_reasoning_delta = false;
            }

            let mut items = Vec::new();
            for block in params.prompt {
                match block {
                    ContentBlock::Text { text } => {
                        if !text.trim().is_empty() {
                            items.push(InputItem::Text { text });
                        }
                    }
                    ContentBlock::Image { data, mime_type } => {
                        match write_temp_image(&data, &mime_type) {
                            Ok(path) => {
                                items.push(InputItem::LocalImage { path });
                            }
                            Err(err) => {
                                eprintln!("[acp-codex] 无法处理 image block: {err}");
                            }
                        }
                    }
                }
            }
            if items.is_empty() {
                let response = JsonRpcResponseOut {
                    jsonrpc: "2.0",
                    id: request.id,
                    result: json!({ "stopReason": "empty" }),
                };
                writer.send_json(&response).await?;
                return Ok(());
            }

            app_server.send_user_message(conversation_id, items).await?;

            {
                let mut guard = state.lock().await;
                guard.pending_prompt_ids.push_back(request.id);
            }
        }
        "session/cancel" => {
            let _params: CancelParamsInput = request
                .params
                .as_ref()
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()?
                .unwrap_or(CancelParamsInput {
                    session_id: "".to_string(),
                });
            if let Some(prompt_id) = {
                let mut guard = state.lock().await;
                guard.pending_prompt_ids.pop_front()
            } {
                send_prompt_complete(writer, prompt_id, "cancelled").await?;
            }
            {
                let mut guard = state.lock().await;
                guard.saw_message_delta = false;
                guard.saw_reasoning_delta = false;
            }
            let response = JsonRpcResponseOut {
                jsonrpc: "2.0",
                id: request.id,
                result: Value::Null,
            };
            writer.send_json(&response).await?;
        }
        _ => {
            let response = JsonRpcErrorOut {
                jsonrpc: "2.0",
                id: request.id,
                error: JsonRpcErrorOutPayload {
                    code: -32601,
                    message: format!("未知方法: {}", request.method),
                    data: None,
                },
            };
            writer.send_json(&response).await?;
        }
    }

    Ok(())
}
