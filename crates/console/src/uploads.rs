use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::RwLock;
use tokio::time::timeout;

use system_utils::path::expand_tilde;
use system_utils::ssh::apply_askpass_env;

use crate::state::TargetSpec;

const LIST_DIR_TIMEOUT: Duration = Duration::from_secs(8);
const UPLOAD_CHUNK_SIZE: usize = 64 * 1024;

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UploadState {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Serialize)]
pub(crate) struct UploadStatus {
    pub(crate) id: String,
    pub(crate) target: String,
    pub(crate) local_path: String,
    pub(crate) remote_path: String,
    pub(crate) status: UploadState,
    pub(crate) total_bytes: u64,
    pub(crate) sent_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

#[derive(Clone, Serialize)]
pub(crate) struct DirectoryEntry {
    pub(crate) name: String,
    pub(crate) path: String,
}

#[derive(Clone)]
pub(crate) struct UploadRegistry {
    inner: Arc<RwLock<HashMap<String, UploadStatus>>>,
}

impl UploadRegistry {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub(crate) async fn insert(&self, status: UploadStatus) {
        let mut guard = self.inner.write().await;
        guard.insert(status.id.clone(), status);
    }

    pub(crate) async fn get(&self, id: &str) -> Option<UploadStatus> {
        let guard = self.inner.read().await;
        guard.get(id).cloned()
    }

