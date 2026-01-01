//! ACP (Agent Client Protocol) type definitions for communication with codex-acp.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 request
#[derive(Debug, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: method.into(),
            params,
        }
    }
}

/// JSON-RPC 2.0 response
#[derive(Debug, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<u64>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error
#[derive(Debug, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<Value>,
}

/// JSON-RPC 2.0 notification (no id)
#[derive(Debug, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

/// Incoming message from codex-acp (either response or notification)
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum AcpMessage {
    Response(JsonRpcResponse),
    Notification(JsonRpcNotification),
}

// ============================================================================
// Initialize
// ============================================================================

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub protocol_version: String,
    pub client_capabilities: ClientCapabilities,
    pub client_info: ClientInfo,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<PromptCapabilities>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedded_context: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    #[serde(default)]
    pub agent_capabilities: Option<AgentCapabilities>,
    #[serde(default)]
    pub agent_info: Option<AgentInfo>,
    #[serde(default)]
    pub auth_methods: Vec<AuthMethod>,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct AgentCapabilities {
    #[serde(default)]
    pub prompt: Option<Value>,
    #[serde(default)]
    pub mcp: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AgentInfo {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AuthMethod {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

// ============================================================================
// Authenticate
// ============================================================================

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticateParams {
    pub method_id: String,
}

// ============================================================================
// Session
// ============================================================================

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSessionParams {
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<Value>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NewSessionResult {
    pub session_id: String,
    #[serde(default)]
    pub modes: Vec<SessionMode>,
    #[serde(default)]
    pub models: Vec<SessionModel>,
    #[serde(default)]
    pub config_options: Vec<Value>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SessionMode {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SessionModel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

// ============================================================================
// Prompt
// ============================================================================

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptParams {
    pub session_id: String,
    pub content: PromptContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Vec<ContextItem>>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum PromptContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    Image { data: String, media_type: String },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContextItem {
    #[serde(rename = "type")]
    pub item_type: String,
    #[serde(flatten)]
    pub data: Value,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PromptResult {
    pub stop_reason: String,
}

// ============================================================================
// Cancel
// ============================================================================

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelParams {
    pub session_id: String,
}

// ============================================================================
// Session Update Notifications
// ============================================================================

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionUpdate {
    ContentDelta {
        session_id: String,
        content: String,
    },
    ToolCallStart {
        session_id: String,
        tool_call_id: String,
        name: String,
        #[serde(default)]
        arguments: Option<Value>,
    },
    ToolCallEnd {
        session_id: String,
        tool_call_id: String,
        #[serde(default)]
        result: Option<String>,
        #[serde(default)]
        error: Option<String>,
    },
    PermissionRequest {
        session_id: String,
        request_id: String,
        #[serde(flatten)]
        request: PermissionRequestData,
    },
    Error {
        session_id: String,
        message: String,
    },
    Complete {
        session_id: String,
        stop_reason: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PermissionRequestData {
    Command {
        command: String,
        #[serde(default)]
        cwd: Option<String>,
    },
    FileWrite {
        path: String,
        #[serde(default)]
        diff: Option<String>,
    },
    #[serde(other)]
    Other,
}

// ============================================================================
// Frontend-facing types (for Tauri commands)
// ============================================================================

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AcpInitResponse {
    pub agent_info: Option<AgentInfo>,
    pub auth_methods: Vec<AuthMethod>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AcpSessionInfo {
    pub session_id: String,
    pub modes: Vec<SessionMode>,
    pub models: Vec<SessionModel>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AcpEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub payload: Value,
}
