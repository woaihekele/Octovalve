use crate::config::LimitsConfig;
use crate::whitelist::Whitelist;
use anyhow::Context;
use protocol::{CommandMode, CommandRequest, CommandResponse, CommandStage, CommandStatus};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::Mutex;

pub async fn execute_request(
    request: &CommandRequest,
    whitelist: &Whitelist,
    limits: &LimitsConfig,
    output_dir: &Path,
    enforce_allowlist: bool,
) -> CommandResponse {
    let started_at = Instant::now();

    if matches!(&request.mode, CommandMode::Shell) && request.raw_command.trim().is_empty() {
        let response = CommandResponse::error(request.id.clone(), "raw_command is empty");
        write_result_record(output_dir, &response, started_at.elapsed()).await;
        return response;
    }

    if enforce_allowlist && request.pipeline.is_empty() {
        let response =
            CommandResponse::error(request.id.clone(), "pipeline is empty for auto-approve");
        write_result_record(output_dir, &response, started_at.elapsed()).await;
        return response;
    }

    if matches!(&request.mode, CommandMode::Argv) && request.pipeline.is_empty() {
        let response = CommandResponse::error(request.id.clone(), "pipeline is empty");
        write_result_record(output_dir, &response, started_at.elapsed()).await;
        return response;
    }

    if request.pipeline.is_empty() {
        let mode = &request.mode;
        tracing::warn!(
            id = %request.id,
            mode = ?mode,
            "empty pipeline, skipping whitelist validation"
        );
    } else {
        for stage in &request.pipeline {
            if let Err(message) = whitelist.validate_deny(stage) {
                let response = CommandResponse::error(request.id.clone(), message);
                write_result_record(output_dir, &response, started_at.elapsed()).await;
                return response;
            }
            if enforce_allowlist {
                if let Err(message) = whitelist.validate_allow(stage) {
                    let response = CommandResponse::error(request.id.clone(), message);
                    write_result_record(output_dir, &response, started_at.elapsed()).await;
                    return response;
                }
            }
        }
    }

    let timeout = Duration::from_secs(limits.timeout_secs);
    let max_bytes = usize::try_from(limits.max_output_bytes).unwrap_or(usize::MAX);
    let stdout_path = output_dir.join(format!("{}.stdout", request.id));
    let stderr_path = output_dir.join(format!("{}.stderr", request.id));

    let response = match tokio::time::timeout(
        timeout,
        execute_command(request, max_bytes, &stdout_path, &stderr_path),
    )
    .await
    {
        Ok(Ok(result)) => CommandResponse::completed(
            request.id.clone(),
            result.exit_code,
            result.stdout,
            result.stderr,
        ),
        Ok(Err(err)) => CommandResponse::error(request.id.clone(), err.to_string()),
        Err(_) => CommandResponse::error(request.id.clone(), "command timed out"),
    };

    write_result_record(output_dir, &response, started_at.elapsed()).await;
    response
}

struct ExecutionResult {
    exit_code: i32,
    stdout: Option<String>,
    stderr: Option<String>,
}

#[derive(Serialize)]
struct ResultRecord {
    id: String,
    status: CommandStatus,
    exit_code: Option<i32>,
    error: Option<String>,
    duration_ms: u128,
}

async fn execute_command(
    request: &CommandRequest,
    max_bytes: usize,
    stdout_path: &Path,
    stderr_path: &Path,
) -> anyhow::Result<ExecutionResult> {
    match request.mode {
        CommandMode::Shell => {
            execute_shell(
                &request.raw_command,
                request.cwd.as_deref(),
                request.env.as_ref(),
                max_bytes,
                stdout_path,
                stderr_path,
            )
            .await
        }
        CommandMode::Argv => {
            execute_pipeline(
                &request.pipeline,
                request.cwd.as_deref(),
                request.env.as_ref(),
                max_bytes,
                stdout_path,
                stderr_path,
            )
            .await
        }
    }
}

