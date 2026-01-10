use std::collections::BTreeMap;
use std::process::Stdio;
use std::time::Duration;

use anyhow::Context;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use protocol::{CommandRequest, CommandResponse};
use system_utils::ssh::apply_askpass_env;

use crate::state::TargetSpec;

use super::policy::{LimitsConfig, Whitelist};
use super::process::{apply_process_group, terminate_child};
use super::stream::read_stream_capture;

pub(super) async fn execute_request(
    target: &TargetSpec,
    request: &CommandRequest,
    whitelist: &Whitelist,
    limits: &LimitsConfig,
    cancel: CancellationToken,
) -> CommandResponse {
    if cancel.is_cancelled() {
        return CommandResponse::cancelled(request.id.clone(), None, None, None);
    }

    if request.raw_command.trim().is_empty() {
        return CommandResponse::error(request.id.clone(), "raw_command is empty");
    }

    if request.pipeline.is_empty() {
        tracing::warn!(
            id = %request.id,
            "empty pipeline, skipping whitelist validation"
        );
    } else {
        for stage in &request.pipeline {
            if let Err(message) = whitelist.validate_deny(stage) {
                return CommandResponse::denied(request.id.clone(), message);
            }
        }
    }

    let max_timeout_ms = limits.timeout_secs.saturating_mul(1000);
    let requested_timeout_ms = request.timeout_ms.filter(|value| *value > 0);
    let timeout_ms = requested_timeout_ms
        .unwrap_or(max_timeout_ms)
        .min(max_timeout_ms);
    let timeout = Duration::from_millis(timeout_ms);

    let max_output_bytes = request
        .max_output_bytes
        .filter(|value| *value > 0)
        .unwrap_or(limits.max_output_bytes)
        .min(limits.max_output_bytes);
    let max_bytes = usize::try_from(max_output_bytes).unwrap_or(usize::MAX);

    let mut timed_out = false;
    let mut exec_fut = Box::pin(execute_ssh_command(
        target,
        request,
        max_bytes,
        cancel.clone(),
        target.tty,
    ));
    let outcome = tokio::select! {
        result = &mut exec_fut => result,
        _ = tokio::time::sleep(timeout) => {
            timed_out = true;
            cancel.cancel();
            exec_fut.await
        }
    };

    if timed_out {
        return CommandResponse::error(request.id.clone(), "command timed out");
    }

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

async fn execute_ssh_command(
    target: &TargetSpec,
    request: &CommandRequest,
    max_bytes: usize,
    cancel: CancellationToken,
    tty: bool,
) -> anyhow::Result<ExecutionOutcome> {
    let ssh = target
        .ssh
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing ssh target"))?;
    let remote_cmd = build_remote_command(target, request);
    let mut cmd = Command::new("ssh");
    if let Some(password) = target.ssh_password.as_deref() {
        apply_askpass_env(&mut cmd, password)?;
    }
    if tty {
        cmd.arg("-tt");
    } else {
        cmd.arg("-T");
    }
    apply_ssh_options(&mut cmd, target.ssh_password.is_some());
    cmd.args(&target.ssh_args);
    cmd.arg(ssh);
    cmd.arg(remote_cmd);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    apply_process_group(&mut cmd);
    let mut child = cmd.spawn().context("spawn ssh command")?;

    let stdout = child.stdout.take().context("missing stdout")?;
    let stderr = child.stderr.take().context("missing stderr")?;
    let stdout_task = tokio::spawn(read_stream_capture(stdout, max_bytes));
    let stderr_task = tokio::spawn(read_stream_capture(stderr, max_bytes));

    let mut cancelled = false;
    let status = tokio::select! {
        status = child.wait() => Some(status.context("wait on ssh")?),
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
        stderr_bytes,
        stderr_truncated,
        cancelled,
        tty,
    ))
}

fn build_remote_command(target: &TargetSpec, request: &CommandRequest) -> String {
    let mut env_pairs: BTreeMap<String, String> = BTreeMap::new();
    if let Some(locale) = target.terminal_locale.as_deref() {
        env_pairs.insert("LANG".to_string(), locale.to_string());
    }
    if let Some(env) = request.env.as_ref() {
        for (key, value) in env {
            env_pairs.insert(key.to_string(), value.to_string());
        }
    }

    let env_prefix = build_env_prefix(&env_pairs);
    let mut command = String::new();
    if let Some(cwd) = request
        .cwd
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        command.push_str("cd ");
        command.push_str(&shell_escape(cwd));
        command.push_str(" && ");
    }
    if !env_prefix.is_empty() {
        command.push_str(&env_prefix);
        command.push(' ');
    }
    command.push_str(request.raw_command.trim());
    format!("bash -lc {}", shell_escape(&command))
}

