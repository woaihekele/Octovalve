use crate::tunnel::TargetRuntime;
use anyhow::Context;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::Instant;
use tokio::process::Command;
use tokio::io::AsyncReadExt;
use tokio::time::{timeout, Duration};
use tracing::{info, warn};

const SSH_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const SCP_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const REMOTE_STOP_TIMEOUT: Duration = Duration::from_secs(10);
const SSH_CONNECT_TIMEOUT_SECS: u64 = 10;

#[derive(Clone, Debug)]
pub(crate) struct BootstrapConfig {
    pub(crate) local_bin: PathBuf,
    pub(crate) local_bin_linux_x86_64: Option<PathBuf>,
    pub(crate) local_config: PathBuf,
    pub(crate) remote_dir: String,
    pub(crate) remote_listen_addr: String,
    pub(crate) remote_control_addr: String,
    pub(crate) remote_audit_dir: String,
}

#[derive(Debug)]
pub(crate) struct UnsupportedRemotePlatform {
    pub(crate) os: String,
    pub(crate) arch: String,
}

impl fmt::Display for UnsupportedRemotePlatform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unsupported remote platform: {} {}; only linux x86_64 is supported",
            self.os, self.arch
        )
    }
}

impl std::error::Error for UnsupportedRemotePlatform {}

pub(crate) async fn bootstrap_remote_broker(
    target: &TargetRuntime,
    bootstrap: &BootstrapConfig,
) -> anyhow::Result<()> {
    if target.ssh.is_none() {
        return Ok(());
    }
    info!(event = "bootstrap.start", target = %target.name, "syncing remote broker");
    let local_bin = run_bootstrap_step(target, "select_local_bin", || {
        select_local_bin(target, bootstrap)
    })
    .await?;
    info!(
        target = %target.name,
        broker_bin = %local_bin.display(),
        "selected remote broker binary"
    );
    if !local_bin.exists() {
        anyhow::bail!("missing local broker bin: {}", local_bin.display());
    }
    if !bootstrap.local_config.exists() {
        anyhow::bail!(
            "missing local broker config: {}",
            bootstrap.local_config.display()
        );
    }

    let remote_dir = run_bootstrap_step(target, "resolve_remote_dir", || {
        resolve_remote_path(target, &bootstrap.remote_dir)
    })
    .await?;
    let remote_audit_dir = run_bootstrap_step(target, "resolve_remote_audit_dir", || {
        resolve_remote_path(target, &bootstrap.remote_audit_dir)
    })
    .await?;
    let remote_bin = join_remote(&remote_dir, "remote-broker");
    let remote_bin_tmp = format!("{remote_bin}.tmp");
    let remote_config = join_remote(&remote_dir, "config.toml");
    let remote_config_tmp = format!("{remote_config}.tmp");
    let remote_log = join_remote(&remote_dir, "remote-broker.log");

    let mkdir_cmd = format!(
        "mkdir -p {} {}",
        shell_escape(&remote_dir),
        shell_escape(&remote_audit_dir)
    );
    run_bootstrap_step(target, "mkdir_remote_dirs", || run_ssh(target, &mkdir_cmd)).await?;

    let skip_bin_upload = match run_bootstrap_step(target, "remote_md5", || {
        remote_md5_hex(target, &remote_bin)
    })
    .await?
    {
        Some(remote_md5) if remote_md5 == local_md5_hex(&local_bin)? => {
            info!(target = %target.name, "remote broker binary up to date, skipping upload");
            true
        }
        _ => false,
    };
    if !skip_bin_upload {
        run_bootstrap_step(target, "upload_bin_scp", || {
            run_scp(target, &local_bin, &remote_bin_tmp)
        })
        .await?;
        let bin_move_cmd = format!(
            "mv -f {} {}",
            shell_escape(&remote_bin_tmp),
            shell_escape(&remote_bin)
        );
        run_bootstrap_step(target, "upload_bin_mv", || run_ssh(target, &bin_move_cmd)).await?;
    }
    run_bootstrap_step(target, "upload_config_scp", || {
        run_scp(target, &bootstrap.local_config, &remote_config_tmp)
    })
    .await?;
    let config_move_cmd = format!(
        "mv -f {} {}",
        shell_escape(&remote_config_tmp),
        shell_escape(&remote_config)
    );
    run_bootstrap_step(target, "upload_config_mv", || run_ssh(target, &config_move_cmd)).await?;

    let chmod_cmd = format!("chmod +x {}", shell_escape(&remote_bin));
    run_bootstrap_step(target, "chmod_remote_bin", || run_ssh(target, &chmod_cmd)).await?;

    let pgrep_pattern = format!(
        "^{}.*--control-addr {}",
        regex_escape(&remote_bin),
        regex_escape(&bootstrap.remote_control_addr)
    );
    let check_cmd = format!(
        "pgrep -f {} >/dev/null 2>&1 && echo running || true",
        shell_escape(&pgrep_pattern)
    );
    let check_output = run_bootstrap_step(target, "check_remote_broker", || {
        run_ssh_capture(target, &check_cmd)
    })
    .await?;
    if check_output.trim() != "running" {
        let start_cmd = format!(
            "setsid {} --listen-addr {} --control-addr {} --headless --config {} --audit-dir {} </dev/null > {} 2>&1 &",
            shell_escape(&remote_bin),
            shell_escape(&bootstrap.remote_listen_addr),
            shell_escape(&bootstrap.remote_control_addr),
            shell_escape(&remote_config),
            shell_escape(&remote_audit_dir),
            shell_escape(&remote_log),
        );
        run_bootstrap_step(target, "start_remote_broker", || run_ssh(target, &start_cmd)).await?;
    } else {
        info!(
            event = "bootstrap.skip_start",
            target = %target.name,
            "remote broker already running"
        );
    }
    info!(event = "bootstrap.ready", target = %target.name, "remote broker ready");

    Ok(())
}

