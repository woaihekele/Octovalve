use std::collections::BTreeMap;
use std::future::Future;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use protocol::{CommandRequest, CommandResponse};
use system_utils::path::expand_tilde;
use system_utils::ssh::apply_askpass_env;
use tracing::warn;

use crate::shell_utils::{apply_ssh_options, build_env_prefix, env_language_locale, env_locale, shell_escape};
use crate::state::TargetSpec;

use super::policy::{LimitsConfig, Whitelist};
use super::process::{apply_process_group, terminate_child};
use super::stream::read_stream_capture;

const DEFAULT_SSH_CONTROL_DIR: &str = "~/.octovalve/ssh-control";
const DEFAULT_SSH_CONTROL_PERSIST: &str = "60s";
const DEFAULT_PTY_COLS: u16 = 120;
const DEFAULT_PTY_ROWS: u16 = 24;
const DEFAULT_PTY_TERM: &str = "xterm-256color";
const PTY_CANCEL_GRACE_SECS: u64 = 2;
const PTY_MARKER_BEGIN_PREFIX: &str = "__OCTOVALVE_BEGIN__";
const PTY_MARKER_END_PREFIX: &str = "__OCTOVALVE_END__";

pub(super) async fn execute_request(
    target: &TargetSpec,
    request: &CommandRequest,
    whitelist: &Whitelist,
    limits: &LimitsConfig,
    pty_manager: Option<Arc<PtySessionManager>>,
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
    let mut exec_fut: std::pin::Pin<
        Box<dyn Future<Output = anyhow::Result<ExecutionOutcome>> + Send>,
    > = if let Some(manager) = pty_manager {
        Box::pin(execute_pty_command(
            manager,
            request,
            max_bytes,
            cancel.clone(),
        ))
    } else {
        Box::pin(execute_ssh_command(
            target,
            request,
            max_bytes,
            cancel.clone(),
            target.tty,
        ))
    };
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

pub(super) struct PtySessionManager {
    target: TargetSpec,
    state: Mutex<PtySessionState>,
}

struct PtySessionState {
    session: Option<PtySession>,
}

struct PtySession {
    writer: Box<dyn Write + Send>,
    reader_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    buffer: Vec<u8>,
    next_id: u64,
    child: Box<dyn portable_pty::Child + Send>,
}

struct PtyCommandOutcome {
    exit_code: Option<i32>,
    output: Vec<u8>,
    truncated: bool,
    cancelled: bool,
    needs_reset: bool,
}

impl PtySessionManager {
    pub(super) fn new(target: TargetSpec) -> Self {
        Self {
            target,
            state: Mutex::new(PtySessionState { session: None }),
        }
    }

    async fn run_command(
        &self,
        request: &CommandRequest,
        max_bytes: usize,
        cancel: CancellationToken,
    ) -> anyhow::Result<PtyCommandOutcome> {
        let mut state = self.state.lock().await;
        if state.session.is_none() {
            state.session = Some(PtySession::spawn(&self.target)?);
        }
        let result = match state.session.as_mut() {
            Some(session) => session.run_command(request, max_bytes, cancel).await,
            None => Err(anyhow::anyhow!("pty session not available")),
        };
        match result {
            Ok(outcome) => {
                if outcome.needs_reset {
                    state.session = None;
                }
                Ok(outcome)
            }
            Err(err) => {
                state.session = None;
                Err(err)
            }
        }
    }
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

async fn execute_pty_command(
    manager: Arc<PtySessionManager>,
    request: &CommandRequest,
    max_bytes: usize,
    cancel: CancellationToken,
) -> anyhow::Result<ExecutionOutcome> {
    let outcome = manager.run_command(request, max_bytes, cancel).await?;
    Ok(build_execution_outcome(
        outcome.exit_code,
        outcome.output,
        outcome.truncated,
        Vec::new(),
        false,
        outcome.cancelled,
        true,
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

fn build_session_command(request: &CommandRequest) -> String {
    let mut env_pairs: BTreeMap<String, String> = BTreeMap::new();
    if let Some(env) = request.env.as_ref() {
        for (key, value) in env {
            env_pairs.insert(key.to_string(), value.to_string());
        }
    }
    let env_prefix = build_env_prefix(&env_pairs);
    let mut command = String::new();
    if !env_prefix.is_empty() {
        command.push_str(&env_prefix);
        command.push(' ');
    }
    command.push_str(request.raw_command.trim());
    if let Some(cwd) = request
        .cwd
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return format!("(cd {} && {})", shell_escape(cwd), command);
    }
    command
}

fn build_pty_command(id: u64, request: &CommandRequest) -> String {
    let begin_marker = format!("{PTY_MARKER_BEGIN_PREFIX}{id}__");
    let end_prefix = format!("{PTY_MARKER_END_PREFIX}{id}__");
    let command = build_session_command(request);
    format!(
        "printf '%s\\n' '{begin_marker}'; {command}; status=$?; printf '%s%d__\\n' '{end_prefix}' \"$status\""
    )
}

fn pty_begin_marker(id: u64) -> Vec<u8> {
    format!("{PTY_MARKER_BEGIN_PREFIX}{id}__\n").into_bytes()
}

fn pty_end_prefix(id: u64) -> Vec<u8> {
    format!("{PTY_MARKER_END_PREFIX}{id}__").into_bytes()
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

impl PtySession {
    fn spawn(target: &TargetSpec) -> anyhow::Result<Self> {
        let ssh = target
            .ssh
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing ssh target"))?;
        let pair = native_pty_system().openpty(PtySize {
            rows: DEFAULT_PTY_ROWS,
            cols: DEFAULT_PTY_COLS,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let mut cmd = CommandBuilder::new("ssh");
        if let Some(password) = target.ssh_password.as_deref() {
            for (key, value) in system_utils::ssh::askpass_env(password)? {
                cmd.env(key, value);
            }
        }
        cmd.arg("-tt");
        apply_ssh_options(&mut cmd, target.ssh_password.is_some());
        if let Some(control_path) = resolve_control_path(target) {
            apply_control_master_builder(&mut cmd, &control_path);
        }
        apply_locale_env_builder(&mut cmd, resolve_exec_locale(target).as_deref());
        for arg in &target.ssh_args {
            cmd.arg(arg);
        }
        cmd.arg(ssh);
        cmd.arg("bash");
        cmd.arg("--noprofile");
        cmd.arg("--norc");
        cmd.env("TERM", DEFAULT_PTY_TERM);
        let child = pair
            .slave
            .spawn_command(cmd)
            .context("spawn pty ssh command")?;
        let reader = pair.master.try_clone_reader().context("clone pty reader")?;
        let (reader_tx, reader_rx) = mpsc::unbounded_channel();
        std::thread::spawn(move || read_pty_loop(reader, reader_tx));
        let writer = pair.master.take_writer().context("take pty writer")?;
        let mut session = Self {
            writer,
            reader_rx,
            buffer: Vec::new(),
            next_id: 1,
            child,
        };
        session.initialize()?;
        Ok(session)
    }

    fn initialize(&mut self) -> anyhow::Result<()> {
        self.write_line("export PS1=")?;
        self.write_line("stty -echo")?;
        Ok(())
    }

    async fn run_command(
        &mut self,
        request: &CommandRequest,
        max_bytes: usize,
        cancel: CancellationToken,
    ) -> anyhow::Result<PtyCommandOutcome> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let command = build_pty_command(id, request);
        let begin_marker = pty_begin_marker(id);
        let end_prefix = pty_end_prefix(id);
        self.write_line(&command)?;

        let mut output = Vec::new();
        let mut truncated = false;
        let mut seen_begin = false;
        let mut cancelled = false;
        let mut cancel_deadline: Option<std::time::Instant> = None;
        loop {
            if let Some(exit_code) = extract_pty_output(
                &mut self.buffer,
                &begin_marker,
                &end_prefix,
                &mut output,
                max_bytes,
                &mut truncated,
                &mut seen_begin,
            ) {
                return Ok(PtyCommandOutcome {
                    exit_code,
                    output,
                    truncated,
                    cancelled,
                    needs_reset: false,
                });
            }

            if cancelled {
                let deadline = cancel_deadline.unwrap_or_else(|| {
                    std::time::Instant::now() + Duration::from_secs(PTY_CANCEL_GRACE_SECS)
                });
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                match tokio::time::timeout(remaining, self.reader_rx.recv()).await {
                    Ok(Some(chunk)) => {
                        self.buffer.extend(chunk);
                    }
                    Ok(None) => {
                        return Ok(PtyCommandOutcome {
                            exit_code: None,
                            output,
                            truncated,
                            cancelled: true,
                            needs_reset: true,
                        });
                    }
                    Err(_) => {
                        return Ok(PtyCommandOutcome {
                            exit_code: None,
                            output,
                            truncated,
                            cancelled: true,
                            needs_reset: true,
                        });
                    }
                }
            } else {
                tokio::select! {
                    chunk = self.reader_rx.recv() => {
                        match chunk {
                            Some(chunk) => self.buffer.extend(chunk),
                            None => {
                                return Ok(PtyCommandOutcome {
                                    exit_code: None,
                                    output,
                                    truncated,
                                    cancelled,
                                    needs_reset: true,
                                });
                            }
                        }
                    }
                    _ = cancel.cancelled() => {
                        cancelled = true;
                        cancel_deadline = Some(std::time::Instant::now() + Duration::from_secs(PTY_CANCEL_GRACE_SECS));
                        if let Err(err) = send_ctrl_c(&mut self.writer) {
                            tracing::warn!(error = %err, "failed to send pty interrupt");
                        }
                    }
                }
            }
        }
    }

    fn write_line(&mut self, line: &str) -> anyhow::Result<()> {
        self.writer
            .write_all(line.as_bytes())
            .context("write pty command")?;
        self.writer.write_all(b"\n").context("write pty newline")?;
        self.writer.flush().context("flush pty command")?;
        Ok(())
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

fn apply_control_master_builder(cmd: &mut CommandBuilder, control_path: &Path) {
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

fn apply_locale_env_builder(cmd: &mut CommandBuilder, locale: Option<&str>) {
    let Some(locale) = locale else {
        return;
    };
    cmd.env("LANG", locale);
    cmd.env("LC_CTYPE", locale);
    cmd.env("LC_ALL", locale);
    cmd.arg("-o");
    cmd.arg("SendEnv=LANG,LC_CTYPE,LC_ALL");
}

fn read_pty_loop(mut reader: Box<dyn Read + Send>, tx: mpsc::UnboundedSender<Vec<u8>>) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(size) => {
                if tx.send(buf[..size].to_vec()).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

fn send_ctrl_c(writer: &mut dyn Write) -> anyhow::Result<()> {
    writer.write_all(b"\x03").context("write ctrl-c")?;
    writer.flush().context("flush ctrl-c")?;
    Ok(())
}

fn extract_pty_output(
    buffer: &mut Vec<u8>,
    begin_marker: &[u8],
    end_prefix: &[u8],
    output: &mut Vec<u8>,
    max_bytes: usize,
    truncated: &mut bool,
    seen_begin: &mut bool,
) -> Option<Option<i32>> {
    if !*seen_begin {
        if let Some(pos) = find_subsequence(buffer, begin_marker) {
            buffer.drain(..pos + begin_marker.len());
            *seen_begin = true;
        } else {
            let keep = begin_marker.len().saturating_sub(1);
            if buffer.len() > keep {
                buffer.drain(..buffer.len() - keep);
            }
            return None;
        }
    }

    if let Some(pos) = find_subsequence(buffer, end_prefix) {
        append_output(output, &buffer[..pos], max_bytes, truncated);
        let tail = &buffer[pos + end_prefix.len()..];
        if let Some(end_pos) = find_subsequence(tail, b"__") {
            let code_bytes = &tail[..end_pos];
            let exit_code = parse_exit_code(code_bytes);
            let mut drain_len = pos + end_prefix.len() + end_pos + 2;
            if buffer.get(drain_len) == Some(&b'\r') {
                drain_len += 1;
            }
            if buffer.get(drain_len) == Some(&b'\n') {
                drain_len += 1;
            }
            buffer.drain(..drain_len);
            return Some(exit_code);
        }
        buffer.drain(..pos);
        return None;
    }

    let keep = end_prefix.len().saturating_sub(1);
    if buffer.len() > keep {
        let split = buffer.len() - keep;
        append_output(output, &buffer[..split], max_bytes, truncated);
        buffer.drain(..split);
    }
    None
}

fn parse_exit_code(bytes: &[u8]) -> Option<i32> {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        trimmed.parse::<i32>().ok()
    }
}

fn append_output(output: &mut Vec<u8>, chunk: &[u8], max_bytes: usize, truncated: &mut bool) {
    if output.len() >= max_bytes {
        *truncated = true;
        return;
    }
    let remaining = max_bytes - output.len();
    if chunk.len() > remaining {
        output.extend_from_slice(&chunk[..remaining]);
        *truncated = true;
    } else {
        output.extend_from_slice(chunk);
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_target() -> TargetSpec {
        TargetSpec {
            name: "dev".to_string(),
            desc: "dev".to_string(),
            ssh: Some("dev@host".to_string()),
            ssh_args: Vec::new(),
            ssh_password: None,
            terminal_locale: Some("en_US.UTF-8".to_string()),
            tty: false,
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
    fn build_session_command_wraps_cwd() {
        let request = sample_request();
        let cmd = build_session_command(&request);
        assert!(cmd.starts_with("(cd "));
        assert!(cmd.contains("&&"));
        assert!(cmd.contains("echo hello"));
    }

    #[test]
    fn build_pty_command_adds_markers() {
        let request = sample_request();
        let cmd = build_pty_command(7, &request);
        assert!(cmd.contains(PTY_MARKER_BEGIN_PREFIX));
        assert!(cmd.contains(PTY_MARKER_END_PREFIX));
        assert!(cmd.contains("status=$?"));
    }

    #[test]
    fn extract_pty_output_collects_output() {
        let begin = pty_begin_marker(1);
        let end_prefix = pty_end_prefix(1);
        let mut buffer = Vec::new();
        let mut output = Vec::new();
        let mut truncated = false;
        let mut seen_begin = false;
        buffer.extend_from_slice(b"noise");
        assert!(extract_pty_output(
            &mut buffer,
            &begin,
            &end_prefix,
            &mut output,
            1024,
            &mut truncated,
            &mut seen_begin,
        )
        .is_none());
        buffer.extend_from_slice(&begin);
        buffer.extend_from_slice(b"hello");
        let end_line = format!("{}0__\n", String::from_utf8_lossy(&end_prefix));
        buffer.extend_from_slice(end_line.as_bytes());
        let exit_code = extract_pty_output(
            &mut buffer,
            &begin,
            &end_prefix,
            &mut output,
            1024,
            &mut truncated,
            &mut seen_begin,
        )
        .expect("exit code");
        assert_eq!(exit_code, Some(0));
        assert_eq!(String::from_utf8_lossy(&output), "hello");
        assert!(buffer.is_empty());
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
            ssh: None,
            ssh_args: Vec::new(),
            ssh_password: None,
            terminal_locale: Some("en_US.utf8".to_string()),
            tty: false,
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
            ssh: None,
            ssh_args: Vec::new(),
            ssh_password: None,
            terminal_locale: None,
            tty: false,
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
            ssh: None,
            ssh_args: Vec::new(),
            ssh_password: None,
            terminal_locale: None,
            tty: false,
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