    pub(crate) async fn update<F>(&self, id: &str, update: F)
    where
        F: FnOnce(&mut UploadStatus),
    {
        let mut guard = self.inner.write().await;
        if let Some(status) = guard.get_mut(id) {
            update(status);
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct UploadRequest {
    pub(crate) local_path: String,
    pub(crate) remote_path: String,
}

pub(crate) fn normalize_remote_path(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("remote path is empty".to_string());
    }
    if trimmed.ends_with('/') {
        return Err("remote path must include a file name".to_string());
    }
    Ok(trimmed.to_string())
}

pub(crate) fn normalize_dir_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) async fn resolve_remote_dir_path(
    target: &TargetSpec,
    path: &str,
) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "~" || trimmed == "~/" {
        return Ok(fetch_remote_home(target).await.unwrap_or_else(|_| "/".to_string()));
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        let home = fetch_remote_home(target).await.unwrap_or_else(|_| "/".to_string());
        if rest.is_empty() {
            return Ok(home);
        }
        return Ok(join_remote_path(&home, rest));
    }
    Ok(normalize_dir_path(trimmed))
}

pub(crate) async fn list_remote_directories(
    target: &TargetSpec,
    path: &str,
) -> Result<Vec<DirectoryEntry>, String> {
    let ssh = target
        .ssh
        .as_deref()
        .ok_or_else(|| "missing ssh target".to_string())?;
    let normalized = normalize_dir_path(path);
    let list_command = build_list_command(&normalized);

    let mut cmd = Command::new("ssh");
    if let Some(password) = target.ssh_password.as_deref() {
        apply_askpass_env(&mut cmd, password).map_err(|err| err.to_string())?;
    }
    apply_ssh_options(&mut cmd, target.ssh_password.is_some());
    for arg in &target.ssh_args {
        cmd.arg(arg);
    }
    cmd.arg(ssh);
    cmd.arg(list_command);

    let output = match timeout(LIST_DIR_TIMEOUT, cmd.output()).await {
        Ok(result) => result.map_err(|err| err.to_string())?,
        Err(_) => return Err("list directory timed out".to_string()),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            format!("list directory failed with status {:?}", output.status.code())
        } else {
            stderr
        };
        return Err(message);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if !trimmed.ends_with('/') {
            continue;
        }
        let name = trimmed.trim_end_matches('/').trim();
        if name.is_empty() || name == "." || name == ".." {
            continue;
        }
        let path = join_remote_path(&normalized, name);
        entries.push(DirectoryEntry {
            name: name.to_string(),
            path,
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

async fn fetch_remote_home(target: &TargetSpec) -> Result<String, String> {
    let ssh = target
        .ssh
        .as_deref()
        .ok_or_else(|| "missing ssh target".to_string())?;
    let mut cmd = Command::new("ssh");
    if let Some(password) = target.ssh_password.as_deref() {
        apply_askpass_env(&mut cmd, password).map_err(|err| err.to_string())?;
    }
    apply_ssh_options(&mut cmd, target.ssh_password.is_some());
    for arg in &target.ssh_args {
        cmd.arg(arg);
    }
    cmd.arg(ssh);
    cmd.arg(build_home_command());

    let output = match timeout(LIST_DIR_TIMEOUT, cmd.output()).await {
        Ok(result) => result.map_err(|err| err.to_string())?,
        Err(_) => return Err("home lookup timed out".to_string()),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            format!("home lookup failed with status {:?}", output.status.code())
        } else {
            stderr
        };
        return Err(message);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut home = stdout.trim().to_string();
    while home.ends_with('/') && home.len() > 1 {
        home.pop();
    }
    if home.is_empty() {
        Ok("/".to_string())
    } else {
        Ok(home)
    }
}

pub(crate) async fn start_upload(
    registry: UploadRegistry,
    target: TargetSpec,
    local_path: String,
    remote_path: String,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let initial = UploadStatus {
        id: id.clone(),
        target: target.name.clone(),
        local_path: local_path.clone(),
        remote_path: remote_path.clone(),
        status: UploadState::Pending,
        total_bytes: 0,
        sent_bytes: 0,
        error: None,
    };

    registry.insert(initial).await;
    let registry_clone = registry.clone();
    let upload_id = id.clone();
    tokio::spawn(async move {
        if let Err(_err) =
            run_upload(registry_clone, target, local_path, remote_path, upload_id).await
        {
        }
    });
    Ok(id)
}

async fn run_upload(
    registry: UploadRegistry,
    target: TargetSpec,
    local_path: String,
    remote_path: String,
    id: String,
) -> Result<(), String> {
    let normalized_remote = match normalize_remote_path(&remote_path) {
        Ok(path) => path,
        Err(err) => {
            registry
                .update(&id, |status| {
                    status.status = UploadState::Failed;
                    status.error = Some(err.clone());
                })
                .await;
            return Err(err);
        }
    };
    let resolved_local = expand_tilde(&local_path);
    let mut file = match tokio::fs::File::open(&resolved_local).await {
        Ok(file) => file,
        Err(err) => {
            registry
                .update(&id, |status| {
                    status.status = UploadState::Failed;
                    status.error = Some(format!("failed to open file: {err}"));
                })
                .await;
            return Err(err.to_string());
        }
    };
    let metadata = match file.metadata().await {
        Ok(meta) => meta,
        Err(err) => {
            registry
                .update(&id, |status| {
                    status.status = UploadState::Failed;
                    status.error = Some(format!("failed to read file metadata: {err}"));
                })
                .await;
            return Err(err.to_string());
        }
    };
    let total_bytes = metadata.len();

    registry
        .update(&id, |status| {
            status.status = UploadState::Running;
            status.total_bytes = total_bytes;
            status.sent_bytes = 0;
            status.local_path = resolved_local.to_string_lossy().to_string();
            status.remote_path = normalized_remote.clone();
            status.error = None;
        })
        .await;

    let ssh = target
        .ssh
        .as_deref()
        .ok_or_else(|| "missing ssh target".to_string())?;
    let upload_command = build_upload_command(&normalized_remote);
    let mut cmd = Command::new("ssh");
    if let Some(password) = target.ssh_password.as_deref() {
        apply_askpass_env(&mut cmd, password).map_err(|err| err.to_string())?;
    }
    apply_ssh_options(&mut cmd, target.ssh_password.is_some());
    for arg in &target.ssh_args {
        cmd.arg(arg);
    }
    cmd.arg(ssh);
    cmd.arg(upload_command);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            registry
                .update(&id, |status| {
                    status.status = UploadState::Failed;
                    status.error = Some(format!("failed to spawn ssh: {err}"));
                })
                .await;
            return Err(err.to_string());
        }
    };
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "failed to open ssh stdin".to_string())?;
    let mut sent_bytes = 0u64;
    let mut buffer = vec![0u8; UPLOAD_CHUNK_SIZE];
    loop {
        let read = match file.read(&mut buffer).await {
            Ok(size) => size,
            Err(err) => {
                registry
                    .update(&id, |status| {
                        status.status = UploadState::Failed;
                        status.error = Some(format!("failed to read file: {err}"));
                    })
                    .await;
                return Err(err.to_string());
            }
        };
        if read == 0 {
            break;
        }
        if let Err(err) = stdin.write_all(&buffer[..read]).await {
            registry
                .update(&id, |status| {
                    status.status = UploadState::Failed;
                    status.error = Some(format!("failed to send file: {err}"));
                })
                .await;
            return Err(err.to_string());
        }
        sent_bytes = sent_bytes.saturating_add(read as u64);
        registry
            .update(&id, |status| {
                status.sent_bytes = sent_bytes;
            })
            .await;
    }
    drop(stdin);
    let output = match child.wait_with_output().await {
        Ok(output) => output,
        Err(err) => {
            registry
                .update(&id, |status| {
                    status.status = UploadState::Failed;
                    status.error = Some(format!("failed to wait for ssh: {err}"));
                })
                .await;
            return Err(err.to_string());
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            format!("upload failed with status {:?}", output.status.code())
        } else {
            stderr
        };
        registry
            .update(&id, |status| {
                status.status = UploadState::Failed;
                status.error = Some(message.clone());
                status.sent_bytes = sent_bytes;
            })
            .await;
        return Err(message);
    }

    registry
        .update(&id, |status| {
            status.status = UploadState::Completed;
            status.sent_bytes = sent_bytes;
            status.error = None;
        })
        .await;
    Ok(())
}

fn build_list_command(path: &str) -> String {
    let mut command = String::new();
    command.push_str("ls -1 -p -a -- ");
    command.push_str(&shell_escape(path));
    format!(
        "bash --noprofile -lc {}",
        shell_escape(&command)
    )
}

fn build_home_command() -> String {
    let command = "printf %s \"$HOME\"";
    format!("bash --noprofile -lc {}", shell_escape(command))
}

fn build_upload_command(remote_path: &str) -> String {
    let parent = remote_parent_dir(remote_path);
    let command = format!(
        "mkdir -p {} && cat > {}",
        shell_escape(&parent),
        shell_escape(remote_path)
    );
    format!(
        "bash --noprofile -lc {}",
        shell_escape(&command)
    )
}

fn join_remote_path(base: &str, name: &str) -> String {
    if base == "/" {
        format!("/{name}")
    } else if base.ends_with('/') {
        format!("{base}{name}")
    } else {
        format!("{base}/{name}")
    }
}

fn remote_parent_dir(path: &str) -> String {
    if let Some((parent, _)) = path.rsplit_once('/') {
        if parent.is_empty() {
            "/".to_string()
        } else {
            parent.to_string()
        }
    } else {
        ".".to_string()
    }
}

fn apply_ssh_options(cmd: &mut Command, has_password: bool) {
    cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
    cmd.arg("-o").arg("ConnectTimeout=10");
    if !has_password {
        cmd.arg("-o").arg("BatchMode=yes");
    }
}

fn shell_escape(value: &str) -> String {
    let mut escaped = String::from("'");
    for ch in value.chars() {
        if ch == '\'' {
            escaped.push_str("'\"'\"'");
        } else {
            escaped.push(ch);
        }
    }
    escaped.push('\'');
    escaped
}
