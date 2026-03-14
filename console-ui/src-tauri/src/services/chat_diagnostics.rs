use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use acp_codex::CliConfig;
use reqwest::redirect::Policy;
use reqwest::Client;
use serde_json::json;
use tauri::{AppHandle, Manager};
use tokio::process::Command;

use crate::clients::acp_client::AcpClient;
use crate::paths::resolve_octovalve_proxy_bin;
use crate::services::http_utils::join_base_path;
use crate::services::logging::append_log_line;
use crate::services::mcp_config::{build_octovalve_server, parse_mcp_config_json, ParsedMcpConfig};
use crate::services::profiles::octovalve_dir;
use crate::services::startup_check;
use crate::types::{
    ChatProviderCheckInput, ChatProviderCheckItem, ChatProviderCheckResult, ProfilesFile,
    ProxyConfigStatus,
};

const OPENAI_TIMEOUT_SECS: u64 = 20;
const ACP_TIMEOUT_SECS: u64 = 25;

pub async fn run_chat_provider_checks(
    app: &AppHandle,
    log_path: &Path,
    proxy_status: &ProxyConfigStatus,
    profiles: &ProfilesFile,
    input: ChatProviderCheckInput,
) -> ChatProviderCheckResult {
    let mut items = Vec::new();

    let startup_item = match startup_check::validate_startup_config(app, proxy_status, profiles) {
        Ok(result) if result.ok => pass_item(
            "startup-config",
            "Local startup config",
            format!(
                "Proxy and broker config parsed successfully. proxy={}, broker={}",
                result.proxy_path, result.broker_path
            ),
        ),
        Ok(result) => fail_item(
            "startup-config",
            "Local startup config",
            join_messages(&result.errors),
            Some(format!(
                "Open the Config tab and fix the files at {} and {}.",
                result.proxy_path, result.broker_path
            )),
        ),
        Err(err) => fail_item(
            "startup-config",
            "Local startup config",
            err,
            Some("Check local profile, proxy config, and broker config paths.".to_string()),
        ),
    };
    items.push(startup_item);

    let parsed_mcp = match parse_mcp_config_json(&input.mcp_config_json) {
        Ok(parsed) => {
            let detail = if parsed.servers.is_empty() {
                "No custom MCP servers configured.".to_string()
            } else {
                format!(
                    "Parsed {} MCP server(s); {} enabled stdio server(s).",
                    parsed.servers.len(),
                    parsed.stdio_servers.len()
                )
            };
            items.push(pass_item("mcp-json", "MCP JSON", detail));
            if parsed.has_octovalve {
                items.push(pass_item(
                    "mcp-octovalve",
                    "Built-in octovalve bridge",
                    "Explicit octovalve MCP server is present in MCP JSON.".to_string(),
                ));
            } else {
                items.push(pass_item(
                    "mcp-octovalve",
                    "Built-in octovalve bridge",
                    "No explicit octovalve MCP server found; the app will auto-inject its built-in bridge for ACP.".to_string(),
                ));
            }
            for server_item in diagnose_mcp_stdio_servers(&parsed) {
                items.push(server_item);
            }
            Some(parsed)
        }
        Err(err) => {
            items.push(fail_item(
                "mcp-json",
                "MCP JSON",
                err,
                Some("Fix the JSON syntax or use the standard { mcpServers: { ... } } shape.".to_string()),
            ));
            None
        }
    };

    if should_check_openai(&input) {
        items.push(run_openai_check(log_path, &input).await);
    } else {
        items.push(skip_item(
            "openai-api",
            "OpenAI-compatible API",
            "Skipped because the OpenAI section is not active and no OpenAI fields are filled in.".to_string(),
        ));
    }

    if should_check_acp(&input) {
        items.extend(run_acp_checks(app, log_path, proxy_status, &input, parsed_mcp).await);
    } else {
        items.push(skip_item(
            "acp-codex",
            "ACP / Codex",
            "Skipped because ACP is not active and no custom Codex path is configured.".to_string(),
        ));
    }

    let ok = items.iter().all(|item| item.status != "fail");
    ChatProviderCheckResult {
        ok,
        checked_at: now_ms(),
        items,
    }
}

