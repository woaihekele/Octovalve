use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct AiRiskRequest {
    pub base_url: String,
    pub chat_path: String,
    pub model: String,
    pub api_key: String,
    pub prompt: String,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct AiRiskModelResponse {
    pub risk: String,
    pub reason: Option<String>,
    #[serde(default)]
    pub key_points: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct AiRiskResponse {
    pub risk: String,
    pub reason: String,
    pub key_points: Vec<String>,
}
