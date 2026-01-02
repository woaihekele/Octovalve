use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command as TokioCommand;
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

const MCP_INITIALIZE_TIMEOUT_DEFAULT: Duration = Duration::from_secs(60);
const MCP_TOOLS_CALL_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);
const MCP_ATTEMPT_TIMEOUT_DEFAULT: Duration = Duration::from_secs(300);

#[derive(Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Deserialize)]
struct JsonRpcError {
    message: String,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    id: Option<u64>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

async fn mcp_write_message<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    value: &Value,
) -> Result<(), String> {
    // rust-mcp-sdk stdio transport uses JSONL (one JSON message per line).
    // Keep the reader compatible with Content-Length too, but write JSONL for best interop.
    let mut payload = serde_json::to_vec(value).map_err(|err| err.to_string())?;
    payload.push(b'\n');
    writer.write_all(&payload).await.map_err(|err| err.to_string())?;
    writer.flush().await.map_err(|err| err.to_string())?;
    Ok(())
}

fn env_duration_ms(key: &str, default: Duration) -> Duration {
    let Ok(raw) = std::env::var(key) else {
        return default;
    };
    let Ok(ms) = raw.trim().parse::<u64>() else {
        return default;
    };
    Duration::from_millis(ms)
}

async fn mcp_read_message<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> Result<Value, String> {
    // Support both:
    // - JSONL: { ... }\n
    // - Content-Length framing: Content-Length: N\r\n\r\n<json>
    let mut first = String::new();
    let n = reader
        .read_line(&mut first)
        .await
        .map_err(|err| err.to_string())?;
    if n == 0 {
        return Err("mcp stream closed".to_string());
    }
    let first_trimmed = first.trim_end_matches(&['\r', '\n'][..]).trim();
    if first_trimmed.starts_with('{') {
        return serde_json::from_str(first_trimmed).map_err(|err| err.to_string());
    }

    let mut content_len: Option<usize> = None;
    let lower = first_trimmed.to_ascii_lowercase();
    if let Some(value) = lower.strip_prefix("content-length:") {
        let parsed = value
            .trim()
            .parse::<usize>()
            .map_err(|_| format!("invalid Content-Length header: {first_trimmed}"))?;
        content_len = Some(parsed);
    }

    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|err| err.to_string())?;
        if n == 0 {
            return Err("mcp stream closed".to_string());
        }
        let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
        if trimmed.is_empty() {
            break;
        }
        let lower = trimmed.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            let parsed = value
                .trim()
                .parse::<usize>()
                .map_err(|_| format!("invalid Content-Length header: {trimmed}"))?;
            content_len = Some(parsed);
        }
    }

    let len = content_len.ok_or_else(|| "missing Content-Length header".to_string())?;
    let mut buf = vec![0u8; len];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|err| err.to_string())?;
    serde_json::from_slice(&buf).map_err(|err| err.to_string())
}

async fn mcp_wait_response<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    want_id: u64,
) -> Result<JsonRpcResponse, String> {
    loop {
        let message = mcp_read_message(reader).await?;
        let parsed: Result<JsonRpcResponse, _> = serde_json::from_value(message);
        let Ok(response) = parsed else {
            continue;
        };
        if response.id == Some(want_id) {
            return Ok(response);
        }
    }
}

fn sidecar_path(name: &str) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    let dir = exe
        .parent()
        .ok_or_else(|| "failed to resolve sidecar dir".to_string())?;
    #[cfg(windows)]
    {
        return Ok(dir.join(format!("{name}.exe")));
    }
    #[cfg(not(windows))]
    {
        return Ok(dir.join(name));
    }
}

fn resolve_octovalve_proxy_bin() -> Result<PathBuf, String> {
    if let Ok(value) = std::env::var("OCTOVALVE_PROXY_BIN") {
        let candidate = PathBuf::from(value);
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    let candidate = sidecar_path("octovalve-proxy")?;
    if candidate.exists() {
        return Ok(candidate);
    }

    let mut cursor = std::env::current_exe().map_err(|err| err.to_string())?;
    for _ in 0..8 {
        if let Some(parent) = cursor.parent() {
            let release = parent.join("target").join("release").join("octovalve-proxy");
            if release.exists() {
                return Ok(release);
            }
            let debug = parent.join("target").join("debug").join("octovalve-proxy");
            if debug.exists() {
                return Ok(debug);
            }
            cursor = parent.to_path_buf();
        } else {
            break;
        }
    }

    Err("failed to locate octovalve-proxy binary (set OCTOVALVE_PROXY_BIN to override)"
        .to_string())
}

fn append_stderr(message: &mut String, stderr: &str) {
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        return;
    }
    message.push('\n');
    message.push_str(trimmed);
}

