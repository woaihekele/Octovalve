use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub(crate) struct JsonRpcIncomingRequest {
    pub(crate) jsonrpc: String,
    pub(crate) id: u64,
    pub(crate) method: String,
    #[serde(default)]
    pub(crate) params: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JsonRpcNotification {
    pub(crate) jsonrpc: String,
    pub(crate) method: String,
    #[serde(default)]
    pub(crate) params: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JsonRpcResponse {
    pub(crate) jsonrpc: String,
    pub(crate) id: Option<u64>,
    #[serde(default)]
    pub(crate) result: Option<Value>,
    #[serde(default)]
    pub(crate) error: Option<JsonRpcErrorPayload>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JsonRpcErrorPayload {
    pub(crate) code: i32,
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) data: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum AcpMessage {
    Request(JsonRpcIncomingRequest),
    Notification(JsonRpcNotification),
    Response(JsonRpcResponse),
}

#[derive(Serialize)]
pub(crate) struct JsonRpcResponseOut {
    pub(crate) jsonrpc: &'static str,
    pub(crate) id: u64,
    pub(crate) result: Value,
}

#[derive(Serialize)]
pub(crate) struct JsonRpcErrorOut {
    pub(crate) jsonrpc: &'static str,
    pub(crate) id: u64,
    pub(crate) error: JsonRpcErrorOutPayload,
}

#[derive(Serialize)]
pub(crate) struct JsonRpcErrorOutPayload {
    pub(crate) code: i32,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) data: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InitializeParamsInput {
    #[serde(alias = "protocol_version")]
    pub(crate) protocol_version: String,
    #[serde(default, alias = "client_capabilities")]
    pub(crate) client_capabilities: Value,
    #[serde(default, alias = "client_info")]
    pub(crate) client_info: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuthenticateParamsInput {
    pub(crate) method_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NewSessionParamsInput {
    pub(crate) cwd: String,
    #[serde(default)]
    pub(crate) mcp_servers: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoadSessionParamsInput {
    pub(crate) session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptParamsInput {
    pub(crate) session_id: String,
    pub(crate) prompt: Vec<ContentBlock>,
    #[serde(default)]
    pub(crate) context: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CancelParamsInput {
    pub(crate) session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ContentBlock {
    Text { text: String },
    Image { data: String, #[serde(rename = "mimeType", alias = "mime_type")] mime_type: String },
}
