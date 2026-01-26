use std::path::{Path, PathBuf};
use std::sync::Arc;

use acp_codex::CliConfig;
use tauri::{AppHandle, Manager, State};

use crate::clients::acp_client::{AcpClient, AcpClientState};
use crate::clients::acp_types::{
    AcpInitResponse, AcpSessionInfo, ContentBlock, ContextItem, ListSessionsResult,
    LoadSessionResult,
};
use crate::paths::resolve_octovalve_proxy_bin;
use crate::services::console_sidecar::{build_console_path, DEFAULT_COMMAND_ADDR};
use crate::services::logging::append_log_line;
use crate::services::mcp_config::{build_octovalve_server, parse_mcp_config_json};
use crate::services::profiles::{expand_tilde_path, octovalve_dir};
use crate::state::{AppLogState, ProxyConfigState};

fn ensure_npm_env(app: &AppHandle, log_path: &Path) {
    let base = match octovalve_dir(app) {
        Ok(dir) => dir.join("npm"),
        Err(err) => {
            let _ = append_log_line(log_path, &format!("[acp_start] npm env skipped: {err}"));
            return;
        }
    };
    let cache = base.join("cache");
    let prefix = base.join("global");
    let prefix_lib = prefix.join("lib");
    let prefix_bin = prefix.join("bin");
    let prefix_modules = prefix_lib.join("node_modules");
    if let Err(err) = std::fs::create_dir_all(&cache) {
        let _ = append_log_line(log_path, &format!("[acp_start] npm cache dir error: {err}"));
    }
    if let Err(err) = std::fs::create_dir_all(&prefix) {
        let _ = append_log_line(
            log_path,
            &format!("[acp_start] npm prefix dir error: {err}"),
        );
    }
    if let Err(err) = std::fs::create_dir_all(&prefix_lib) {
        let _ = append_log_line(
            log_path,
            &format!("[acp_start] npm prefix lib error: {err}"),
        );
    }
    if let Err(err) = std::fs::create_dir_all(&prefix_bin) {
        let _ = append_log_line(
            log_path,
            &format!("[acp_start] npm prefix bin error: {err}"),
        );
    }
    if let Err(err) = std::fs::create_dir_all(&prefix_modules) {
        let _ = append_log_line(
            log_path,
            &format!("[acp_start] npm prefix node_modules error: {err}"),
        );
    }
    std::env::set_var("NPM_CONFIG_CACHE", cache.to_string_lossy().as_ref());
    std::env::set_var("NPM_CONFIG_PREFIX", prefix.to_string_lossy().as_ref());
    let _ = append_log_line(
        log_path,
        &format!(
            "[acp_start] npm env cache={} prefix={} lib={} bin={}",
            cache.display(),
            prefix.display(),
            prefix_lib.display(),
            prefix_bin.display()
        ),
    );
}

fn parse_acp_args(raw: Option<String>) -> Result<Vec<String>, String> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    shlex::split(trimmed).ok_or_else(|| "Invalid ACP arguments (check quotes)".to_string())
}

fn format_config_literal(value: &str) -> Result<String, String> {
    serde_json::to_string(value).map_err(|err| err.to_string())
}

fn build_mcp_cli_override(proxy_bin: &Path, proxy_config: &Path) -> Result<String, String> {
    let command_value = proxy_bin.to_string_lossy();
    let config_value = proxy_config.to_string_lossy();
    let command = format_config_literal(command_value.as_ref())?;
    let args = vec![
        format_config_literal("--config")?,
        format_config_literal(config_value.as_ref())?,
        format_config_literal("--command-addr")?,
        format_config_literal(DEFAULT_COMMAND_ADDR)?,
    ];
    let args_literal = format!("[{}]", args.join(", "));
    // Codex（app-server）对 MCP tool call 默认 60s 超时；给 octovalve MCP server 提高上限。
    // 注意：这是 Codex 的配置值（秒），不是我们的 MCP proxy 超时。
    let tool_timeout_sec = 60 * 60;
    Ok(format!(
        "mcp_servers.octovalve={{command={},args={},tool_timeout_sec={}}}",
        command, args_literal, tool_timeout_sec
    ))
}

fn build_mcp_servers(proxy_bin: &Path, proxy_config: &Path) -> Vec<serde_json::Value> {
    let (_, value) = build_octovalve_server(proxy_bin, proxy_config, DEFAULT_COMMAND_ADDR);
    vec![value]
}

fn resolve_acp_cwd(app: &AppHandle, cwd: &str) -> Result<PathBuf, String> {
    let trimmed = cwd.trim();
    let workspace_base = octovalve_dir(app)
        .map(|dir| dir.join("workspace"))
        .or_else(|_| app.path().app_config_dir().map(|dir| dir.join("workspace")))
        .or_else(|_| app.path().home_dir().map(|dir| dir.join("workspace")))
        .map_err(|err| err.to_string())?;

    std::fs::create_dir_all(&workspace_base).map_err(|err| err.to_string())?;

    if trimmed.is_empty() || trimmed == "." {
        return Ok(workspace_base);
    }

    let expanded = expand_tilde_path(app, trimmed).unwrap_or_else(|_| PathBuf::from(trimmed));
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        workspace_base.join(expanded)
    };
    std::fs::create_dir_all(&absolute).map_err(|err| err.to_string())?;
    Ok(absolute)
}

