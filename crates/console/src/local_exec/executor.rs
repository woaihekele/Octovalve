use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::Context;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use protocol::{CommandRequest, CommandResponse};
use system_utils::path::expand_tilde;
use system_utils::ssh::apply_askpass_env;
use tracing::warn;

use crate::state::TargetSpec;

use super::policy::{LimitsConfig, Whitelist};
use super::process::{apply_process_group, terminate_child};
use super::stream::read_stream_capture;

const DEFAULT_SSH_CONTROL_DIR: &str = "~/.octovalve/ssh-control";
const DEFAULT_SSH_CONTROL_PERSIST: &str = "60s";

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
    let locale = resolve_exec_locale(target);
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
    if let Some(control_path) = resolve_control_path(target) {
        apply_control_master(&mut cmd, &control_path);
    }
    apply_locale_env(&mut cmd, locale.as_deref());
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
    if let Some(env) = request.env.as_ref() {
        for (key, value) in env {
            env_pairs.insert(key.to_string(), value.to_string());
        }
    }

    let mut shell_prefix = String::new();
    if let Some(locale) = resolve_exec_locale(target) {
        let escaped = shell_escape(&locale);
        shell_prefix.push_str(&format!(
            "LANG={escaped} LC_CTYPE={escaped} LC_ALL={escaped} "
        ));
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
    format!(
        "{shell_prefix}bash --noprofile -lc {}",
        shell_escape(&command)
    )
}

fn resolve_exec_locale(target: &TargetSpec) -> Option<String> {
    let target_locale = target
        .terminal_locale
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    if target_locale.is_some() {
        return target_locale;
    }
    if let Some(locale) = env_locale("OCTOVALVE_TERMINAL_LOCALE") {
        return Some(locale);
    }
    if let Some(locale) = env_language_locale("OCTOVALVE_APP_LANGUAGE") {
        return Some(locale);
    }
    if let Some(locale) = env_language_locale("LANG") {
        return Some(locale);
    }
    Some("en_US.utf8".to_string())
}

