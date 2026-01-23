use serde_json::{json, Value};
use std::path::PathBuf;

use octovalve_backend::clients::mcp_client::{McpClient, McpServerSpec};
use octovalve_backend::paths::resolve_octovalve_proxy_bin;
use serde::Serialize;
use std::collections::HashMap;
use std::collections::HashSet;
use octovalve_backend::services::http_utils::join_base_path;
use octovalve_backend::services::mcp_config::parse_mcp_config_json;

const DEFAULT_COMMAND_ADDR: &str = "127.0.0.1:19310";

#[derive(Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiToolFunction,
}

#[derive(Serialize)]
struct OpenAiToolFunction {
    name: String,
    description: String,
    parameters: Value,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
struct OpenAiConfig {
    base_url: String,
    api_key: String,
    model: String,
    chat_path: String,
}

fn env_or_default(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

#[derive(Default)]
struct McpTimeouts {
    initialize_timeout_ms: Option<u64>,
    tools_call_timeout_ms: Option<u64>,
    attempt_timeout_ms: Option<u64>,
}

struct TestConfig {
    proxy_config: PathBuf,
    target: Option<String>,
    timeouts: McpTimeouts,
    mcp_config_json: Option<String>,
    question: String,
}

fn parse_args() -> TestConfig {
    let mut args = std::env::args().skip(1);
    let mut proxy_config: Option<PathBuf> = None;
    let mut target: Option<String> = None;
    let mut timeouts = McpTimeouts::default();
    let mut mcp_json: Option<String> = None;
    let mut mcp_json_path: Option<PathBuf> = None;
    let mut question: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--proxy-config" => {
                proxy_config = args.next().map(PathBuf::from);
            }
            "--target" => {
                target = args.next();
            }
            "--mcp-initialize-timeout-ms" => {
                timeouts.initialize_timeout_ms = args.next().and_then(|v| v.parse::<u64>().ok());
            }
            "--mcp-tools-call-timeout-ms" => {
                timeouts.tools_call_timeout_ms = args.next().and_then(|v| v.parse::<u64>().ok());
            }
            "--mcp-attempt-timeout-ms" => {
                timeouts.attempt_timeout_ms = args.next().and_then(|v| v.parse::<u64>().ok());
            }
            "--mcp-json" => {
                mcp_json = args.next();
            }
            "--mcp-json-path" => {
                mcp_json_path = args.next().map(PathBuf::from);
            }
            "--question" => {
                question = args.next();
            }
            _ => {}
        }
    }

    let proxy_config = proxy_config
        .or_else(|| {
            std::env::var("OCTOVALVE_PROXY_CONFIG")
                .ok()
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| PathBuf::from("config/local-proxy-config.toml"));
    let mcp_config_json = mcp_json
        .or_else(|| {
            mcp_json_path.and_then(|path| std::fs::read_to_string(path).ok())
        })
        .or_else(|| std::env::var("MCP_CONFIG_JSON").ok());
    let question = question.unwrap_or_else(|| "当前有哪些工具可以使用？".to_string());
    TestConfig {
        proxy_config,
        target,
        timeouts,
        mcp_config_json,
        question,
    }
}

