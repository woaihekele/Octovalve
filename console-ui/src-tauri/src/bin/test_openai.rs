use serde_json::{json, Value};
use std::path::PathBuf;

use octovalve_console::clients::mcp_client::McpClient;

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

fn parse_args() -> (PathBuf, Option<String>, McpTimeouts) {
    let mut args = std::env::args().skip(1);
    let mut proxy_config: Option<PathBuf> = None;
    let mut target: Option<String> = None;
    let mut timeouts = McpTimeouts::default();
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
    (proxy_config, target, timeouts)
}

#[tokio::main]
async fn main() -> Result<(), String> {
    // This config mirrors what console-ui would hold, but we don't actually call OpenAI here.
    // It exists so you can supply the same env vars you use in the app.
    let _openai_config = OpenAiConfig {
        base_url: env_or_default("OPENAI_BASE_URL", ""),
        api_key: env_or_default("OPENAI_API_KEY", ""),
        model: env_or_default("OPENAI_MODEL", ""),
        chat_path: env_or_default("OPENAI_CHAT_PATH", "/v1/chat/completions"),
    };

    let (proxy_config, target, timeouts) = parse_args();
    eprintln!("proxy_config={}", proxy_config.display());

    if let Some(value) = timeouts.initialize_timeout_ms {
        std::env::set_var("OCTOVALVE_MCP_INITIALIZE_TIMEOUT_MS", value.to_string());
    }
    if let Some(value) = timeouts.tools_call_timeout_ms {
        std::env::set_var("OCTOVALVE_MCP_TOOLS_CALL_TIMEOUT_MS", value.to_string());
    }
    if let Some(value) = timeouts.attempt_timeout_ms {
        std::env::set_var("OCTOVALVE_MCP_ATTEMPT_TIMEOUT_MS", value.to_string());
    }

    let client = McpClient::start(&proxy_config, "octovalve-console-openai").await?;

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
    if let Some(target) = target {
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

    Ok(())
}