fn build_env_prefix(pairs: &BTreeMap<String, String>) -> String {
    let mut parts = Vec::new();
    for (key, value) in pairs {
        if key.trim().is_empty() {
            continue;
        }
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        parts.push(format!("{key}={}", shell_escape(value)));
    }
    parts.join(" ")
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

fn build_execution_outcome(
    exit_code: Option<i32>,
    stdout_bytes: Vec<u8>,
    stdout_truncated: bool,
    stderr_bytes: Vec<u8>,
    stderr_truncated: bool,
    cancelled: bool,
    tty: bool,
) -> ExecutionOutcome {
    let (stdout, stderr) = if tty {
        let merged = merge_pty_output(
            stdout_bytes,
            stdout_truncated,
            stderr_bytes,
            stderr_truncated,
        );
        (merged, None)
    } else {
        (
            format_output(&stdout_bytes, stdout_truncated),
            format_output(&stderr_bytes, stderr_truncated),
        )
    };
    let result = ExecutionResult {
        exit_code,
        stdout,
        stderr,
    };
    if cancelled {
        ExecutionOutcome::Cancelled(result)
    } else {
        ExecutionOutcome::Completed(result)
    }
}

fn merge_pty_output(
    stdout_bytes: Vec<u8>,
    stdout_truncated: bool,
    stderr_bytes: Vec<u8>,
    stderr_truncated: bool,
) -> Option<String> {
    if stdout_bytes.is_empty() && stderr_bytes.is_empty() {
        return None;
    }
    let mut merged = stdout_bytes;
    if !stderr_bytes.is_empty() {
        if !merged.is_empty() {
            merged.extend_from_slice(b"\n[stderr]\n");
        } else {
            merged.extend_from_slice(b"[stderr]\n");
        }
        merged.extend_from_slice(&stderr_bytes);
    }
    format_output(&merged, stdout_truncated || stderr_truncated)
}

fn format_output(bytes: &[u8], truncated: bool) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let mut out = String::from_utf8_lossy(bytes).to_string();
    if truncated {
        out.push_str("\n[output truncated]");
    }
    Some(out)
}

fn apply_ssh_options(cmd: &mut Command, has_password: bool) {
    cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
    cmd.arg("-o").arg("ConnectTimeout=10");
    if !has_password {
        cmd.arg("-o").arg("BatchMode=yes");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_escape_wraps_and_escapes() {
        assert_eq!(shell_escape("plain"), "'plain'");
        assert_eq!(shell_escape("has space"), "'has space'");
        assert_eq!(shell_escape("a'b"), "'a'\"'\"'b'");
    }

    #[test]
    fn build_remote_command_includes_env_and_cwd() {
        let target = TargetSpec {
            name: "dev".to_string(),
            desc: "dev".to_string(),
            hostname: None,
            ip: None,
            ssh: Some("dev@host".to_string()),
            ssh_args: Vec::new(),
            ssh_password: None,
            terminal_locale: Some("en_US.UTF-8".to_string()),
            tty: false,
            control_remote_addr: "127.0.0.1:19308".to_string(),
            control_local_bind: None,
            control_local_port: None,
            control_local_addr: None,
        };
        let request = CommandRequest {
            id: "req-1".to_string(),
            client: "client".to_string(),
            target: "dev".to_string(),
            intent: "intent".to_string(),
            mode: protocol::CommandMode::Shell,
            raw_command: "echo hello".to_string(),
            cwd: Some("/tmp/work dir".to_string()),
            env: Some(BTreeMap::from([("FOO".to_string(), "bar baz".to_string())])),
            timeout_ms: None,
            max_output_bytes: None,
            pipeline: Vec::new(),
        };
        let cmd = build_remote_command(&target, &request);
        assert!(cmd.starts_with("bash -lc "));
        assert!(cmd.contains("cd "));
        assert!(cmd.contains("/tmp/work dir"));
        assert!(cmd.contains("LANG="));
        assert!(cmd.contains("FOO="));
        assert!(cmd.contains("echo hello"));
    }

    #[test]
    fn build_env_prefix_skips_empty_keys_and_values() {
        let mut pairs = BTreeMap::new();
        pairs.insert("FOO".to_string(), "bar".to_string());
        pairs.insert("".to_string(), "skip".to_string());
        pairs.insert("EMPTY".to_string(), "".to_string());
        assert_eq!(build_env_prefix(&pairs), "FOO='bar'");
    }

    #[test]
    fn format_output_marks_truncation() {
        let out = format_output(b"hello", true).expect("output");
        assert!(out.contains("hello"));
        assert!(out.contains("[output truncated]"));
    }

    #[test]
    fn pty_merges_stderr_into_stdout() {
        let outcome = build_execution_outcome(
            Some(0),
            b"out".to_vec(),
            false,
            b"err".to_vec(),
            false,
            false,
            true,
        );
        match outcome {
            ExecutionOutcome::Completed(result) => {
                let stdout = result.stdout.expect("stdout");
                assert!(result.stderr.is_none());
                assert!(stdout.contains("out"));
                assert!(stdout.contains("[stderr]"));
                assert!(stdout.contains("err"));
            }
            _ => panic!("unexpected outcome"),
        }
    }
}
