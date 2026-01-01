use futures_util::StreamExt;
use reqwest::Client;
use serde_json::json;

#[tokio::main]
async fn main() {
    let config = OpenAiConfig {
        base_url: "https://api.openai.com/v1".to_string(),
        api_key: "".to_string(),
        model: "gpt-4o-mini".to_string(),
        chat_path: "/chat/completions".to_string(),
    };

    let client = Client::new();
    let url = format!("{}{}", config.base_url, config.chat_path);

    let body = json!({
        "model": config.model,
        "messages": [
            {"role": "user", "content": "say hi"}
        ],
        "stream": true
    });

    println!("URL: {}", url);
    println!("Body: {}", serde_json::to_string_pretty(&body).unwrap());

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .expect("Request failed");

    println!("Status: {}", response.status());

    if !response.status().is_success() {
        let text = response.text().await.unwrap_or_default();
        println!("Error: {}", text);
        return;
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut full_content = String::new();

    println!("\n=== Stream ===");
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("Stream error");
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buffer.find('\n') {
            let line = buffer[..pos].trim().to_string();
            buffer = buffer[pos + 1..].to_string();

            if line.is_empty() || line.starts_with(':') {
                continue;
            }

            if let Some(data) = line.strip_prefix("data: ") {
                if data == "[DONE]" {
                    println!("\n[DONE]");
                    continue;
                }

                if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(choices) = json.get("choices").and_then(|c| c.as_array()) {
                        if let Some(choice) = choices.first() {
                            if let Some(delta) = choice.get("delta") {
                                if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                                    full_content.push_str(content);
                                    print!("{}", content);
                                    use std::io::Write;
                                    std::io::stdout().flush().unwrap();
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    println!("\n\n=== Full Response ===\n{}", full_content);
}

struct OpenAiConfig {
    base_url: String,
    api_key: String,
    model: String,
    chat_path: String,
}