fn diagnose_mcp_stdio_servers(parsed: &ParsedMcpConfig) -> Vec<ChatProviderCheckItem> {
    parsed
        .stdio_servers
        .iter()
        .map(|server| {
            let key = format!("mcp-server-{}", server.name);
            let label = format!("MCP server: {}", server.name);
            let command_display = server.command.display().to_string();
            let resolved = resolve_command_path(&server.command);
            match resolved {
                Some(command_path) => {
                    let cwd_note = match &server.cwd {
                        Some(cwd) if !cwd.exists() => {
                            return warn_item(
                                key,
                                label,
                                format!(
                                    "Command resolved to {}, but cwd does not exist: {}",
                                    command_path.display(),
                                    cwd.display()
                                ),
                                Some("Create the working directory or remove the `cwd` override.".to_string()),
                            )
                        }
                        Some(cwd) => format!(", cwd={}", cwd.display()),
                        None => String::new(),
                    };
                    pass_item(
                        key,
                        label,
                        format!("Command resolved to {}{}", command_path.display(), cwd_note),
                    )
                }
                None => fail_item(
                    key,
                    label,
                    format!("Command not found: {}", command_display),
                    Some(
                        "Install the command, fix the absolute path, or ensure it is available on PATH."
                            .to_string(),
                    ),
                ),
            }
        })
        .collect()
}

async fn run_openai_check(log_path: &Path, input: &ChatProviderCheckInput) -> ChatProviderCheckItem {
    let config = &input.openai;
    if config.base_url.trim().is_empty() {
        return fail_item(
            "openai-api",
            "OpenAI-compatible API",
            "Base URL is empty.".to_string(),
            Some("Fill in the API base URL, for example https://api.openai.com.".to_string()),
        );
    }
    if config.chat_path.trim().is_empty() {
        return fail_item(
            "openai-api",
            "OpenAI-compatible API",
            "Chat Path is empty.".to_string(),
            Some("Use the chat endpoint path, for example /v1/chat/completions.".to_string()),
        );
    }
    if config.model.trim().is_empty() {
        return fail_item(
            "openai-api",
            "OpenAI-compatible API",
            "Model is empty.".to_string(),
            Some("Choose a model that your provider actually supports.".to_string()),
        );
    }
    if config.api_key.trim().is_empty() {
        return fail_item(
            "openai-api",
            "OpenAI-compatible API",
            "API key is empty.".to_string(),
            Some("Paste a valid API key before running the check.".to_string()),
        );
    }

    let url = match join_base_path(&config.base_url, &config.chat_path) {
        Ok(url) => url,
        Err(err) => {
            return fail_item(
                "openai-api",
                "OpenAI-compatible API",
                err,
                Some("Check the Base URL and Chat Path fields.".to_string()),
            )
        }
    };

    let payload = json!({
        "model": config.model,
        "messages": [{ "role": "user", "content": "ping" }],
        "max_tokens": 1,
        "stream": false,
    });
    let _ = append_log_line(log_path, &format!("[chat_check] openai probe url={url}"));

    let client = match Client::builder()
        .http1_only()
        .redirect(Policy::none())
        .pool_max_idle_per_host(0)
        .timeout(Duration::from_secs(OPENAI_TIMEOUT_SECS))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            return fail_item(
                "openai-api",
                "OpenAI-compatible API",
                format!("Failed to build HTTP client: {err}"),
                Some("Check local proxy and TLS settings.".to_string()),
            )
        }
    };

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key.trim()))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("User-Agent", "octovalve-config-check")
        .json(&payload)
        .send()
        .await;

    match response {
        Ok(response) if response.status().is_success() => pass_item(
            "openai-api",
            "OpenAI-compatible API",
            format!(
                "Probe request succeeded via {} with model {}.",
                url, config.model
            ),
        ),
        Ok(response) => {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let snippet = truncate_body(&body);
            match status.as_u16() {
                401 => fail_item(
                    "openai-api",
                    "OpenAI-compatible API",
                    format!("HTTP 401 Unauthorized. {snippet}"),
                    Some("The API key is likely invalid, expired, or not accepted by this endpoint.".to_string()),
                ),
                403 => fail_item(
                    "openai-api",
                    "OpenAI-compatible API",
                    format!("HTTP 403 Forbidden. {snippet}"),
                    Some("Check account permissions, project restrictions, or provider allowlists.".to_string()),
                ),
                404 => fail_item(
                    "openai-api",
                    "OpenAI-compatible API",
                    format!("HTTP 404 Not Found. {snippet}"),
                    Some("The Base URL or Chat Path is probably wrong for this provider.".to_string()),
                ),
                429 => warn_item(
                    "openai-api",
                    "OpenAI-compatible API",
                    format!("HTTP 429 Too Many Requests. {snippet}"),
                    Some("The config is probably valid, but the account is rate-limited or quota-limited right now.".to_string()),
                ),
                _ => fail_item(
                    "openai-api",
                    "OpenAI-compatible API",
                    format!("HTTP {}. {snippet}", status.as_u16()),
                    Some("Check the model name, endpoint compatibility, and provider-specific request format.".to_string()),
                ),
            }
        }
        Err(err) => fail_item(
            "openai-api",
            "OpenAI-compatible API",
            format!("Request failed: {err}"),
            Some("Check network access, proxy settings, Base URL, and TLS certificates.".to_string()),
        ),
    }
}