async fn run_bootstrap_step<T, F, Fut>(
    target: &TargetRuntime,
    step: &'static str,
    f: F,
) -> anyhow::Result<T>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    info!(
        event = "bootstrap.step.start",
        target = %target.name,
        step,
        "bootstrap step start"
    );
    let start = Instant::now();
    match f().await {
        Ok(value) => {
            info!(
                event = "bootstrap.step.done",
                target = %target.name,
                step,
                elapsed_ms = start.elapsed().as_millis(),
                "bootstrap step done"
            );
            Ok(value)
        }
        Err(err) => {
            warn!(
                event = "bootstrap.step.failed",
                target = %target.name,
                step,
                elapsed_ms = start.elapsed().as_millis(),
                error = %err,
                "bootstrap step failed"
            );
            Err(err)
        }
    }
}

pub(crate) async fn stop_remote_broker(
    target: &TargetRuntime,
    bootstrap: &BootstrapConfig,
) -> anyhow::Result<()> {
    if target.ssh.is_none() {
        return Ok(());
    }
    let pgrep_pattern = shell_escape(&format!(
        "[r]emote-broker.*--control-addr {}",
        bootstrap.remote_control_addr
    ));
    let stop_cmd = format!("pkill -f {} >/dev/null 2>&1 || true", pgrep_pattern);
    run_ssh_with_timeout(target, &stop_cmd, REMOTE_STOP_TIMEOUT).await?;
    Ok(())
}

async fn run_ssh(target: &TargetRuntime, remote_cmd: &str) -> anyhow::Result<()> {
    run_ssh_with_timeout(target, remote_cmd, SSH_COMMAND_TIMEOUT).await
}

async fn run_ssh_with_timeout(
    target: &TargetRuntime,
    remote_cmd: &str,
    timeout: Duration,
) -> anyhow::Result<()> {
    let ssh = target
        .ssh
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing ssh target"))?;
    let mut cmd = build_ssh_base(target, "ssh")?;
    cmd.arg("-T");
    apply_ssh_options(&mut cmd, target.ssh_password.is_some());
    cmd.args(&target.ssh_args);
    cmd.arg(ssh);
    cmd.arg(remote_cmd);
    let output = run_command_with_timeout(&mut cmd, timeout, "ssh").await?;
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ssh failed: {}{}", stdout, stderr);
    }
    Ok(())
}

async fn run_scp(target: &TargetRuntime, local: &Path, remote_path: &str) -> anyhow::Result<()> {
    let ssh = target
        .ssh
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing ssh target"))?;
    let remote = format!("{}:{}", ssh, remote_path);
    let mut cmd = build_ssh_base(target, "scp")?;
    apply_ssh_options(&mut cmd, target.ssh_password.is_some());
    cmd.args(&target.ssh_args);
    cmd.arg(local);
    cmd.arg(remote);
    let output = run_command_with_timeout(&mut cmd, SCP_COMMAND_TIMEOUT, "scp").await?;
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("scp failed: {}{}", stdout, stderr);
    }
    Ok(())
}

async fn run_ssh_capture(target: &TargetRuntime, remote_cmd: &str) -> anyhow::Result<String> {
    let ssh = target
        .ssh
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing ssh target"))?;
    let mut cmd = build_ssh_base(target, "ssh")?;
    cmd.arg("-T");
    apply_ssh_options(&mut cmd, target.ssh_password.is_some());
    cmd.args(&target.ssh_args);
    cmd.arg(ssh);
    cmd.arg(remote_cmd);
    let output = run_command_with_timeout(&mut cmd, SSH_COMMAND_TIMEOUT, "ssh").await?;
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("ssh failed: {}{}", stdout, stderr);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn local_md5_hex(path: &Path) -> anyhow::Result<String> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    let mut context = md5::Context::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        context.consume(&buffer[..read]);
    }
    Ok(format!("{:x}", context.compute()))
}

