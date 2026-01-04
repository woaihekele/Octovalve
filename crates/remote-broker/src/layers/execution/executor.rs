use crate::layers::execution::output::write_result_record;
use crate::layers::policy::config::LimitsConfig;
use crate::layers::policy::whitelist::Whitelist;
use anyhow::Context;
use protocol::{CommandMode, CommandRequest, CommandResponse, CommandStage};
use std::collections::BTreeMap;
use std::io;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

const CANCEL_GRACE: Duration = Duration::from_secs(2);

pub async fn execute_request(
    request: &CommandRequest,
    whitelist: &Whitelist,
    limits: &LimitsConfig,
    output_dir: &Path,
    cancel: CancellationToken,
) -> CommandResponse {
    let started_at = Instant::now();

    if cancel.is_cancelled() {
        let response = CommandResponse::cancelled(request.id.clone(), None, None, None);
        write_result_record(output_dir, &response, started_at.elapsed()).await;
        return response;
    }

    if matches!(&request.mode, CommandMode::Shell) && request.raw_command.trim().is_empty() {
        let response = CommandResponse::error(request.id.clone(), "raw_command is empty");
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
                let response = CommandResponse::denied(request.id.clone(), message);
                write_result_record(output_dir, &response, started_at.elapsed()).await;
                return response;
            }
        }
    }

    let timeout = Duration::from_secs(limits.timeout_secs);
    let max_bytes = usize::try_from(limits.max_output_bytes).unwrap_or(usize::MAX);
    let stdout_path = output_dir.join(format!("{}.stdout", request.id));
    let stderr_path = output_dir.join(format!("{}.stderr", request.id));

    let mut timed_out = false;
    let mut exec_fut = Box::pin(execute_command(
        request,
        max_bytes,
        &stdout_path,
        &stderr_path,
        cancel.clone(),
    ));
    let outcome = tokio::select! {
        result = &mut exec_fut => result,
        _ = tokio::time::sleep(timeout) => {
            timed_out = true;
            cancel.cancel();
            exec_fut.await
        }
    };

    let response = if timed_out {
        CommandResponse::error(request.id.clone(), "command timed out")
    } else {
        match outcome {
            Ok(ExecutionOutcome::Completed(result)) => CommandResponse::completed(
                request.id.clone(),
                result.exit_code.unwrap_or(1),
                result.stdout,
                result.stderr,
            ),
            Ok(ExecutionOutcome::Cancelled(result)) => CommandResponse::cancelled(
                request.id.clone(),
                result.exit_code,
                result.stdout,
                result.stderr,
            ),
            Err(err) => CommandResponse::error(request.id.clone(), err.to_string()),
        }
    };

    write_result_record(output_dir, &response, started_at.elapsed()).await;
    response
}

struct ExecutionResult {
    exit_code: Option<i32>,
    stdout: Option<String>,
    stderr: Option<String>,
}

enum ExecutionOutcome {
    Completed(ExecutionResult),
    Cancelled(ExecutionResult),
}

async fn execute_command(
    request: &CommandRequest,
    max_bytes: usize,
    stdout_path: &Path,
    stderr_path: &Path,
    cancel: CancellationToken,
) -> anyhow::Result<ExecutionOutcome> {
    match request.mode {
        CommandMode::Shell => {
            execute_shell(
                &request.raw_command,
                request.cwd.as_deref(),
                request.env.as_ref(),
                max_bytes,
                stdout_path,
                stderr_path,
                cancel,
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
                cancel,
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
    cancel: CancellationToken,
) -> anyhow::Result<ExecutionOutcome> {
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
        apply_process_group(&mut cmd);
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

    let mut cancelled = false;
    let exit_code = tokio::select! {
        result = async {
            let mut exit_code = None;
            for child in &mut children {
                let status = child.wait().await.context("wait on child")?;
                exit_code = status.code();
            }
            Ok::<Option<i32>, anyhow::Error>(exit_code)
        } => result?,
        _ = cancel.cancelled() => {
            cancelled = true;
            terminate_children(&mut children).await;
            None
        }
    };

    for task in pipe_tasks {
        let _ = task.await;
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

    let result = ExecutionResult {
        exit_code,
        stdout,
        stderr,
    };
    Ok(if cancelled {
        ExecutionOutcome::Cancelled(result)
    } else {
        ExecutionOutcome::Completed(result)
    })
}

async fn execute_shell(
    raw_command: &str,
    cwd: Option<&str>,
    env: Option<&BTreeMap<String, String>>,
    max_bytes: usize,
    stdout_path: &Path,
    stderr_path: &Path,
    cancel: CancellationToken,
) -> anyhow::Result<ExecutionOutcome> {
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
    apply_process_group(&mut cmd);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn /bin/bash -lc {raw_command}"))?;

    let stdout = child.stdout.take().context("missing stdout")?;
    let stderr = child.stderr.take().context("missing stderr")?;
    let stdout_task = tokio::spawn(read_stream_capture(stdout, max_bytes, Some(stdout_writer)));
    let stderr_task = tokio::spawn(read_stream_capture(stderr, max_bytes, Some(stderr_writer)));

    let mut cancelled = false;
    let status = tokio::select! {
        status = child.wait() => Some(status.context("wait on child")?),
        _ = cancel.cancelled() => {
            cancelled = true;
            terminate_child(&mut child).await
        }
    };
    let exit_code = status.and_then(|status| status.code());

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

    let result = ExecutionResult {
        exit_code,
        stdout,
        stderr,
    };
    Ok(if cancelled {
        ExecutionOutcome::Cancelled(result)
    } else {
        ExecutionOutcome::Completed(result)
    })
}

#[cfg(unix)]
fn apply_process_group(cmd: &mut Command) {
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn apply_process_group(_cmd: &mut Command) {}

#[cfg(unix)]
fn signal_child(child: &mut tokio::process::Child, signal: i32) {
    if let Some(pid) = child.id() {
        unsafe {
            libc::kill(-(pid as i32), signal);
        }
    }
}

#[cfg(not(unix))]
fn signal_child(_child: &mut tokio::process::Child, _signal: i32) {}

async fn terminate_child(child: &mut tokio::process::Child) -> Option<std::process::ExitStatus> {
    signal_child(child, libc::SIGINT);
    match tokio::time::timeout(CANCEL_GRACE, child.wait()).await {
        Ok(status) => return status.ok(),
        Err(_) => {
            signal_child(child, libc::SIGKILL);
            let _ = child.kill().await;
            match tokio::time::timeout(CANCEL_GRACE, child.wait()).await {
                Ok(status) => status.ok(),
                Err(_) => None,
            }
        }
    }
}

async fn terminate_children(children: &mut [tokio::process::Child]) {
    for child in children.iter_mut() {
        signal_child(child, libc::SIGINT);
    }
    for child in children.iter_mut() {
        if tokio::time::timeout(CANCEL_GRACE, child.wait())
            .await
            .is_ok()
        {
            continue;
        }
        signal_child(child, libc::SIGKILL);
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
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