async fn run_acp_checks(
    app: &AppHandle,
    log_path: &Path,
    proxy_status: &ProxyConfigStatus,
    input: &ChatProviderCheckInput,
    parsed_mcp: Option<ParsedMcpConfig>,
) -> Vec<ChatProviderCheckItem> {
    let mut items = Vec::new();
    let codex_program = input.acp.codex_path.trim();
    let codex_program = if codex_program.is_empty() {
        if cfg!(windows) {
            "codex.cmd".to_string()
        } else {
            "codex".to_string()
        }
    } else {
        codex_program.to_string()
    };

    match probe_codex_command(&codex_program).await {
        Ok(detail) => items.push(pass_item("codex-cli", "Codex CLI", detail)),
        Err((detail, suggestion)) => {
            items.push(fail_item("codex-cli", "Codex CLI", detail, Some(suggestion)));
            items.push(skip_item(
                "acp-init",
                "ACP initialization",
                "Skipped because Codex CLI is not ready.".to_string(),
            ));
            return items;
        }
    }

    let parsed_mcp = match parsed_mcp {
        Some(parsed) => parsed,
        None => {
            items.push(skip_item(
                "acp-init",
                "ACP initialization",
                "Skipped because MCP JSON failed to parse.".to_string(),
            ));
            return items;
        }
    };

    let proxy_bin = match resolve_octovalve_proxy_bin() {
        Ok(path) => path,
        Err(err) => {
            items.push(fail_item(
                "acp-init",
                "ACP initialization",
                err,
                Some("Rebuild or reinstall the app sidecars, or set OCTOVALVE_PROXY_BIN.".to_string()),
            ));
            return items;
        }
    };
    if !proxy_status.present {
        items.push(fail_item(
            "acp-init",
            "ACP initialization",
            format!("Local proxy config is missing: {}", proxy_status.path),
            Some("Create the local proxy config from the example file before using ACP.".to_string()),
        ));
        return items;
    }

    let proxy_config_path = PathBuf::from(proxy_status.path.clone());
    let mut mcp_servers = parsed_mcp.servers;
    if !parsed_mcp.has_octovalve {
        let (_, value) = build_octovalve_server(&proxy_bin, &proxy_config_path, "127.0.0.1:0");
        mcp_servers.push(value);
    }

    let cwd = match diagnostic_acp_cwd(app) {
        Ok(path) => path,
        Err(err) => {
            items.push(fail_item(
                "acp-init",
                "ACP initialization",
                err,
                Some("Ensure the app can create and access its workspace directory.".to_string()),
            ));
            return items;
        }
    };

    let cli_config = CliConfig {
        codex_path: non_empty(input.acp.codex_path.trim()),
        codex_home: diagnostic_codex_home(app).ok(),
        approval_policy: normalized_auto(input.acp.approval_policy.trim()),
        sandbox_mode: normalized_auto(input.acp.sandbox_mode.trim()),
        app_server_args: Vec::new(),
    };

    if let Some(codex_home) = cli_config.codex_home.clone() {
        std::env::set_var("CODEX_HOME", codex_home);
    }

    let _ = append_log_line(
        log_path,
        &format!("[chat_check] acp init codex={} cwd={}", codex_program, cwd.display()),
    );
    let probe = tokio::time::timeout(
        Duration::from_secs(ACP_TIMEOUT_SECS),
        async {
            let client = AcpClient::start(app.clone(), log_path.to_path_buf(), cli_config, mcp_servers)
                .await
                .map_err(|err| err.to_string())?;
            let init_result = client.initialize().await.map_err(|err| err.to_string())?;
            let new_session = client
                .new_session(cwd.to_string_lossy().as_ref())
                .await
                .map_err(|err| err.to_string());
            client.stop().await;
            let agent_name = init_result
                .agent_info
                .as_ref()
                .map(|info| info.title.clone().unwrap_or_else(|| info.name.clone()))
                .unwrap_or_else(|| "Codex".to_string());
            new_session.map(|session| (agent_name, session.session_id))
        },
    )
    .await;

    match probe {
        Ok(Ok((agent_name, session_id))) => items.push(pass_item(
            "acp-init",
            "ACP initialization",
            format!("ACP initialized successfully with agent {agent_name}; test session {session_id} created."),
        )),
        Ok(Err(err)) => items.push(fail_item(
            "acp-init",
            "ACP initialization",
            err.clone(),
            Some(suggestion_for_acp_error(&err)),
        )),
        Err(_) => items.push(fail_item(
            "acp-init",
            "ACP initialization",
            format!("Timed out after {} seconds.", ACP_TIMEOUT_SECS),
            Some("Codex may be blocked by trust prompts, a broken MCP server, or a stalled local environment.".to_string()),
        )),
    }

    items
}

