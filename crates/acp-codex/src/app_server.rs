use std::{
    collections::HashMap,
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc,
    },
};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use codex_app_server_protocol::{
    AddConversationListenerParams, AddConversationSubscriptionResponse, ApplyPatchApprovalResponse,
    ClientInfo, ClientNotification, ClientRequest, ExecCommandApprovalResponse, InitializeParams,
    InitializeResponse, InputItem, InterruptConversationParams, InterruptConversationResponse,
    JSONRPCError, JSONRPCMessage, JSONRPCNotification, JSONRPCRequest, JSONRPCResponse,
    NewConversationParams, NewConversationResponse, RemoveConversationListenerParams,
    RemoveConversationSubscriptionResponse, RequestId, ResumeConversationParams,
    ResumeConversationResponse, SendUserMessageParams, SendUserMessageResponse, ServerRequest,
};
use codex_protocol::{protocol::EventMsg, protocol::ReviewDecision, ConversationId};
use serde::Deserialize;
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::{mpsc, oneshot, Mutex},
};

use crate::cli::CliConfig;

const CODEX_BASE_COMMAND: &[&str] = &["npx", "-y", "@openai/codex@0.77.0", "app-server"];

#[derive(Debug)]
pub(crate) enum AppServerEvent {
    SessionConfigured {
        session_id: String,
    },
    CodexEvent {
        conversation_id: ConversationId,
        msg: EventMsg,
    },
    StderrLine(String),
}

pub(crate) struct AppServerClient {
    rpc: JsonRpcPeer,
    _child: Mutex<tokio::process::Child>,
}

