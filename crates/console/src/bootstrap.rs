use crate::tunnel::TargetRuntime;
use anyhow::Context;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tokio::time::{timeout, Duration};
use tracing::info;

const SSH_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const SCP_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const REMOTE_STOP_TIMEOUT: Duration = Duration::from_secs(10);
const SSH_CONNECT_TIMEOUT_SECS: u64 = 10;

#[derive(Clone, Debug)]
pub(crate) struct BootstrapConfig {
    pub(crate) local_bin: PathBuf,
    pub(crate) local_bin_linux_x86_64: Option<PathBuf>,
    pub(crate) local_bin_linux_aarch64: Option<PathBuf>,
    pub(crate) local_config: PathBuf,
    pub(crate) remote_dir: String,
    pub(crate) remote_listen_addr: String,
    pub(crate) remote_control_addr: String,
    pub(crate) remote_audit_dir: String,
}

pub(crate) async fn bootstrap_remote_broker(
    target: &TargetRuntime,
    bootstrap: &BootstrapConfig,
) -> anyhow::Result<()> {
    if target.ssh.is_none() {
        return Ok(());
    }
    info!(target = %target.name, "syncing remote broker");
    let local_bin = select_local_bin(target, bootstrap).await?;
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

    let remote_dir = resolve_remote_path(target, &bootstrap.remote_dir).await?;
    let remote_audit_dir = resolve_remote_path(target, &bootstrap.remote_audit_dir).await?;
    let remote_bin = join_remote(&remote_dir, "remote-broker");
    let remote_bin_tmp = format!("{remote_bin}.tmp");
    let remote_config = join_remote(&remote_dir, "config.toml");
    let remote_config_tmp = format!("{remote_config}.tmp");
    let remote_log = join_remote(&remote_dir, "remote-broker.log");

    run_ssh(
        target,
        &format!(
            "mkdir -p {} {}",
            shell_escape(&remote_dir),
            shell_escape(&remote_audit_dir)
        ),
    )
    .await?;

    run_scp(target, &local_bin, &remote_bin_tmp).await?;
    run_scp(target, &bootstrap.local_config, &remote_config_tmp).await?;
    run_ssh(
        target,
        &format!(
            "mv -f {} {} && mv -f {} {}",
            shell_escape(&remote_bin_tmp),
            shell_escape(&remote_bin),
            shell_escape(&remote_config_tmp),
            shell_escape(&remote_config)
        ),
    )
    .await?;

    run_ssh(target, &format!("chmod +x {}", shell_escape(&remote_bin))).await?;

    let pgrep_pattern = shell_escape(&format!(
        "[r]emote-broker.*--control-addr {}",
        bootstrap.remote_control_addr
    ));
    let start_cmd = format!(
        "pgrep -f {} >/dev/null 2>&1 || nohup {} --listen-addr {} --control-addr {} --headless --config {} --audit-dir {} > {} 2>&1 &",
        pgrep_pattern,
        shell_escape(&remote_bin),
        shell_escape(&bootstrap.remote_listen_addr),
        shell_escape(&bootstrap.remote_control_addr),
        shell_escape(&remote_config),
        shell_escape(&remote_audit_dir),
        shell_escape(&remote_log),
    );
    run_ssh(target, &start_cmd).await?;
    info!(target = %target.name, "remote broker ready");

    Ok(())
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

#[derive(Debug, Clone, Copy)]
enum RemotePlatform {
    LinuxX86_64,
    LinuxAarch64,
    Other,
}

async fn select_local_bin(
    target: &TargetRuntime,
    bootstrap: &BootstrapConfig,
) -> anyhow::Result<PathBuf> {
    let platform = detect_remote_platform(target).await?;
    match platform {
        RemotePlatform::LinuxX86_64 => {
            if let Some(path) = bootstrap.local_bin_linux_x86_64.as_ref() {
                Ok(path.clone())
            } else {
                anyhow::bail!("missing linux x86_64 broker bin; use --broker-bin-linux-x86_64")
            }
        }
        RemotePlatform::LinuxAarch64 => {
            if let Some(path) = bootstrap.local_bin_linux_aarch64.as_ref() {
                Ok(path.clone())
            } else {
                anyhow::bail!("missing linux aarch64 broker bin; use --broker-bin-linux-aarch64")
            }
        }
        RemotePlatform::Other => Ok(bootstrap.local_bin.clone()),
    }
}

async fn detect_remote_platform(target: &TargetRuntime) -> anyhow::Result<RemotePlatform> {
    let output = run_ssh_capture(target, "uname -s && uname -m").await?;
    let mut lines = output.lines();
    let os = lines.next().unwrap_or("").trim().to_lowercase();
    let arch = lines.next().unwrap_or("").trim().to_lowercase();
    let platform = match (os.as_str(), arch.as_str()) {
        ("linux", "x86_64") | ("linux", "amd64") => RemotePlatform::LinuxX86_64,
        ("linux", "aarch64") | ("linux", "arm64") => RemotePlatform::LinuxAarch64,
        _ => RemotePlatform::Other,
    };
    Ok(platform)
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
) -> anyhow::Result<std::process::Output> {
    match timeout(command_timeout, cmd.output()).await {
        Ok(result) => result.with_context(|| format!("{label} command failed")),
        Err(_) => anyhow::bail!(
            "{label} command timed out after {}s",
            command_timeout.as_secs()
        ),
    }
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
