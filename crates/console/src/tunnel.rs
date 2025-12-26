use anyhow::Context;
use std::process::Stdio;
use tokio::process::Child;
use tokio::process::Command;
use tokio::time::{sleep, Duration};
use tracing::warn;

pub(crate) struct TargetRuntime {
    pub(crate) name: String,
    pub(crate) ssh: Option<String>,
    pub(crate) ssh_args: Vec<String>,
    pub(crate) ssh_password: Option<String>,
    pub(crate) control_remote_addr: String,
    pub(crate) control_local_bind: Option<String>,
    pub(crate) control_local_port: Option<u16>,
    pub(crate) control_local_addr: Option<String>,
    pub(crate) tunnel: Option<Child>,
    pub(crate) tunnel_pgid: Option<libc::pid_t>,
}

impl TargetRuntime {
    pub(crate) fn connect_addr(&self) -> String {
        self.control_local_addr
            .clone()
            .unwrap_or_else(|| self.control_remote_addr.clone())
    }

    pub(crate) fn refresh_tunnel(&mut self) -> bool {
        if let Some(child) = self.tunnel.as_mut() {
            match child.try_wait() {
                Ok(None) => true,
                Ok(Some(status)) => {
                    warn!(target = %self.name, status = %status, "ssh tunnel exited");
                    self.tunnel = None;
                    self.tunnel_pgid = None;
                    false
                }
                Err(err) => {
                    warn!(target = %self.name, error = %err, "ssh tunnel status check failed");
                    self.tunnel = None;
                    self.tunnel_pgid = None;
                    false
                }
            }
        } else {
            false
        }
    }
}

pub(crate) fn spawn_tunnel(target: &mut TargetRuntime) -> anyhow::Result<()> {
    if target.ssh.is_none() {
        return Ok(());
    }
    let bind = target
        .control_local_bind
        .as_ref()
        .context("missing control_local_bind")?;
    let port = target
        .control_local_port
        .context("missing control_local_port")?;
    let (remote_host, remote_port) = parse_host_port(&target.control_remote_addr)?;

    let mut cmd = if let Some(password) = target.ssh_password.as_ref() {
        let mut cmd = Command::new("sshpass");
        cmd.arg("-e");
        cmd.env("SSHPASS", password);
        cmd.arg("ssh");
        cmd
    } else {
        Command::new("ssh")
    };
    unsafe {
        cmd.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    cmd.arg("-N")
        .arg("-T")
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("ConnectTimeout=10")
        .arg("-o")
        .arg("ServerAliveInterval=30")
        .arg("-o")
        .arg("ServerAliveCountMax=3")
        .arg("-L")
        .arg(format!("{bind}:{port}:{remote_host}:{remote_port}"));
    if target.ssh_password.is_none() {
        cmd.arg("-o").arg("BatchMode=yes");
    }

    if !target.ssh_args.is_empty() {
        cmd.args(&target.ssh_args);
    }

    cmd.arg(
        target
            .ssh
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing ssh target"))?,
    );
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let child = cmd
        .spawn()
        .map_err(|err| {
            if target.ssh_password.is_some() && err.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!("sshpass not found; install sshpass or remove ssh_password")
            } else {
                anyhow::anyhow!(err)
            }
        })
        .context("failed to spawn ssh tunnel")?;
    target.tunnel_pgid = child.id().map(|pid| pid as libc::pid_t);
    target.tunnel = Some(child);
    Ok(())
}

pub(crate) async fn stop_tunnel(target: &mut TargetRuntime) {
    let pgid = target.tunnel_pgid.take();
    let mut child = target.tunnel.take();

    if let Some(pgid) = pgid {
        if let Err(err) = kill_process_group(pgid, libc::SIGTERM) {
            tracing::warn!(error = %err, "failed to send SIGTERM to tunnel process group");
        }
    }

    if let Some(child_ref) = child.as_mut() {
        for _ in 0..10 {
            if let Ok(Some(_)) = child_ref.try_wait() {
                return;
            }
            sleep(Duration::from_millis(100)).await;
        }
    }

    if let Some(pgid) = pgid {
        if let Err(err) = kill_process_group(pgid, libc::SIGKILL) {
            tracing::warn!(error = %err, "failed to send SIGKILL to tunnel process group");
        }
    }

    if let Some(mut child) = child {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
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

fn kill_process_group(pgid: libc::pid_t, signal: i32) -> std::io::Result<()> {
    let rc = unsafe { libc::killpg(pgid, signal) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}