async fn execute_pipeline(
    pipeline: &[CommandStage],
    cwd: Option<&str>,
    env: Option<&BTreeMap<String, String>>,
    max_bytes: usize,
    stdout_path: &Path,
    stderr_path: &Path,
) -> anyhow::Result<ExecutionResult> {
    let mut children = Vec::with_capacity(pipeline.len());
    let stdout_file = File::create(stdout_path).await?;
    let stderr_file = File::create(stderr_path).await?;
    let stdout_writer = Arc::new(Mutex::new(stdout_file));
    let stderr_writer = Arc::new(Mutex::new(stderr_file));

    for (index, stage) in pipeline.iter().enumerate() {
        let command = stage
            .command()
            .ok_or_else(|| anyhow::anyhow!("empty command"))?;
        let resolved = resolve_command_path(command);
        let mut cmd = Command::new(&resolved);
        cmd.args(stage.argv.iter().skip(1));
        if let Some(cwd) = cwd {
            cmd.current_dir(cwd);
        }
        if let Some(env) = env {
            cmd.envs(env);
        }
        if index > 0 {
            cmd.stdin(Stdio::piped());
        } else {
            cmd.stdin(Stdio::null());
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.kill_on_drop(true);
        let child = cmd
            .spawn()
            .with_context(|| format!("spawn {command} ({resolved})"))?;
        children.push(child);
    }

    let mut pipe_tasks = Vec::new();
    for index in 0..children.len().saturating_sub(1) {
        let mut stdout = children[index].stdout.take().context("missing stdout")?;
        let mut stdin = children[index + 1].stdin.take().context("missing stdin")?;
        pipe_tasks.push(tokio::spawn(async move {
            let _ = tokio::io::copy(&mut stdout, &mut stdin).await;
        }));
    }

    let mut stderr_tasks = Vec::new();
    for child in &mut children {
        if let Some(stderr) = child.stderr.take() {
            let writer = Arc::clone(&stderr_writer);
            stderr_tasks.push(tokio::spawn(read_stream_capture(
                stderr,
                max_bytes,
                Some(writer),
            )));
        }
    }

    let stdout_task = {
        let last = children
            .last_mut()
            .context("missing last command")?
            .stdout
            .take()
            .context("missing stdout")?;
        let writer = Arc::clone(&stdout_writer);
        tokio::spawn(read_stream_capture(last, max_bytes, Some(writer)))
    };

    for task in pipe_tasks {
        let _ = task.await;
    }

    let mut exit_code = 0;
    for child in &mut children {
        let status = child.wait().await.context("wait on child")?;
        exit_code = status.code().unwrap_or(1);
    }

    let (stdout_bytes, stdout_truncated) = stdout_task
        .await
        .context("stdout task join")?
        .context("stdout read")?;
    let mut stderr = String::new();
    for task in stderr_tasks {
        let (bytes, truncated) = task
            .await
            .context("stderr task join")?
            .context("stderr read")?;
        if !bytes.is_empty() {
            if !stderr.is_empty() {
                stderr.push('\n');
            }
            stderr.push_str(&String::from_utf8_lossy(&bytes));
            if truncated {
                stderr.push_str("\n[output truncated]");
            }
        }
    }

    let stdout = if stdout_bytes.is_empty() {
        None
    } else {
        let mut out = String::from_utf8_lossy(&stdout_bytes).to_string();
        if stdout_truncated {
            out.push_str("\n[output truncated]");
        }
        Some(out)
    };

    let stderr = if stderr.is_empty() {
        None
    } else {
        Some(stderr)
    };

    Ok(ExecutionResult {
        exit_code,
        stdout,
        stderr,
    })
}

async fn execute_shell(
    raw_command: &str,
    cwd: Option<&str>,
    env: Option<&BTreeMap<String, String>>,
    max_bytes: usize,
    stdout_path: &Path,
    stderr_path: &Path,
) -> anyhow::Result<ExecutionResult> {
    let stdout_file = File::create(stdout_path).await?;
    let stderr_file = File::create(stderr_path).await?;
    let stdout_writer = Arc::new(Mutex::new(stdout_file));
    let stderr_writer = Arc::new(Mutex::new(stderr_file));

    let mut cmd = Command::new("/bin/bash");
    cmd.arg("-lc").arg(raw_command);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    if let Some(env) = env {
        cmd.envs(env);
    }
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn /bin/bash -lc {raw_command}"))?;

    let stdout = child.stdout.take().context("missing stdout")?;
    let stderr = child.stderr.take().context("missing stderr")?;
    let stdout_task = tokio::spawn(read_stream_capture(stdout, max_bytes, Some(stdout_writer)));
    let stderr_task = tokio::spawn(read_stream_capture(stderr, max_bytes, Some(stderr_writer)));

    let status = child.wait().await.context("wait on child")?;
    let exit_code = status.code().unwrap_or(1);

    let (stdout_bytes, stdout_truncated) = stdout_task
        .await
        .context("stdout task join")?
        .context("stdout read")?;
    let (stderr_bytes, stderr_truncated) = stderr_task
        .await
        .context("stderr task join")?
        .context("stderr read")?;

    let stdout = if stdout_bytes.is_empty() {
        None
    } else {
        let mut out = String::from_utf8_lossy(&stdout_bytes).to_string();
        if stdout_truncated {
            out.push_str("\n[output truncated]");
        }
        Some(out)
    };
    let stderr = if stderr_bytes.is_empty() {
        None
    } else {
        let mut out = String::from_utf8_lossy(&stderr_bytes).to_string();
        if stderr_truncated {
            out.push_str("\n[output truncated]");
        }
        Some(out)
    };

    Ok(ExecutionResult {
        exit_code,
        stdout,
        stderr,
    })
}

fn resolve_command_path(command: &str) -> String {
    if command.contains('/') {
        return command.to_string();
    }

    let candidates = ["/usr/bin", "/bin", "/usr/sbin", "/sbin"];
    for dir in candidates {
        let path = Path::new(dir).join(command);
        if path.is_file() {
            return path.to_string_lossy().to_string();
        }
    }

    command.to_string()
}

async fn read_stream_capture<R: AsyncRead + Unpin>(
    mut reader: R,
    max_bytes: usize,
    writer: Option<Arc<Mutex<File>>>,
) -> io::Result<(Vec<u8>, bool)> {
    let mut buffer = Vec::new();
    let mut truncated = false;
    let mut chunk = [0u8; 4096];
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        if let Some(writer) = &writer {
            write_chunk(writer, &chunk[..n]).await?;
        }
        if buffer.len() < max_bytes {
            let remaining = max_bytes - buffer.len();
            let to_copy = remaining.min(n);
            buffer.extend_from_slice(&chunk[..to_copy]);
            if to_copy < n {
                truncated = true;
            }
        } else {
            truncated = true;
        }
    }
    Ok((buffer, truncated))
}

pub async fn write_result_record(
    output_dir: &Path,
    response: &CommandResponse,
    duration: Duration,
) {
    let record = ResultRecord {
        id: response.id.clone(),
        status: response.status.clone(),
        exit_code: response.exit_code,
        error: response.error.clone(),
        duration_ms: duration.as_millis(),
    };
    let path = output_dir.join(format!("{}.result.json", response.id));
    if let Ok(payload) = serde_json::to_vec_pretty(&record) {
        if let Err(err) = tokio::fs::write(path, payload).await {
            tracing::warn!(error = %err, "failed to write result record");
        }
    }
}

async fn write_chunk(writer: &Arc<Mutex<File>>, data: &[u8]) -> io::Result<()> {
    let mut file = writer.lock().await;
    file.write_all(data).await
}

#[cfg(test)]
mod tests {
    use super::resolve_command_path;

    #[test]
    fn resolve_keeps_explicit_path() {
        let path = "/usr/bin/ls";
        assert_eq!(resolve_command_path(path), path.to_string());
    }
}
