use std::time::Duration;

use reqwest::Client;
use serde_json::{json, Value};

use crate::services::http_utils::join_base_path;
use crate::types::ai::{AiRiskModelResponse, AiRiskRequest, AiRiskResponse};

pub async fn ai_risk_assess(request: AiRiskRequest) -> Result<AiRiskResponse, String> {
    if request.api_key.trim().is_empty() {
        return Err("missing api key".to_string());
    }
    let url = join_base_path(&request.base_url, &request.chat_path)?;
    let timeout_ms = request.timeout_ms.unwrap_or(10000);
    let client = Client::new();
    let payload = json!({
        "model": request.model,
        "messages": [
            { "role": "user", "content": request.prompt }
        ],
        "temperature": 0.2,
    });

    let response = client
        .post(url)
        .bearer_auth(request.api_key)
        .timeout(Duration::from_millis(timeout_ms))
        .json(&payload)
        .send()
        .await
        .map_err(|err| err.to_string())?;

    let status = response.status();
    let body = response.text().await.map_err(|err| err.to_string())?;
    if !status.is_success() {
        return Err(format!("ai request failed status={} body={}", status, body));
    }

    let value: Value = serde_json::from_str(&body).map_err(|err| err.to_string())?;
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(|val| val.as_str())
        .or_else(|| value.pointer("/choices/0/text").and_then(|val| val.as_str()))
        .unwrap_or("")
        .trim();
    if content.is_empty() {
        return Err("ai response missing content".to_string());
    }
    parse_ai_risk_content(content)
}

fn parse_ai_risk_content(content: &str) -> Result<AiRiskResponse, String> {
    let payload = extract_json_block(content).unwrap_or(content);
    let parsed: AiRiskModelResponse =
        serde_json::from_str(payload).map_err(|err| err.to_string())?;
    let risk = normalize_ai_risk(&parsed.risk)
        .ok_or_else(|| "risk must be low|medium|high".to_string())?;
    Ok(AiRiskResponse {
        risk,
        reason: parsed.reason.unwrap_or_default(),
        key_points: parsed.key_points.unwrap_or_default(),
    })
}

fn extract_json_block(input: &str) -> Option<&str> {
    let start = input.find('{')?;
    let end = input.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(&input[start..=end])
}

fn normalize_ai_risk(value: &str) -> Option<String> {
    let normalized = value.trim().to_lowercase();
    match normalized.as_str() {
        "low" | "medium" | "high" => Some(normalized),
        _ => None,
    }
}
