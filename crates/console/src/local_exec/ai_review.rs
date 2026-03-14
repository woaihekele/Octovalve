use anyhow::Context;
use protocol::CommandRequest;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

use super::policy::AiReadonlyReviewConfig;

#[derive(Clone)]
pub(crate) struct AiReadonlyReviewer {
    client: Client,
    endpoint: String,
    model: String,
    api_key: String,
    min_confidence: f64,
    max_command_chars: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct AiReadonlyDecision {
    pub(crate) read_only: bool,
    pub(crate) confidence: f64,
    pub(crate) reason: String,
    pub(crate) risk_flags: Vec<String>,
}

impl AiReadonlyReviewer {
    pub(crate) fn from_config(config: &AiReadonlyReviewConfig) -> anyhow::Result<Option<Self>> {
        if !config.enabled {
            return Ok(None);
        }

        let endpoint = config.endpoint.trim().to_string();
        if endpoint.is_empty() {
            anyhow::bail!("ai_readonly_review.endpoint cannot be empty when enabled");
        }

        let model = config.model.trim().to_string();
        if model.is_empty() {
            anyhow::bail!("ai_readonly_review.model cannot be empty when enabled");
        }

        let api_key_env = config.api_key_env.trim();
        if api_key_env.is_empty() {
            anyhow::bail!("ai_readonly_review.api_key_env cannot be empty when enabled");
        }
        let api_key = std::env::var(api_key_env)
            .with_context(|| format!("missing api key env var {api_key_env}"))?;
        let api_key = api_key.trim().to_string();
        if api_key.is_empty() {
            anyhow::bail!("api key env var {api_key_env} is empty");
        }

        let timeout_ms = config.timeout_ms.max(200);
        let client = Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()
            .context("build ai readonly review client")?;

        let min_confidence = config.min_confidence.clamp(0.0, 1.0);
        let max_command_chars = config.max_command_chars.max(256);

        Ok(Some(Self {
            client,
            endpoint,
            model,
            api_key,
            min_confidence,
            max_command_chars,
        }))
    }

    pub(crate) fn min_confidence(&self) -> f64 {
        self.min_confidence
    }

    pub(crate) fn model(&self) -> &str {
        &self.model
    }

    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(crate) async fn review(
        &self,
        request: &CommandRequest,
    ) -> anyhow::Result<AiReadonlyDecision> {
        let payload = json!({
            "model": self.model,
            "temperature": 0,
            "response_format": { "type": "json_object" },
            "messages": [
                {
                    "role": "system",
                    "content": "You are a command safety reviewer. Determine whether executing the command is strictly read-only. Read-only means observation only: listing, searching, inspecting. Any write/mutate/delete/network-changing/system-changing/credential-changing behavior is NOT read-only. If uncertain, mark read_only=false. Return JSON with keys: read_only(boolean), confidence(number 0..1), reason(string), risk_flags(array of short strings)."
                },
                {
                    "role": "user",
                    "content": build_user_prompt(request, self.max_command_chars)
                }
            ]
        });

        let response = self
            .client
            .post(&self.endpoint)
            .header(CONTENT_TYPE, "application/json")
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            .json(&payload)
            .send()
            .await
            .context("ai readonly review request failed")?;

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!(
                "ai readonly review http {}: {}",
                status,
                clip_text(&body, 300)
            );
        }

        let parsed: ChatCompletionResponse =
            serde_json::from_str(&body).context("parse ai readonly review response")?;
        let content = parsed
            .choices
            .first()
            .and_then(|choice| extract_content(choice.message.content.as_ref()))
            .ok_or_else(|| anyhow::anyhow!("ai readonly review missing message content"))?;
        let output = parse_model_output(&content)?;

        let read_only = output.read_only.unwrap_or(false);
        let confidence = output.confidence.unwrap_or(0.0).clamp(0.0, 1.0);
        let reason = output
            .reason
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "no reason provided".to_string());
        let risk_flags = output
            .risk_flags
            .unwrap_or_default()
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .take(8)
            .collect();

        Ok(AiReadonlyDecision {
            read_only,
            confidence,
            reason,
            risk_flags,
        })
    }
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    content: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ModelOutput {
    read_only: Option<bool>,
    confidence: Option<f64>,
    reason: Option<String>,
    risk_flags: Option<Vec<String>>,
}

fn build_user_prompt(request: &CommandRequest, max_command_chars: usize) -> String {
    let env_keys = request
        .env
        .as_ref()
        .map(|pairs| pairs.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let pipeline = request
        .pipeline
        .iter()
        .map(|stage| stage.argv.join(" "))
        .collect::<Vec<_>>();
    let command = clip_text(request.raw_command.trim(), max_command_chars);
    format!(
        "mode: {:?}\nintent: {}\nraw_command: {}\npipeline: {:?}\ncwd: {:?}\nenv_keys: {:?}",
        request.mode, request.intent, command, pipeline, request.cwd, env_keys
    )
}

fn extract_content(content: Option<&Value>) -> Option<String> {
    let content = content?;
    match content {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Value::Array(items) => {
            let mut merged = String::new();
            for item in items {
                if let Some(text) = item
                    .get("text")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    if !merged.is_empty() {
                        merged.push('\n');
                    }
                    merged.push_str(text);
                }
            }
            if merged.is_empty() {
                None
            } else {
                Some(merged)
            }
        }
        _ => None,
    }
}

fn parse_model_output(content: &str) -> anyhow::Result<ModelOutput> {
    if let Ok(output) = serde_json::from_str::<ModelOutput>(content) {
        return Ok(output);
    }

    let start = content
        .find('{')
        .ok_or_else(|| anyhow::anyhow!("ai readonly review content is not json"))?;
    let end = content
        .rfind('}')
        .ok_or_else(|| anyhow::anyhow!("ai readonly review content is not json"))?;
    if start >= end {
        anyhow::bail!("ai readonly review content is not json");
    }
    let candidate = &content[start..=end];
    let output = serde_json::from_str::<ModelOutput>(candidate)
        .context("parse ai readonly review json content")?;
    Ok(output)
}

fn clip_text(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    let mut clipped = String::new();
    for (idx, ch) in value.chars().enumerate() {
        if idx >= max_chars {
            break;
        }
        clipped.push(ch);
    }
    clipped.push_str(" ...[truncated]");
    clipped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_model_output_accepts_plain_json() {
        let output = parse_model_output(
            r#"{"read_only":true,"confidence":0.93,"reason":"list only","risk_flags":[]}"#,
        )
        .expect("parse");
        assert_eq!(output.read_only, Some(true));
        assert_eq!(output.confidence, Some(0.93));
    }

    #[test]
    fn parse_model_output_extracts_json_from_wrapped_text() {
        let output = parse_model_output(
            "result:\n```json\n{\"read_only\":false,\"confidence\":0.2,\"reason\":\"uses rm\"}\n```",
        )
        .expect("parse");
        assert_eq!(output.read_only, Some(false));
        assert_eq!(output.confidence, Some(0.2));
    }

    #[test]
    fn clip_text_truncates_long_text() {
        let clipped = clip_text("abcdef", 3);
        assert!(clipped.starts_with("abc"));
        assert!(clipped.contains("[truncated]"));
    }
}
