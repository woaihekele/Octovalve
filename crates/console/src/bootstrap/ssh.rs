use std::path::Path;

use tokio::process::Command;
use tokio::time::Duration;
use tracing::info;
use system_utils::process::run_command_with_timeout;
use system_utils::ssh::apply_askpass_env;

use crate::tunnel::TargetRuntime;

const SSH_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const SCP_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const SSH_CONNECT_TIMEOUT_SECS: u64 = 10;

pub(crate) async fn run_ssh(target: &TargetRuntime, remote_cmd: &str) -> anyhow::Result<()> {
    run_ssh_with_timeout(target, remote_cmd, SSH_COMMAND_TIMEOUT).await
}

pub(crate) async fn run_ssh_with_timeout(
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
    let output = run_command_with_timeout(&mut cmd, timeout, "ssh command").await?;
    if !output.status.success() {
        let message = format_ssh_failure(
            "ssh",
            &output.stdout,
            &output.stderr,
            target.ssh_password.is_some(),
        );
        anyhow::bail!(message);
    }
    Ok(())
}

pub(crate) async fn run_scp(
    target: &TargetRuntime,
    local: &Path,
    remote_path: &str,
) -> anyhow::Result<()> {
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
    let output = run_command_with_timeout(&mut cmd, SCP_COMMAND_TIMEOUT, "scp command").await?;
    if !output.status.success() {
        let message = format_ssh_failure(
            "scp",
            &output.stdout,
            &output.stderr,
            target.ssh_password.is_some(),
        );
        anyhow::bail!(message);
    }
    Ok(())
}

pub(crate) async fn run_ssh_capture(
    target: &TargetRuntime,
    remote_cmd: &str,
) -> anyhow::Result<String> {
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
    let output = run_command_with_timeout(&mut cmd, SSH_COMMAND_TIMEOUT, "ssh command").await?;
    if !output.status.success() {
        let message = format_ssh_failure(
            "ssh",
            &output.stdout,
            &output.stderr,
            target.ssh_password.is_some(),
        );
        anyhow::bail!(message);
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn format_ssh_failure(label: &str, stdout: &[u8], stderr: &[u8], has_password: bool) -> String {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    let detail = format!("{}{}", stdout, stderr).trim().to_string();
    let mut message = if detail.is_empty() {
        format!("{label} failed")
    } else {
        format!("{label} failed: {detail}")
    };
    if let Some(hint) = ssh_auth_hint(&detail, has_password) {
        message.push('\n');
        message.push_str(hint);
    }
    message
}

fn ssh_auth_hint(detail: &str, has_password: bool) -> Option<&'static str> {
    let detail = detail.to_lowercase();
    if detail.contains("keyboard-interactive")
        || detail.contains("verification code")
        || detail.contains("two-factor")
    {
        return Some(
            "ssh requires keyboard-interactive/2FA; SSH_ASKPASS cannot handle it. Use SSH key auth or adjust server auth settings.",
        );
    }
    if detail.contains("permission denied")
        || detail.contains("authentication failed")
        || detail.contains("no supported authentication methods available")
        || detail.contains("too many authentication failures")
    {
        if has_password {
            return Some(
                "ssh password auth failed. Check ssh_password; if 2FA/keyboard-interactive is required, use SSH keys instead.",
            );
        }
        return Some(
            "ssh authentication failed. Configure SSH keys (preferred) or set ssh_password if password login is allowed.",
        );
    }
    None
}

fn build_ssh_base(target: &TargetRuntime, command: &str) -> anyhow::Result<Command> {
    let mut cmd = Command::new(command);
    if let Some(password) = target.ssh_password.as_ref() {
        configure_askpass(&mut cmd, password)?;
    }
    Ok(cmd)
}

fn configure_askpass(cmd: &mut Command, password: &str) -> anyhow::Result<()> {
    info!(
        event = "ssh.auth.askpass",
        "using SSH_ASKPASS for password auth"
    );
    apply_askpass_env(cmd, password)
}

fn apply_ssh_options(cmd: &mut Command, has_password: bool) {
    cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
    cmd.arg("-o")
        .arg(format!("ConnectTimeout={}", SSH_CONNECT_TIMEOUT_SECS));
    if !has_password {
        cmd.arg("-o").arg("BatchMode=yes");
    }
}
