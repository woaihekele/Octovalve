use crate::tunnel::TargetRuntime;
use anyhow::Context;
use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Clone, Debug)]
pub(crate) struct BootstrapConfig {
    pub(crate) local_bin: PathBuf,
    pub(crate) local_config: PathBuf,
    pub(crate) remote_dir: String,
    pub(crate) remote_listen_addr: String,
    pub(crate) remote_control_addr: String,
    pub(crate) remote_audit_dir: String,
}

impl BootstrapConfig {
    pub(crate) fn remote_bin_path(&self) -> String {
        join_remote(&self.remote_dir, "remote-broker")
    }

    pub(crate) fn remote_config_path(&self) -> String {
        join_remote(&self.remote_dir, "config.toml")
    }

    pub(crate) fn remote_log_path(&self) -> String {
        join_remote(&self.remote_dir, "remote-broker.log")
    }
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

    run_ssh(
        target,
        &format!(
            "mkdir -p {} {}",
            shell_escape(&bootstrap.remote_dir),
            shell_escape(&bootstrap.remote_audit_dir)
        ),
    )
    .await?;

    let remote_bin = bootstrap.remote_bin_path();
    let remote_config = bootstrap.remote_config_path();

    run_scp(target, &bootstrap.local_bin, &remote_bin).await?;
    run_scp(target, &bootstrap.local_config, &remote_config).await?;

    run_ssh(target, &format!("chmod +x {}", shell_escape(&remote_bin))).await?;

    let pgrep_pattern = shell_escape(&format!(
        "remote-broker.*--control-addr {}",
        bootstrap.remote_control_addr
    ));
    let start_cmd = format!(
        "pgrep -f {} >/dev/null 2>&1 || nohup {} --listen-addr {} --control-addr {} --headless --config {} --audit-dir {} > {} 2>&1 &",
        pgrep_pattern,
        shell_escape(&remote_bin),
        shell_escape(&bootstrap.remote_listen_addr),
        shell_escape(&bootstrap.remote_control_addr),
        shell_escape(&remote_config),
        shell_escape(&bootstrap.remote_audit_dir),
        shell_escape(&bootstrap.remote_log_path()),
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
    if target.ssh_password.is_none() {
        cmd.arg("-o").arg("BatchMode=yes");
    }
    cmd.args(&target.ssh_args);
    cmd.arg(ssh);
    cmd.arg("sh").arg("-lc").arg(remote_cmd);
    let output = cmd.output().await.context("ssh command failed")?;
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
    if target.ssh_password.is_none() {
        cmd.arg("-o").arg("BatchMode=yes");
    }
    cmd.args(&target.ssh_args);
    cmd.arg(local);
    cmd.arg(remote);
    let output = cmd.output().await.context("scp command failed")?;
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("scp failed: {}{}", stdout, stderr);
    }
    Ok(())
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
