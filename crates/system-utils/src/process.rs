use std::process::{Output, Stdio};

use anyhow::Context;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

pub async fn run_command_with_timeout(
    cmd: &mut Command,
    command_timeout: Duration,
    label: &str,
) -> anyhow::Result<Output> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|err| anyhow::anyhow!(err))?;
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let status = match timeout(command_timeout, child.wait()).await {
        Ok(result) => result.with_context(|| format!("{label} failed"))?,
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            anyhow::bail!(
                "{label} timed out after {}s",
                command_timeout.as_secs()
            )
        }
    };
    let mut stdout = Vec::new();
    if let Some(mut pipe) = stdout_pipe.take() {
        let _ = pipe.read_to_end(&mut stdout).await;
    }
    let mut stderr = Vec::new();
    if let Some(mut pipe) = stderr_pipe.take() {
        let _ = pipe.read_to_end(&mut stderr).await;
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}
