use crate::config::LimitsConfig;
use crate::whitelist::Whitelist;
use anyhow::Context;
use protocol::{CommandRequest, CommandResponse, CommandStage};
use std::collections::BTreeMap;
use std::io;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

pub async fn execute_request(
    request: &CommandRequest,
    whitelist: &Whitelist,
    limits: &LimitsConfig,
) -> CommandResponse {
    if request.pipeline.is_empty() {
        return CommandResponse::error(request.id.clone(), "empty pipeline");
    }

    for stage in &request.pipeline {
        if let Err(message) = whitelist.validate(stage) {
            return CommandResponse::error(request.id.clone(), message);
        }
    }

    let timeout = Duration::from_secs(limits.timeout_secs);
    let max_bytes = usize::try_from(limits.max_output_bytes).unwrap_or(usize::MAX);

    match tokio::time::timeout(
        timeout,
        execute_pipeline(
            &request.pipeline,
            request.cwd.as_deref(),
            request.env.as_ref(),
            max_bytes,
        ),
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
    }
}

struct ExecutionResult {
    exit_code: i32,
    stdout: Option<String>,
    stderr: Option<String>,
}

async fn execute_pipeline(
    pipeline: &[CommandStage],
    cwd: Option<&str>,
    env: Option<&BTreeMap<String, String>>,
    max_bytes: usize,
) -> anyhow::Result<ExecutionResult> {
    let mut children = Vec::with_capacity(pipeline.len());

    for (index, stage) in pipeline.iter().enumerate() {
        let command = stage
            .command()
            .ok_or_else(|| anyhow::anyhow!("empty command"))?;
        let mut cmd = Command::new(command);
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
        let child = cmd.spawn().with_context(|| format!("spawn {command}"))?;
        children.push(child);
    }

    let mut pipe_tasks = Vec::new();
    for index in 0..children.len().saturating_sub(1) {
        let mut stdout = children[index]
            .stdout
            .take()
            .context("missing stdout")?;
        let mut stdin = children[index + 1]
            .stdin
            .take()
            .context("missing stdin")?;
        pipe_tasks.push(tokio::spawn(async move {
            let _ = tokio::io::copy(&mut stdout, &mut stdin).await;
        }));
    }

    let mut stderr_tasks = Vec::new();
    for child in &mut children {
        if let Some(stderr) = child.stderr.take() {
            stderr_tasks.push(tokio::spawn(read_stream_limited(stderr, max_bytes)));
        }
    }

    let stdout_task = {
        let last = children
            .last_mut()
            .context("missing last command")?
            .stdout
            .take()
            .context("missing stdout")?;
        tokio::spawn(read_stream_limited(last, max_bytes))
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

    let stderr = if stderr.is_empty() { None } else { Some(stderr) };

    Ok(ExecutionResult {
        exit_code,
        stdout,
        stderr,
    })
}

async fn read_stream_limited<R: AsyncRead + Unpin>(
    mut reader: R,
    max_bytes: usize,
) -> io::Result<(Vec<u8>, bool)> {
    let mut buffer = Vec::new();
    let mut truncated = false;
    let mut chunk = [0u8; 4096];
    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            break;
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