impl AppServerClient {
    pub(crate) async fn spawn(
        config: &CliConfig,
    ) -> Result<(Self, mpsc::UnboundedReceiver<AppServerEvent>)> {
        let (events_tx, events_rx) = mpsc::unbounded_channel();

        let mut cmd = Command::new(CODEX_BASE_COMMAND[0]);
        cmd.args(&CODEX_BASE_COMMAND[1..]);
        cmd.args(&config.app_server_args);
        if let Some(home) = dirs::home_dir() {
            cmd.current_dir(&home).env("PWD", &home);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env("NODE_NO_WARNINGS", "1")
            .env("NO_COLOR", "1")
            .env("RUST_LOG", "error");

        let mut child = cmd.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("无法获取 app-server stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("无法获取 app-server stdout"))?;
        let stderr = child.stderr.take();

        if let Some(stderr) = stderr {
            let mut reader = BufReader::new(stderr);
            let mut buffer = String::new();
            let stderr_tx = events_tx.clone();
            tokio::spawn(async move {
                loop {
                    buffer.clear();
                    match reader.read_line(&mut buffer).await {
                        Ok(0) => break,
                        Ok(_) => {
                            let line = buffer.trim_end_matches(['\n', '\r']);
                            if line.is_empty() {
                                continue;
                            }
                            eprintln!("{}", line);
                            let _ = stderr_tx.send(AppServerEvent::StderrLine(line.to_string()));
                        }
                        Err(err) => {
                            eprintln!("[acp-codex] app-server stderr read failed: {err}");
                            break;
                        }
                    }
                }
            });
        }

        let callbacks = Arc::new(AppServerCallbacks { events_tx });
        let rpc = JsonRpcPeer::spawn(stdin, stdout, callbacks);
        Ok((
            Self {
                rpc,
                _child: Mutex::new(child),
            },
            events_rx,
        ))
    }

    pub(crate) async fn initialize(&self) -> Result<()> {
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

    pub(crate) async fn new_conversation(
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

    pub(crate) async fn resume_conversation(
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

    pub(crate) async fn add_conversation_listener(
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

    pub(crate) async fn send_user_message(
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

    pub(crate) async fn interrupt_conversation(
        &self,
        conversation_id: ConversationId,
    ) -> Result<InterruptConversationResponse> {
        let request = ClientRequest::InterruptConversation {
            request_id: self.rpc.next_request_id(),
            params: InterruptConversationParams { conversation_id },
        };
        self.rpc
            .request(request_id(&request), &request, "interruptConversation")
            .await
    }

    pub(crate) async fn interrupt_conversation_no_wait(
        &self,
        conversation_id: ConversationId,
    ) -> Result<()> {
        // `interruptConversation` returns only after `TurnAborted`; if the conversation
        // has no active turn, the server may never respond. For our purposes we only
        // need best-effort delivery.
        let request = ClientRequest::InterruptConversation {
            request_id: self.rpc.next_request_id(),
            params: InterruptConversationParams { conversation_id },
        };
        self.rpc.send(&request).await
    }

    pub(crate) async fn remove_conversation_listener(
        &self,
        subscription_id: uuid::Uuid,
    ) -> Result<RemoveConversationSubscriptionResponse> {
        let request = ClientRequest::RemoveConversationListener {
            request_id: self.rpc.next_request_id(),
            params: RemoveConversationListenerParams { subscription_id },
        };
        self.rpc
            .request(request_id(&request), &request, "removeConversationListener")
            .await
    }
}

#[async_trait]
trait JsonRpcCallbacks: Send + Sync {
    async fn on_request(&self, peer: &JsonRpcPeer, request: JSONRPCRequest) -> Result<()>;
    async fn on_response(&self, _peer: &JsonRpcPeer, _response: &JSONRPCResponse) -> Result<()>;
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
        let peer_clone = peer.clone();
        tokio::spawn(async move {
            if let Err(err) = peer_clone.read_loop(stdout, callbacks).await {
                eprintln!("[acp-codex] app-server read loop failed: {err}");
            }
        });
        peer
    }

    fn next_request_id(&self) -> RequestId {
        RequestId::Integer(self.id_counter.fetch_add(1, Ordering::SeqCst))
    }

    async fn send<T: serde::Serialize>(&self, message: &T) -> Result<()> {
        let raw = serde_json::to_string(message)?;
        let mut guard = self.stdin.lock().await;
        guard.write_all(raw.as_bytes()).await?;
        guard.write_all(b"\n").await?;
        guard.flush().await?;
        Ok(())
    }

    async fn request<T, M>(&self, id: RequestId, message: &M, method: &str) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
        M: serde::Serialize,
    {
        let raw = serde_json::to_string(message)?;
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id.clone(), tx);
        }
        {
            let mut guard = self.stdin.lock().await;
            guard.write_all(raw.as_bytes()).await?;
            guard.write_all(b"\n").await?;
            guard.flush().await?;
        }
        let response = rx
            .await
            .map_err(|_| anyhow!("app-server 请求失败: {}", method))?;
        match response {
            PendingResponse::Ok(value) => Ok(serde_json::from_value(value)?),
            PendingResponse::Err(err) => Err(anyhow!("app-server 错误: {}", err)),
        }
    }

    async fn read_loop(
        &self,
        stdout: tokio::process::ChildStdout,
        callbacks: Arc<dyn JsonRpcCallbacks>,
    ) -> Result<()> {
        let mut reader = BufReader::new(stdout);
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
            let message: JSONRPCMessage = match serde_json::from_str(line) {
                Ok(message) => message,
                Err(_) => {
                    callbacks.on_non_json(line).await?;
                    continue;
                }
            };
            match message {
                JSONRPCMessage::Request(request) => {
                    callbacks.on_request(self, request).await?;
                }
                JSONRPCMessage::Response(response) => {
                    callbacks.on_response(self, &response).await?;
                    let id = response.id.clone();
                    let result = response.result;
                    let mut pending = self.pending.lock().await;
                    if let Some(sender) = pending.remove(&id) {
                        let _ = sender.send(PendingResponse::Ok(result));
                    }
                }
                JSONRPCMessage::Error(error) => {
                    callbacks.on_error(self, &error).await?;
                    let id = error.id.clone();
                    let message = error.error.message.clone();
                    let mut pending = self.pending.lock().await;
                    if let Some(sender) = pending.remove(&id) {
                        let _ = sender.send(PendingResponse::Err(message));
                    }
                }
                JSONRPCMessage::Notification(notification) => {
                    callbacks.on_notification(self, notification).await?;
                }
            }
        }
        Ok(())
    }
}

enum PendingResponse {
    Ok(Value),
    Err(String),
}

fn request_id(request: &ClientRequest) -> RequestId {
    let value = serde_json::to_value(request).expect("serialize client request");
    let id_value = value.get("id").cloned().expect("client request missing id");
    serde_json::from_value(id_value).expect("invalid request id")
}

#[derive(Debug, Deserialize)]
struct CodexNotificationParams {
    #[serde(rename = "conversationId")]
    conversation_id: ConversationId,
    #[serde(rename = "msg")]
    msg: EventMsg,
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

        let _ = self.events_tx.send(AppServerEvent::CodexEvent {
            conversation_id: params.conversation_id,
            msg: params.msg,
        });
        Ok(())
    }

    async fn on_non_json(&self, _raw: &str) -> Result<()> {
        Ok(())
    }
}
