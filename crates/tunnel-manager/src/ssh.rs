use anyhow::Context;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncReadExt;
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

async fn run_ssh_command(cmd: &mut Command, label: &str) -> anyhow::Result<()> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|err| anyhow::anyhow!(err))?;
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let status = match timeout(SSH_COMMAND_TIMEOUT, child.wait()).await {
        Ok(result) => result.with_context(|| format!("{label} failed"))?,
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(anyhow::anyhow!(
                "{label} timed out after {}s",
                SSH_COMMAND_TIMEOUT.as_secs()
            ));
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
    if !status.success() {
        let stdout = String::from_utf8_lossy(&stdout);
        let stderr = String::from_utf8_lossy(&stderr);
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
