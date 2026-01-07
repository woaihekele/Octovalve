use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::Context;
use tokio::fs::File;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::output::build_execution_outcome;
use super::process::{apply_process_group, terminate_child};
use super::stream::read_stream_capture;
use super::ExecutionOutcome;

pub(super) async fn execute_shell(
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

    Ok(build_execution_outcome(
        exit_code,
        stdout_bytes,
        stdout_truncated,
        vec![(stderr_bytes, stderr_truncated)],
        cancelled,
    ))
}
