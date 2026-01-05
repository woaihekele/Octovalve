use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::Context;
use protocol::CommandStage;
use tokio::fs::File;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::process::{apply_process_group, terminate_children};
use super::stream::read_stream_capture;
use super::output::build_execution_outcome;
use super::ExecutionOutcome;

pub(super) async fn execute_pipeline(
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
    let mut stderr_chunks = Vec::with_capacity(stderr_tasks.len());
    for task in stderr_tasks {
        let (bytes, truncated) = task
            .await
            .context("stderr task join")?
            .context("stderr read")?;
        stderr_chunks.push((bytes, truncated));
    }

    Ok(build_execution_outcome(
        exit_code,
        stdout_bytes,
        stdout_truncated,
        stderr_chunks,
        cancelled,
    ))
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

#[cfg(test)]
mod tests {
    use super::resolve_command_path;

    #[test]
    fn resolve_keeps_explicit_path() {
        let path = "/usr/bin/ls";
        assert_eq!(resolve_command_path(path), path.to_string());
    }
}