fn env_locale(key: &str) -> Option<String> {
    let value = std::env::var(key).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn env_language_locale(key: &str) -> Option<String> {
    let value = std::env::var(key).ok()?;
    let trimmed = value.trim().to_lowercase();
    if trimmed.starts_with("zh") {
        return Some("zh_CN.utf8".to_string());
    }
    if trimmed.starts_with("en") {
        return Some("en_US.utf8".to_string());
    }
    None
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

fn resolve_control_path(target: &TargetSpec) -> Option<PathBuf> {
    let ssh = target.ssh.as_deref()?.trim();
    if ssh.is_empty() {
        return None;
    }
    let control_dir = resolve_control_dir()?;
    Some(control_path_for_target(&control_dir, target, ssh))
}

fn resolve_control_dir() -> Option<PathBuf> {
    let value = std::env::var("OCTOVALVE_SSH_CONTROL_DIR")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_SSH_CONTROL_DIR.to_string());
    let dir = expand_tilde(&value);
    if let Err(err) = std::fs::create_dir_all(&dir) {
        warn!(error = %err, path = %dir.display(), "failed to create ssh control dir");
        return None;
    }
    Some(dir)
}

fn control_path_for_target(control_dir: &Path, target: &TargetSpec, ssh: &str) -> PathBuf {
    let fingerprint = format!("{}|{}", target.name, ssh);
    let digest = md5::compute(fingerprint.as_bytes());
    let filename = format!("cm-{:x}", digest);
    control_dir.join(filename)
}

fn control_master_args(control_path: &Path) -> [String; 6] {
    [
        "-o".to_string(),
        "ControlMaster=auto".to_string(),
        "-o".to_string(),
        format!("ControlPersist={}", DEFAULT_SSH_CONTROL_PERSIST),
        "-o".to_string(),
        format!("ControlPath={}", control_path.display()),
    ]
}

fn apply_control_master(cmd: &mut Command, control_path: &Path) {
    for arg in control_master_args(control_path) {
        cmd.arg(arg);
    }
}

fn apply_locale_env(cmd: &mut Command, locale: Option<&str>) {
    let Some(locale) = locale else {
        return;
    };
    cmd.env("LANG", locale);
    cmd.env("LC_CTYPE", locale);
    cmd.env("LC_ALL", locale);
    cmd.arg("-o").arg("SendEnv=LANG,LC_CTYPE,LC_ALL");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_target() -> TargetSpec {
        TargetSpec {
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
        }
    }

    fn sample_request() -> CommandRequest {
        CommandRequest {
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
        }
    }

    #[test]
    fn shell_escape_wraps_and_escapes() {
        assert_eq!(shell_escape("plain"), "'plain'");
        assert_eq!(shell_escape("has space"), "'has space'");
        assert_eq!(shell_escape("a'b"), "'a'\"'\"'b'");
    }

    #[test]
    fn build_remote_command_includes_env_and_cwd() {
        let target = sample_target();
        let request = sample_request();
        let cmd = build_remote_command(&target, &request);
        assert!(cmd.contains("bash --noprofile -lc "));
        assert!(cmd.contains("cd "));
        assert!(cmd.contains("/tmp/work dir"));
        assert!(cmd.contains("LANG="));
        assert!(cmd.contains("LC_CTYPE="));
        assert!(cmd.contains("LC_ALL="));
        assert!(cmd.contains("FOO="));
        assert!(cmd.contains("echo hello"));
    }

    #[test]
    fn control_path_is_stable_per_target() {
        let target = sample_target();
        let dir = PathBuf::from("/tmp/ssh-control");
        let ssh = target.ssh.as_deref().unwrap_or_default();
        let first = control_path_for_target(&dir, &target, ssh);
        let second = control_path_for_target(&dir, &target, ssh);
        assert_eq!(first, second);
    }

    #[test]
    fn control_master_args_include_path() {
        let path = PathBuf::from("/tmp/ssh-control/cm-test");
        let args = control_master_args(&path);
        assert!(args.iter().any(|arg| arg == "ControlMaster=auto"));
        assert!(args.iter().any(|arg| arg == "ControlPersist=60s"));
        assert!(args
            .iter()
            .any(|arg| arg == "ControlPath=/tmp/ssh-control/cm-test"));
    }

    #[test]
    fn build_remote_command_disables_profiles() {
        let target = sample_target();
        let request = sample_request();
        let cmd = build_remote_command(&target, &request);
        assert!(cmd.contains("bash --noprofile -lc "));
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

    #[test]
    fn resolve_exec_locale_prefers_target() {
        let target = TargetSpec {
            name: "dev".to_string(),
            desc: "dev".to_string(),
            hostname: None,
            ip: None,
            ssh: None,
            ssh_args: Vec::new(),
            ssh_password: None,
            terminal_locale: Some("en_US.utf8".to_string()),
            tty: false,
            control_remote_addr: "127.0.0.1:19308".to_string(),
            control_local_bind: None,
            control_local_port: None,
            control_local_addr: None,
        };
        let backup = std::env::var("OCTOVALVE_TERMINAL_LOCALE").ok();
        std::env::set_var("OCTOVALVE_TERMINAL_LOCALE", "zh_CN.utf8");
        let resolved = resolve_exec_locale(&target);
        if let Some(value) = backup {
            std::env::set_var("OCTOVALVE_TERMINAL_LOCALE", value);
        } else {
            std::env::remove_var("OCTOVALVE_TERMINAL_LOCALE");
        }
        assert_eq!(resolved.as_deref(), Some("en_US.utf8"));
    }

    #[test]
    fn resolve_exec_locale_uses_env_fallback() {
        let target = TargetSpec {
            name: "dev".to_string(),
            desc: "dev".to_string(),
            hostname: None,
            ip: None,
            ssh: None,
            ssh_args: Vec::new(),
            ssh_password: None,
            terminal_locale: None,
            tty: false,
            control_remote_addr: "127.0.0.1:19308".to_string(),
            control_local_bind: None,
            control_local_port: None,
            control_local_addr: None,
        };
        let backup = std::env::var("OCTOVALVE_TERMINAL_LOCALE").ok();
        std::env::set_var("OCTOVALVE_TERMINAL_LOCALE", "zh_CN.utf8");
        let resolved = resolve_exec_locale(&target);
        if let Some(value) = backup {
            std::env::set_var("OCTOVALVE_TERMINAL_LOCALE", value);
        } else {
            std::env::remove_var("OCTOVALVE_TERMINAL_LOCALE");
        }
        assert_eq!(resolved.as_deref(), Some("zh_CN.utf8"));
    }

    #[test]
    fn resolve_exec_locale_uses_app_language() {
        let target = TargetSpec {
            name: "dev".to_string(),
            desc: "dev".to_string(),
            hostname: None,
            ip: None,
            ssh: None,
            ssh_args: Vec::new(),
            ssh_password: None,
            terminal_locale: None,
            tty: false,
            control_remote_addr: "127.0.0.1:19308".to_string(),
            control_local_bind: None,
            control_local_port: None,
            control_local_addr: None,
        };
        let backup = std::env::var("OCTOVALVE_APP_LANGUAGE").ok();
        std::env::set_var("OCTOVALVE_APP_LANGUAGE", "zh-CN");
        let resolved = resolve_exec_locale(&target);
        if let Some(value) = backup {
            std::env::set_var("OCTOVALVE_APP_LANGUAGE", value);
        } else {
            std::env::remove_var("OCTOVALVE_APP_LANGUAGE");
        }
        assert_eq!(resolved.as_deref(), Some("zh_CN.utf8"));
    }
}