pub async fn call_tool(proxy_config_path: &Path, tool_name: &str, arguments: Value) -> Result<Value, String> {
    let proxy_bin = resolve_octovalve_proxy_bin()?;

    let initialize_timeout = env_duration_ms(
        "OCTOVALVE_MCP_INITIALIZE_TIMEOUT_MS",
        MCP_INITIALIZE_TIMEOUT_DEFAULT,
    );
    let tools_call_timeout = env_duration_ms(
        "OCTOVALVE_MCP_TOOLS_CALL_TIMEOUT_MS",
        MCP_TOOLS_CALL_TIMEOUT_DEFAULT,
    );
    let attempt_timeout = env_duration_ms(
        "OCTOVALVE_MCP_ATTEMPT_TIMEOUT_MS",
        MCP_ATTEMPT_TIMEOUT_DEFAULT,
    );

    let init = JsonRpcRequest {
        jsonrpc: "2.0",
        id: 1,
        method: "initialize".to_string(),
        params: Some(json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "octovalve-console", "version": "0.1.0"}
        })),
    };
    let call = JsonRpcRequest {
        jsonrpc: "2.0",
        id: 2,
        method: "tools/call".to_string(),
        params: Some(json!({
            "name": tool_name,
            "arguments": arguments,
        })),
    };

    // rust-mcp-sdk stdio transport uses JSONL (one JSON message per line), so we only write JSONL.
    // We keep the reader compatible with Content-Length too.
    let mut child = TokioCommand::new(&proxy_bin)
        .arg("--config")
        .arg(proxy_config_path)
        .arg("--client-id")
        .arg("octovalve-console-openai")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|err| err.to_string())?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "missing proxy stdin".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "missing proxy stdout".to_string())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "missing proxy stderr".to_string())?;
    let mut reader = BufReader::new(stdout);

    let stderr_buf: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let stderr_buf_task = Arc::clone(&stderr_buf);
    let stderr_task = tokio::spawn(async move {
        let mut buf = [0u8; 4096];
        loop {
            match stderr.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                    let mut guard = stderr_buf_task.lock().await;
                    guard.push_str(&chunk);
                }
                Err(_) => break,
            }
        }
    });

    let run = async {
        let init_value = serde_json::to_value(&init).map_err(|err| err.to_string())?;
        mcp_write_message(&mut stdin, &init_value).await?;

        let init_resp = timeout(initialize_timeout, mcp_wait_response(&mut reader, 1))
            .await
            .map_err(|_| "mcp initialize timed out".to_string())??;
        if let Some(err) = init_resp.error {
            return Err(format!("mcp initialize failed: {}", err.message));
        }

        let call_value = serde_json::to_value(&call).map_err(|err| err.to_string())?;
        mcp_write_message(&mut stdin, &call_value).await?;

        let resp = timeout(tools_call_timeout, mcp_wait_response(&mut reader, 2))
            .await
            .map_err(|_| "mcp tools/call timed out".to_string())??;
        if let Some(err) = resp.error {
            return Err(format!("mcp tools/call failed: {}", err.message));
        }
        resp.result.ok_or_else(|| "mcp tools/call missing result".to_string())
    };

    let run = timeout(attempt_timeout, run)
        .await
        .map_err(|_| "mcp attempt timed out".to_string())
        .and_then(|value| value);

    let _ = child.kill().await;
    let _ = stderr_task.await;
    let stderr_out = { stderr_buf.lock().await.clone() };

    match run {
        Ok(value) => Ok(value),
        Err(err) => {
            let mut message = format!("{err} (framing=jsonl)");
            append_stderr(&mut message, &stderr_out);
            Err(message)
        }
    }
}
