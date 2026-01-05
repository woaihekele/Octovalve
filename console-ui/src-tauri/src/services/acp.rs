use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager, State};

use crate::clients::acp_client::{self, AcpClient, AcpClientState};
use crate::clients::acp_types::{AcpInitResponse, AcpSessionInfo, ContextItem, LoadSessionResult};
use crate::services::console_sidecar::build_console_path;
use crate::services::logging::append_log_line;
use crate::services::profiles::octovalve_dir;
use crate::state::AppLogState;

fn resolve_codex_acp_path(app: &AppHandle, configured: Option<&str>) -> Result<PathBuf, String> {
    if let Some(value) = configured {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    if let Ok(dir) = octovalve_dir(app) {
        let custom_path = dir.join("codex-acp");
        if custom_path.exists() {
            return Ok(custom_path);
        }
    }

    let search_path = build_console_path();
    if let Some(found) = find_in_path("codex-acp", &search_path) {
        return Ok(found);
    }

    Ok(PathBuf::from("codex-acp"))
}

fn find_in_path(program: &str, path_var: &str) -> Option<PathBuf> {
    let candidates = path_var.split(':').filter(|value| !value.is_empty());
    for dir in candidates {
        let base = Path::new(dir);
        #[cfg(windows)]
        let candidate = base.join(format!("{program}.exe"));
        #[cfg(not(windows))]
        let candidate = base.join(program);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

pub async fn acp_start(
    app: AppHandle,
    state: State<'_, AcpClientState>,
    cwd: String,
    codex_acp_path: Option<String>,
) -> Result<AcpInitResponse, String> {
    let log_path = app.state::<AppLogState>().app_log.clone();
    let _ = append_log_line(&log_path, &format!("[acp_start] called with cwd: {}", cwd));

    let codex_acp_path = match resolve_codex_acp_path(&app, codex_acp_path.as_deref()) {
        Ok(path) => {
            let _ = append_log_line(
                &log_path,
                &format!("[acp_start] codex_acp_path resolved: {:?}", path),
            );
            path
        }
        Err(e) => {
            let _ = append_log_line(
                &log_path,
                &format!("[acp_start] failed to resolve codex_acp_path: {}", e),
            );
            return Err(e);
        }
    };

    {
        let mut guard = state.0.lock().unwrap();
        if let Some(mut client) = guard.take() {
            let _ = append_log_line(&log_path, "[acp_start] stopping existing client");
            client.stop();
        }
    }

    let _ = append_log_line(&log_path, "[acp_start] starting new client...");
    let app_clone = app.clone();
    let log_path_clone = log_path.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _ = append_log_line(
            &log_path_clone,
            "[acp_start] spawn_blocking: calling AcpClient::start",
        );
        let client = AcpClient::start(&codex_acp_path, app_clone, log_path_clone.clone())?;
        let _ = append_log_line(
            &log_path_clone,
            "[acp_start] spawn_blocking: client started, calling initialize",
        );
        let init_result = client.initialize()?;
        let _ = append_log_line(
            &log_path_clone,
            "[acp_start] spawn_blocking: initialize done",
        );
        Ok::<_, acp_client::AcpError>((client, init_result))
    })
    .await
    .map_err(|e| {
        let _ = append_log_line(&log_path, &format!("[acp_start] task error: {}", e));
        format!("Task error: {}", e)
    })?
    .map_err(|e| {
        let _ = append_log_line(&log_path, &format!("[acp_start] ACP error: {}", e));
        e.to_string()
    })?;
    let (client, init_result) = result;

    {
        let mut guard = state.0.lock().unwrap();
        *guard = Some(client);
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

pub fn acp_authenticate(state: State<'_, AcpClientState>, method_id: String) -> Result<(), String> {
    let guard = state.0.lock().unwrap();
    let client = guard.as_ref().ok_or("ACP client not started")?;
    client.authenticate(&method_id).map_err(|e| e.to_string())
}

pub fn acp_new_session(
    state: State<'_, AcpClientState>,
    cwd: String,
) -> Result<AcpSessionInfo, String> {
    let guard = state.0.lock().unwrap();
    let client = guard.as_ref().ok_or("ACP client not started")?;
    let result = client.new_session(&cwd).map_err(|e| e.to_string())?;
    Ok(AcpSessionInfo {
        session_id: result.session_id,
        modes: vec![],
        models: vec![],
    })
}

pub fn acp_load_session(
    state: State<'_, AcpClientState>,
    session_id: String,
) -> Result<LoadSessionResult, String> {
    let guard = state.0.lock().unwrap();
    let client = guard.as_ref().ok_or("ACP client not started")?;
    client.load_session(&session_id).map_err(|e| e.to_string())
}

pub fn acp_prompt(
    state: State<'_, AcpClientState>,
    log_state: State<'_, AppLogState>,
    content: String,
    context: Option<Vec<ContextItem>>,
) -> Result<(), String> {
    let _ = append_log_line(
        &log_state.app_log,
        &format!("[acp_prompt] called with content: {}", content),
    );
    let guard = state.0.lock().unwrap();
    let client = guard.as_ref().ok_or("ACP client not started")?;
    let _ = append_log_line(&log_state.app_log, "[acp_prompt] calling client.prompt...");
    client.prompt(&content, context).map_err(|e| {
        let _ = append_log_line(&log_state.app_log, &format!("[acp_prompt] error: {}", e));
        e.to_string()
    })?;
    let _ = append_log_line(&log_state.app_log, "[acp_prompt] done");
    Ok(())
}

pub fn acp_cancel(state: State<'_, AcpClientState>) -> Result<(), String> {
    let guard = state.0.lock().unwrap();
    let client = guard.as_ref().ok_or("ACP client not started")?;
    client.cancel().map_err(|e| e.to_string())
}

pub async fn acp_stop(state: State<'_, AcpClientState>) -> Result<(), String> {
    let client = {
        let mut guard = state.0.lock().unwrap();
        guard.take()
    };

    if let Some(mut client) = client {
        tokio::task::spawn_blocking(move || {
            client.stop();
        })
        .await
        .map_err(|e| format!("Failed to stop ACP: {}", e))?;
    }
    Ok(())
}