async fn probe_codex_command(program: &str) -> Result<String, (String, String)> {
    let output = Command::new(program)
        .arg("--version")
        .output()
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => (
                format!("Command not found: {program}"),
                "Install Codex CLI or set the full Codex Path in chat settings.".to_string(),
            ),
            std::io::ErrorKind::PermissionDenied => (
                format!("Command is not executable: {program}"),
                "Check file permissions and make sure the path points to the actual Codex executable.".to_string(),
            ),
            _ => (
                format!("Failed to start {program}: {err}"),
                "Check the Codex Path and try the command manually in a terminal.".to_string(),
            ),
        })?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let version = stdout.lines().next().unwrap_or("Codex CLI is executable").trim();
        Ok(version.to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err((
            if stderr.is_empty() {
                format!("{} exited with status {}", program, output.status)
            } else {
                format!("{}", stderr)
            },
            "Run `codex --version` manually and verify the configured path points to a working install.".to_string(),
        ))
    }
}

fn resolve_command_path(command: &Path) -> Option<PathBuf> {
    if command.is_absolute() {
        return command.exists().then(|| command.to_path_buf());
    }
    let command_name = command.as_os_str();
    let path_var = std::env::var_os("PATH")?;
    let mut candidates = Vec::new();
    #[cfg(windows)]
    {
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        let exts: Vec<String> = pathext.split(';').map(|value| value.to_ascii_lowercase()).collect();
        let has_ext = command
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| format!(".{}", value).to_ascii_lowercase())
            .is_some_and(|value| exts.contains(&value));
        for dir in std::env::split_paths(&path_var) {
            let base = dir.join(command_name);
            if has_ext && base.exists() {
                return Some(base);
            }
            candidates.push(base.clone());
            for ext in &exts {
                let trimmed = ext.trim_start_matches('.');
                let with_ext = dir.join(format!("{}.{}", command.to_string_lossy(), trimmed));
                if with_ext.exists() {
                    return Some(with_ext);
                }
                candidates.push(with_ext);
            }
        }
    }
    #[cfg(not(windows))]
    {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(command_name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    candidates.into_iter().find(|candidate| candidate.exists())
}

fn diagnostic_acp_cwd(app: &AppHandle) -> Result<PathBuf, String> {
    let base = octovalve_dir(app)
        .map(|dir| dir.join("workspace"))
        .or_else(|_| app.path().app_config_dir().map(|dir| dir.join("workspace")))
        .or_else(|_| app.path().home_dir().map(|dir| dir.join("workspace")))
        .map_err(|err| err.to_string())?;
    fs::create_dir_all(&base).map_err(|err| err.to_string())?;
    Ok(base)
}

fn diagnostic_codex_home(app: &AppHandle) -> Result<String, String> {
    let dir = octovalve_dir(app)
        .map(|dir| dir.join("codex"))
        .or_else(|_| app.path().app_config_dir().map(|dir| dir.join("codex")))
        .map_err(|err| err.to_string())?;
    fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    Ok(dir.to_string_lossy().to_string())
}

fn normalized_auto(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "auto" {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn should_check_openai(input: &ChatProviderCheckInput) -> bool {
    input.provider == "openai"
        || !input.openai.base_url.trim().is_empty()
        || !input.openai.api_key.trim().is_empty()
        || !input.openai.model.trim().is_empty()
        || !input.openai.chat_path.trim().is_empty()
}

fn should_check_acp(input: &ChatProviderCheckInput) -> bool {
    input.provider == "acp" || !input.acp.codex_path.trim().is_empty()
}

fn suggestion_for_acp_error(error: &str) -> String {
    let normalized = error.trim();
    match normalized {
        "CODEX_NOT_FOUND" => {
            "Codex CLI is missing. Install Codex or fill in the Codex Path field with the executable location.".to_string()
        }
        "CODEX_NOT_EXECUTABLE" => {
            "Codex CLI exists but is not executable. Fix permissions or point Codex Path at a valid executable.".to_string()
        }
        "CODEX_CONFIG_UNTRUSTED" => {
            "Codex refused to use the current config because the project is not trusted. Trust the project in Codex config and retry.".to_string()
        }
        _ if normalized.contains("MCP JSON") => {
            "Fix the MCP JSON first; ACP forwards it directly to Codex.".to_string()
        }
        _ => "Check Codex Path, MCP server commands, local proxy config, and Codex trust settings.".to_string(),
    }
}

fn join_messages(messages: &[String]) -> String {
    messages
        .iter()
        .map(|message| message.trim())
        .filter(|message| !message.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate_body(body: &str) -> String {
    let normalized = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let truncated: String = chars.by_ref().take(240).collect();
    if chars.next().is_some() {
        format!("Response: {}…", truncated)
    } else if truncated.is_empty() {
        "Response body is empty.".to_string()
    } else {
        format!("Response: {}", truncated)
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn pass_item(key: impl Into<String>, label: impl Into<String>, detail: impl Into<String>) -> ChatProviderCheckItem {
    ChatProviderCheckItem {
        key: key.into(),
        label: label.into(),
        status: "pass".to_string(),
        detail: detail.into(),
        suggestion: None,
    }
}

fn warn_item(
    key: impl Into<String>,
    label: impl Into<String>,
    detail: impl Into<String>,
    suggestion: Option<String>,
) -> ChatProviderCheckItem {
    ChatProviderCheckItem {
        key: key.into(),
        label: label.into(),
        status: "warn".to_string(),
        detail: detail.into(),
        suggestion,
    }
}

fn fail_item(
    key: impl Into<String>,
    label: impl Into<String>,
    detail: impl Into<String>,
    suggestion: Option<String>,
) -> ChatProviderCheckItem {
    ChatProviderCheckItem {
        key: key.into(),
        label: label.into(),
        status: "fail".to_string(),
        detail: detail.into(),
        suggestion,
    }
}

fn skip_item(key: impl Into<String>, label: impl Into<String>, detail: impl Into<String>) -> ChatProviderCheckItem {
    ChatProviderCheckItem {
        key: key.into(),
        label: label.into(),
        status: "skip".to_string(),
        detail: detail.into(),
        suggestion: None,
    }
}
