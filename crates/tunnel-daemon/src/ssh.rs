use anyhow::Context;
use std::path::Path;
use std::process::Stdio;
use tokio::process::{Child, Command};
use tokio::time::{timeout, Duration};
use tunnel_protocol::ForwardSpec;

const SSH_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const SSH_CONNECT_TIMEOUT_SECS: u64 = 10;

pub(crate) struct SshTarget {
    pub(crate) ssh: String,
    pub(crate) ssh_args: Vec<String>,
    pub(crate) ssh_password: Option<String>,
}

pub(crate) async fn spawn_master(target: &SshTarget, control_path: &Path) -> anyhow::Result<Child> {
    let mut cmd = build_ssh_base(target, true);
    cmd.arg("-N")
        .arg("-T")
        .arg("-o")
        .arg("ControlMaster=yes")
        .arg("-o")
        .arg(format!("ControlPath={}", control_path.display()))
        .arg("-o")
        .arg("ControlPersist=yes")
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
    let child = cmd.spawn().map_err(|err| {
        if target.ssh_password.is_some() && err.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!("sshpass not found; install sshpass or remove ssh_password")
        } else {
            anyhow::anyhow!(err)
        }
    })?;
    Ok(child)
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

fn build_ssh_base(target: &SshTarget, allow_password: bool) -> Command {
    if allow_password {
        if let Some(password) = target.ssh_password.as_ref() {
            let mut cmd = Command::new("sshpass");
            cmd.arg("-e");
            cmd.env("SSHPASS", password);
            cmd.arg("ssh");
            return cmd;
        }
    }
    Command::new("ssh")
}

async fn run_ssh_command(cmd: &mut Command, label: &str) -> anyhow::Result<()> {
    let output = match timeout(SSH_COMMAND_TIMEOUT, cmd.output()).await {
        Ok(result) => result.with_context(|| format!("{label} failed"))?,
        Err(_) => anyhow::bail!("{label} timed out after {}s", SSH_COMMAND_TIMEOUT.as_secs()),
    };
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{label} failed: {}{}", stdout, stderr);
    }
    Ok(())
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
