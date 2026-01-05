use anyhow::Context;
use std::path::Path;
use std::process::{Output, Stdio};
use tokio::process::{Child, Command};
use tokio::time::Duration;
use tracing::info;
use tunnel_protocol::ForwardSpec;
use system_utils::process::run_command_with_timeout;
use system_utils::ssh::apply_askpass_env;

const SSH_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const SSH_CONNECT_TIMEOUT_SECS: u64 = 10;

pub(crate) struct SshTarget {
    pub(crate) ssh: String,
    pub(crate) ssh_args: Vec<String>,
    pub(crate) ssh_password: Option<String>,
}

pub(crate) struct MasterCheck {
    pub(crate) running: bool,
    pub(crate) detail: String,
}

pub(crate) async fn spawn_master(target: &SshTarget, control_path: &Path) -> anyhow::Result<Child> {
    let mut cmd = build_ssh_base(target, true)?;
    cmd.arg("-N")
        .arg("-T")
        .arg("-o")
        .arg("ControlMaster=yes")
        .arg("-o")
        .arg(format!("ControlPath={}", control_path.display()))
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg(format!("ConnectTimeout={}", SSH_CONNECT_TIMEOUT_SECS))
        .arg("-o")
        .arg("ServerAliveInterval=30")
        .arg("-o")
        .arg("ServerAliveCountMax=3");
    if target.ssh_password.is_none() {
        cmd.arg("-o").arg("BatchMode=yes");
    }
    if !target.ssh_args.is_empty() {
        cmd.args(&target.ssh_args);
    }
    cmd.arg(&target.ssh);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = cmd.spawn().map_err(|err| anyhow::anyhow!(err))?;
    Ok(child)
}

pub(crate) async fn check_master(
    target: &SshTarget,
    control_path: &Path,
) -> anyhow::Result<MasterCheck> {
    let mut cmd = Command::new("ssh");
    cmd.arg("-S")
        .arg(control_path)
        .arg("-O")
        .arg("check")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg(format!("ConnectTimeout={}", SSH_CONNECT_TIMEOUT_SECS))
        .arg(&target.ssh);
    if !target.ssh_args.is_empty() {
        cmd.args(&target.ssh_args);
    }
    let output = run_ssh_command_output(&mut cmd, "ssh -O check").await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = format!("{}{}", stdout, stderr).trim().to_string();
    Ok(MasterCheck {
        running: output.status.success(),
        detail,
    })
}

pub(crate) async fn forward_add(
    target: &SshTarget,
    control_path: &Path,
    forward: &ForwardSpec,
) -> anyhow::Result<()> {
    let (remote_host, remote_port) = parse_host_port(&forward.remote_addr)?;
    let mut cmd = Command::new("ssh");
    cmd.arg("-S")
        .arg(control_path)
        .arg("-O")
        .arg("forward")
        .arg("-L")
        .arg(format!(
            "{}:{}:{}:{}",
            forward.local_bind, forward.local_port, remote_host, remote_port
        ))
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg(format!("ConnectTimeout={}", SSH_CONNECT_TIMEOUT_SECS))
        .arg(&target.ssh);
    if !target.ssh_args.is_empty() {
        cmd.args(&target.ssh_args);
    }
    run_ssh_command(&mut cmd, "ssh -O forward").await
}

pub(crate) async fn forward_cancel(
    target: &SshTarget,
    control_path: &Path,
    forward: &ForwardSpec,
) -> anyhow::Result<()> {
    let (remote_host, remote_port) = parse_host_port(&forward.remote_addr)?;
    let mut cmd = Command::new("ssh");
    cmd.arg("-S")
        .arg(control_path)
        .arg("-O")
        .arg("cancel")
        .arg("-L")
        .arg(format!(
            "{}:{}:{}:{}",
            forward.local_bind, forward.local_port, remote_host, remote_port
        ))
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg(format!("ConnectTimeout={}", SSH_CONNECT_TIMEOUT_SECS))
        .arg(&target.ssh);
    if !target.ssh_args.is_empty() {
        cmd.args(&target.ssh_args);
    }
    run_ssh_command(&mut cmd, "ssh -O cancel").await
}

pub(crate) async fn exit_master(target: &SshTarget, control_path: &Path) -> anyhow::Result<()> {
    let mut cmd = Command::new("ssh");
    cmd.arg("-S")
        .arg(control_path)
        .arg("-O")
        .arg("exit")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg(format!("ConnectTimeout={}", SSH_CONNECT_TIMEOUT_SECS))
        .arg(&target.ssh);
    if !target.ssh_args.is_empty() {
        cmd.args(&target.ssh_args);
    }
    run_ssh_command(&mut cmd, "ssh -O exit").await
}

fn build_ssh_base(target: &SshTarget, allow_password: bool) -> anyhow::Result<Command> {
    let mut cmd = Command::new("ssh");
    if allow_password {
        if let Some(password) = target.ssh_password.as_ref() {
            configure_askpass(&mut cmd, password)?;
        }
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

async fn run_ssh_command(cmd: &mut Command, label: &str) -> anyhow::Result<()> {
    let output = run_ssh_command_output(cmd, label).await?;
    let status = output.status;
    let stdout = output.stdout;
    let stderr = output.stderr;
    if !status.success() {
        let stdout = String::from_utf8_lossy(&stdout);
        let stderr = String::from_utf8_lossy(&stderr);
        anyhow::bail!("{label} failed: {}{}", stdout, stderr);
    }
    Ok(())
}

async fn run_ssh_command_output(cmd: &mut Command, label: &str) -> anyhow::Result<Output> {
    run_command_with_timeout(cmd, SSH_COMMAND_TIMEOUT, label).await
}

fn parse_host_port(addr: &str) -> anyhow::Result<(String, u16)> {
    let (host, port) = addr
        .rsplit_once(':')
        .ok_or_else(|| anyhow::anyhow!("invalid address {addr}, expected host:port"))?;
    let port = port
        .parse::<u16>()
        .with_context(|| format!("invalid port in address {addr}"))?;
    Ok((host.to_string(), port))
}
