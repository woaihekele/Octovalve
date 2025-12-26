use crate::tunnel::TargetRuntime;
use anyhow::Context;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

const SSH_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const SCP_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const SSH_CONNECT_TIMEOUT_SECS: u64 = 10;

#[derive(Clone, Debug)]
pub(crate) struct BootstrapConfig {
    pub(crate) local_bin: PathBuf,
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
    if !bootstrap.local_bin.exists() {
        anyhow::bail!(
            "missing local broker bin: {}",
            bootstrap.local_bin.display()
        );
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

    run_scp(target, &bootstrap.local_bin, &remote_bin_tmp).await?;
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

    Ok(())
}

async fn run_ssh(target: &TargetRuntime, remote_cmd: &str) -> anyhow::Result<()> {
    let ssh = target
        .ssh
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing ssh target"))?;
    let mut cmd = build_ssh_base(target, "ssh");
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
    Ok(())
}

async fn run_scp(target: &TargetRuntime, local: &Path, remote_path: &str) -> anyhow::Result<()> {
    let ssh = target
        .ssh
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("missing ssh target"))?;
    let remote = format!("{}:{}", ssh, remote_path);
    let mut cmd = build_ssh_base(target, "scp");
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
    let mut cmd = build_ssh_base(target, "ssh");
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

fn build_ssh_base(target: &TargetRuntime, command: &str) -> Command {
    if let Some(password) = target.ssh_password.as_ref() {
        let mut cmd = Command::new("sshpass");
        cmd.arg("-e");
        cmd.env("SSHPASS", password);
        cmd.arg(command);
        cmd
    } else {
        Command::new(command)
    }
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