pub async fn acp_start(
    app: AppHandle,
    state: State<'_, AcpClientState>,
    proxy_state: State<'_, ProxyConfigState>,
    cwd: String,
    acp_args: Option<String>,
    mcp_config_json: Option<String>,
) -> Result<AcpInitResponse, String> {
    let log_path = app.state::<AppLogState>().app_log.clone();
    let _ = append_log_line(&log_path, &format!("[acp_start] called with cwd: {}", cwd));
    let cwd_value = std::env::current_dir()
        .ok()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| "<error>".to_string());
    let pwd_value = std::env::var("PWD").unwrap_or_else(|_| "<unset>".to_string());
    let home_value = std::env::var("HOME").unwrap_or_else(|_| "<unset>".to_string());
    let _ = append_log_line(
        &log_path,
        &format!(
            "[acp_start] env cwd={} PWD={} HOME={}",
            cwd_value, pwd_value, home_value
        ),
    );
    std::env::set_var("PATH", build_console_path());
    ensure_npm_env(&app, &log_path);

    let proxy_status = proxy_state.0.lock().unwrap().clone();
    let proxy_config_path = PathBuf::from(proxy_status.path);
    let proxy_bin = resolve_octovalve_proxy_bin().map_err(|e| {
        let _ = append_log_line(
            &log_path,
            &format!("[acp_start] proxy resolve error: {}", e),
        );
        e
    })?;
    let _ = append_log_line(
        &log_path,
        &format!(
            "[acp_start] proxy_bin={} proxy_config={}",
            proxy_bin.display(),
            proxy_config_path.display()
        ),
    );
    let mut parsed = parse_mcp_config_json(mcp_config_json.as_deref().unwrap_or(""))?;
    let mut uses_builtin = false;
    if !parsed.has_octovalve {
        let (_, value) =
            build_octovalve_server(&proxy_bin, &proxy_config_path, DEFAULT_COMMAND_ADDR);
        parsed.servers.push(value);
        uses_builtin = true;
    }
    let mcp_servers = parsed.servers;
    let _ = append_log_line(
        &log_path,
        &format!(
            "[acp_start] mcp_servers={}",
            serde_json::to_string(&mcp_servers).unwrap_or_else(|_| "<error>".to_string())
        ),
    );
    let mut acp_args = parse_acp_args(acp_args)?;
    if uses_builtin {
        let mcp_override = build_mcp_cli_override(&proxy_bin, &proxy_config_path)?;
        let _ = append_log_line(
            &log_path,
            &format!("[acp_start] mcp_override={}", mcp_override),
        );
        acp_args.push("-c".to_string());
        acp_args.push(mcp_override);
    }
    let cli_config = CliConfig::parse_from(acp_args).map_err(|err| err.to_string())?;

    let _ = append_log_line(&log_path, "[acp_start] starting new client...");
    let client = Arc::new(
        AcpClient::start(app.clone(), log_path.clone(), cli_config, mcp_servers)
            .await
            .map_err(|e| {
                let _ = append_log_line(&log_path, &format!("[acp_start] ACP error: {}", e));
                e.to_string()
            })?,
    );
    let init_result = client.initialize().await.map_err(|e| {
        let _ = append_log_line(&log_path, &format!("[acp_start] init error: {}", e));
        e.to_string()
    })?;

    let old_client = {
        let mut guard = state.0.lock().await;
        guard.replace(client)
    };
    if let Some(old_client) = old_client {
        let _ = append_log_line(&log_path, "[acp_start] stopping previous client");
        old_client.stop().await;
    }

    let _ = append_log_line(
        &log_path,
        &format!(
            "[acp_start] success, agent_info: {:?}, auth_methods: {:?}",
            init_result.agent_info, init_result.auth_methods
        ),
    );
    Ok(AcpInitResponse {
        agent_info: init_result.agent_info,
        auth_methods: init_result.auth_methods,
        agent_capabilities: init_result.agent_capabilities,
    })
}

pub async fn acp_authenticate(app: AppHandle, method_id: String) -> Result<(), String> {
    let client = {
        let state = app.state::<AcpClientState>();
        let guard = state.0.lock().await;
        guard.as_ref().cloned().ok_or("ACP client not started")?
    };

    client
        .authenticate(&method_id)
        .await
        .map_err(|e| e.to_string())
}