async fn remote_md5_hex(
    target: &TargetRuntime,
    remote_path: &str,
) -> anyhow::Result<Option<String>> {
    let escaped = shell_escape(remote_path);
    let remote_cmd = format!(
        "if command -v md5sum >/dev/null 2>&1 && [ -f {escaped} ]; then md5sum {escaped}; fi; true"
    );
    let output = match run_ssh_capture(target, &remote_cmd).await {
        Ok(output) => output,
        Err(err) => {
            warn!(target = %target.name, error = %err, "failed to check remote md5");
            return Ok(None);
        }
    };
    let hash = output.split_whitespace().next().unwrap_or("");
    if hash.is_empty() {
        return Ok(None);
    }
    Ok(Some(hash.to_string()))
}

async fn resolve_remote_path(target: &TargetRuntime, path: &str) -> anyhow::Result<String> {
    if path == "~" {
        return remote_home(target).await;
    }
    if let Some(rest) = path.strip_prefix("~/") {
        let home = remote_home(target).await?;
        return Ok(join_remote(&home, rest));
    }
    Ok(path.to_string())
}

async fn remote_home(target: &TargetRuntime) -> anyhow::Result<String> {
    let home = run_ssh_capture(target, "printf '%s' \"$HOME\"").await?;
    if home.is_empty() {
        anyhow::bail!("unable to resolve remote home directory");
    }
    Ok(home)
}

async fn select_local_bin(
    target: &TargetRuntime,
    bootstrap: &BootstrapConfig,
) -> anyhow::Result<PathBuf> {
    let (os, arch) = detect_remote_platform(target).await?;
    if os == "linux" && (arch == "x86_64" || arch == "amd64") {
        Ok(bootstrap
            .local_bin_linux_x86_64
            .as_ref()
            .unwrap_or(&bootstrap.local_bin)
            .clone())
    } else {
        Err(UnsupportedRemotePlatform { os, arch }.into())
    }
}

async fn detect_remote_platform(target: &TargetRuntime) -> anyhow::Result<(String, String)> {
    let output = run_ssh_capture(target, "uname -s && uname -m").await?;
    let mut lines = output.lines();
    let os = lines.next().unwrap_or("unknown").trim().to_lowercase();
    let arch = lines.next().unwrap_or("unknown").trim().to_lowercase();
    Ok((os, arch))
}

fn build_ssh_base(target: &TargetRuntime, command: &str) -> anyhow::Result<Command> {
    let mut cmd = Command::new(command);
    if let Some(password) = target.ssh_password.as_ref() {
        configure_askpass(&mut cmd, password)?;
    }
    Ok(cmd)
}

fn configure_askpass(cmd: &mut Command, password: &str) -> anyhow::Result<()> {
    let script = ensure_askpass_script()?;
    cmd.env("OCTOVALVE_SSH_PASS", password);
    cmd.env("SSH_ASKPASS", script);
    cmd.env("SSH_ASKPASS_REQUIRE", "force");
    cmd.env("DISPLAY", "1");
    Ok(())
}

fn ensure_askpass_script() -> anyhow::Result<PathBuf> {
    let home = std::env::var("HOME").context("failed to resolve HOME for askpass")?;
    let dir = PathBuf::from(home).join(".octovalve");
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let path = dir.join("ssh-askpass.sh");
    if !path.exists() {
        std::fs::write(&path, "#!/bin/sh\nprintf '%s' \"$OCTOVALVE_SSH_PASS\"\n")
            .with_context(|| format!("failed to write {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path)?.permissions();
            perms.set_mode(0o700);
            std::fs::set_permissions(&path, perms)?;
        }
    }
    Ok(path)
}

fn apply_ssh_options(cmd: &mut Command, has_password: bool) {
    cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
    cmd.arg("-o")
        .arg(format!("ConnectTimeout={}", SSH_CONNECT_TIMEOUT_SECS));
    if !has_password {
        cmd.arg("-o").arg("BatchMode=yes");
    }
}

async fn run_command_with_timeout(
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
        Ok(result) => result.with_context(|| format!("{label} command failed"))?,
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            anyhow::bail!(
                "{label} command timed out after {}s",
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

fn join_remote(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

fn shell_escape(value: &str) -> String {
    let escaped = value.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

fn regex_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '.' | '+' | '*' | '?' | '(' | ')' | '|' | '{' | '}' | '[' | ']' | '^' | '$'
            | '\\' | '-' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}
