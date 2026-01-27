use serde::Serialize;

/// Stable error payload returned from Tauri commands.
///
/// Note: The frontend is responsible for mapping `code` -> i18n message.
#[derive(Debug, Serialize)]
pub struct AppErrorPayload {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
}

pub fn app_error(code: impl Into<String>, message: impl Into<String>) -> String {
    let payload = AppErrorPayload {
        code: code.into(),
        message: message.into(),
        details: None,
        retryable: None,
    };
    serde_json::to_string(&payload).unwrap_or_else(|_| "unknown error".to_string())
}