#[tokio::main]
async fn main() -> Result<(), String> {
    // This config mirrors what console-ui would hold, but we don't actually call OpenAI here.
    // It exists so you can supply the same env vars you use in the app.
    let openai_config = OpenAiConfig {
        base_url: env_or_default("OPENAI_BASE_URL", ""),
        api_key: env_or_default("OPENAI_API_KEY", ""),
        model: env_or_default("OPENAI_MODEL", ""),
        chat_path: env_or_default("OPENAI_CHAT_PATH", "/v1/chat/completions"),
    };

    let test_config = parse_args();
    eprintln!("proxy_config={}", test_config.proxy_config.display());

    if let Some(value) = test_config.timeouts.initialize_timeout_ms {
        std::env::set_var("OCTOVALVE_MCP_INITIALIZE_TIMEOUT_MS", value.to_string());
    }
    if let Some(value) = test_config.timeouts.tools_call_timeout_ms {
        std::env::set_var("OCTOVALVE_MCP_TOOLS_CALL_TIMEOUT_MS", value.to_string());
    }
    if let Some(value) = test_config.timeouts.attempt_timeout_ms {
        std::env::set_var("OCTOVALVE_MCP_ATTEMPT_TIMEOUT_MS", value.to_string());
    }

    let proxy_bin = resolve_octovalve_proxy_bin()?;
    let spec = McpServerSpec {
        name: "octovalve".to_string(),
        command: proxy_bin,
        args: vec![
            "--config".to_string(),
            test_config.proxy_config.to_string_lossy().to_string(),
            "--command-addr".to_string(),
            DEFAULT_COMMAND_ADDR.to_string(),
        ],
        env: HashMap::new(),
        cwd: None,
    };
    let client = McpClient::start_with_spec(&spec).await?;

    // 1) Validate MCP path: list_tools
    let tools = client.list_tools().await?;
    println!(
        "list_tools result:\n{}",
        serde_json::to_string_pretty(&tools).unwrap_or_default()
    );

    // 2) Validate MCP path: list_targets
    let result = client.call_tool("list_targets", json!({})).await?;
    println!(
        "list_targets result:\n{}",
        serde_json::to_string_pretty(&result).unwrap_or_default()
    );

    // 3) Optional: validate run_command end-to-end (requires target)
    if let Some(target) = test_config.target {
        let args: Value = json!({
            "command": "echo octovalve-mcp-ok",
            "intent": "mcp bridge smoke test",
            "target": target,
            "mode": "shell",
            "timeout_ms": 30000,
        });
        let result = client.call_tool("run_command", args).await?;
        println!(
            "run_command result:\n{}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );
    } else {
        eprintln!(
            "(skip run_command) pass --target <name> to run a full command approval/execution test"
        );
        eprintln!("(timeout overrides) --mcp-initialize-timeout-ms <ms> --mcp-tools-call-timeout-ms <ms> --mcp-attempt-timeout-ms <ms>");
    }

    client.shutdown().await;

    if let Some(mcp_json) = test_config.mcp_config_json {
        if openai_config.base_url.is_empty() || openai_config.model.is_empty() {
            return Err("OPENAI_BASE_URL / OPENAI_MODEL 不能为空".to_string());
        }
        let parsed = parse_mcp_config_json(&mcp_json)?;
        let mut mcp_tools = Vec::new();
        for spec in &parsed.stdio_servers {
            let mcp_client = McpClient::start_with_spec(spec).await?;
            let result = mcp_client.list_tools().await?;
            for tool in result.tools {
                mcp_tools.push((spec.name.clone(), tool));
            }
            mcp_client.shutdown().await;
        }

        let mut used_names = HashSet::new();
        let mut openai_tools = Vec::new();
        let mut tool_names = Vec::new();
        for (server, tool) in &mcp_tools {
            let base = format!(
                "mcp_{}_{}",
                normalize_identifier(server),
                normalize_identifier(&tool.name)
            );
            let name = dedupe_name(&base, &mut used_names);
            tool_names.push(name.clone());
            openai_tools.push(OpenAiTool {
                tool_type: "function".to_string(),
                function: OpenAiToolFunction {
                    name,
                    description: tool.description.as_ref().map(|v| v.to_string()).unwrap_or_default(),
                    parameters: Value::Object(tool.input_schema.as_ref().clone()),
                },
            });
        }

        println!(
            "mcp tools ({}): {}",
            tool_names.len(),
            tool_names.join(", ")
        );

        let url = join_base_path(&openai_config.base_url, &openai_config.chat_path)
            .map_err(|err| err.to_string())?;
        let payload = json!({
            "model": openai_config.model,
            "messages": [
                { "role": "user", "content": test_config.question }
            ],
            "tools": openai_tools,
            "tool_choice": "auto"
        });
        println!(
            "openai request payload={}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
        let client = reqwest::Client::new();
        let mut request = client.post(url).json(&payload);
        if !openai_config.api_key.is_empty() {
            request = request.bearer_auth(openai_config.api_key);
        }
        let response = request.send().await.map_err(|err| err.to_string())?;
        let status = response.status();
        let text = response.text().await.map_err(|err| err.to_string())?;
        println!("openai status={}", status);
        println!("openai response={}", text);

        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            let content = value
                .get("choices")
                .and_then(|v| v.get(0))
                .and_then(|v| v.get("message"))
                .and_then(|v| v.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !content.is_empty() {
                let mentions = tool_names
                    .iter()
                    .filter(|name| content.contains(*name))
                    .cloned()
                    .collect::<Vec<_>>();
                println!("model mentions tools: {}", mentions.join(", "));
            }
        }
    }

    Ok(())
}

fn normalize_identifier(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "tool".to_string();
    }
    let mut normalized = String::with_capacity(trimmed.len());
    let mut prev_underscore = false;
    for ch in trimmed.chars() {
        let is_ok = ch.is_ascii_alphanumeric() || ch == '_';
        if is_ok {
            normalized.push(ch);
            prev_underscore = false;
        } else if !prev_underscore {
            normalized.push('_');
            prev_underscore = true;
        }
    }
    let normalized = normalized.trim_matches('_');
    if normalized.is_empty() {
        "tool".to_string()
    } else {
        normalized.to_string()
    }
}

fn dedupe_name(base: &str, used: &mut HashSet<String>) -> String {
    let mut candidate = base.to_string();
    let mut idx = 1;
    while used.contains(&candidate) {
        candidate = format!("{}_{}", base, idx);
        idx += 1;
    }
    used.insert(candidate.clone());
    candidate
}