pub async fn acp_new_session(app: AppHandle, cwd: String) -> Result<AcpSessionInfo, String> {
    let log_path = app.state::<AppLogState>().app_log.clone();
    let _ = append_log_line(
        &log_path,
        &format!("[acp_new_session] called with cwd: {}", cwd),
    );
    let resolved_cwd = resolve_acp_cwd(&app, &cwd)?;
    let client = {
        let state = app.state::<AcpClientState>();
        let guard = state.0.lock().await;
        guard.as_ref().cloned().ok_or("ACP client not started")?
    };
    let result = client
        .new_session(&resolved_cwd.to_string_lossy())
        .await
        .map_err(|e| {
            let _ = append_log_line(&log_path, &format!("[acp_new_session] error: {}", e));
            e.to_string()
        })?;
    let _ = append_log_line(
        &log_path,
        &format!("[acp_new_session] session_id={}", result.session_id),
    );
    Ok(AcpSessionInfo {
        session_id: result.session_id,
        modes: vec![],
        models: vec![],
    })
}

pub async fn acp_load_session(
    app: AppHandle,
    session_id: String,
) -> Result<LoadSessionResult, String> {
    let client = {
        let state = app.state::<AcpClientState>();
        let guard = state.0.lock().await;
        guard.as_ref().cloned().ok_or("ACP client not started")?
    };
    client
        .load_session(&session_id)
        .await
        .map_err(|e| e.to_string())
}

pub async fn acp_list_sessions(app: AppHandle) -> Result<ListSessionsResult, String> {
    let log_path = app.state::<AppLogState>().app_log.clone();
    let _ = append_log_line(&log_path, "[acp_list_sessions] called");
    let client = {
        let state = app.state::<AcpClientState>();
        let guard = state.0.lock().await;
        guard.as_ref().cloned().ok_or("ACP client not started")?
    };
    client.list_sessions().await.map_err(|e| {
        let _ = append_log_line(&log_path, &format!("[acp_list_sessions] error: {}", e));
        e.to_string()
    })
}

pub async fn acp_delete_session(app: AppHandle, session_id: String) -> Result<(), String> {
    let client = {
        let state = app.state::<AcpClientState>();
        let guard = state.0.lock().await;
        guard.as_ref().cloned().ok_or("ACP client not started")?
    };
    client
        .delete_session(&session_id)
        .await
        .map_err(|e| e.to_string())
}

pub async fn acp_prompt(
    app: AppHandle,
    prompt: Vec<ContentBlock>,
    context: Option<Vec<ContextItem>>,
) -> Result<(), String> {
    let log_path = app.state::<AppLogState>().app_log.clone();
    let _ = append_log_line(
        &log_path,
        &format!("[acp_prompt] called with prompt blocks: {}", prompt.len()),
    );

    let client = {
        let state = app.state::<AcpClientState>();
        let guard = state.0.lock().await;
        guard.as_ref().cloned().ok_or("ACP client not started")?
    };

    let _ = append_log_line(&log_path, "[acp_prompt] calling client.prompt...");
    client.prompt(prompt, context).await.map_err(|e| {
        let _ = append_log_line(&log_path, &format!("[acp_prompt] error: {}", e));
        e.to_string()
    })?;
    let _ = append_log_line(&log_path, "[acp_prompt] done");
    Ok(())
}

pub async fn acp_cancel(app: AppHandle) -> Result<(), String> {
    let log_path = app.state::<AppLogState>().app_log.clone();
    let _ = append_log_line(&log_path, "[acp_cancel] request interrupt");
    let client = {
        let state = app.state::<AcpClientState>();
        let guard = state.0.lock().await;
        guard.as_ref().cloned().ok_or("ACP client not started")?
    };
    client.cancel().await.map_err(|e| {
        let _ = append_log_line(&log_path, &format!("[acp_cancel] error: {}", e));
        e.to_string()
    })?;
    let _ = append_log_line(&log_path, "[acp_cancel] interrupt sent");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn mcp_override_does_not_include_exec_mode() {
        let proxy_bin = PathBuf::from("/tmp/octovalve-proxy");
        let proxy_config = PathBuf::from("/tmp/local-proxy-config.toml");
        let value = build_mcp_cli_override(&proxy_bin, &proxy_config).unwrap();
        assert!(value.contains("mcp_servers.octovalve="));
        assert!(value.contains("--command-addr"));
        assert!(!value.contains("exec-mode"));
    }

    #[test]
    fn mcp_servers_args_exclude_exec_mode() {
        let proxy_bin = PathBuf::from("/tmp/octovalve-proxy");
        let proxy_config = PathBuf::from("/tmp/local-proxy-config.toml");
        let servers = build_mcp_servers(&proxy_bin, &proxy_config);
        let args = servers[0].get("args").and_then(|value| value.as_array()).unwrap();
        let args_text = args
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(args_text.contains("--command-addr"));
        assert!(!args_text.contains("exec-mode"));
    }
}

pub async fn acp_stop(state: State<'_, AcpClientState>) -> Result<(), String> {
    let client = {
        let mut guard = state.0.lock().await;
        guard.take()
    };

    if let Some(client) = client {
        client.stop().await;
    }
    Ok(())
}
