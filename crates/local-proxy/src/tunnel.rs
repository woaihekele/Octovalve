use crate::state::{ProxyState, TargetRuntime, TargetStatus};
use anyhow::Context;
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;

pub(crate) fn spawn_tunnel_manager(state: Arc<RwLock<ProxyState>>, shutdown: CancellationToken) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = interval.tick() => {
                    let mut state = state.write().await;
                    state.ensure_all_tunnels();
                }
            }
        }
    });
}

pub(crate) fn spawn_shutdown_handler(state: Arc<RwLock<ProxyState>>, shutdown: CancellationToken) {
    tokio::spawn(async move {
        let mut sigint = match signal(SignalKind::interrupt()) {
            Ok(signal) => signal,
            Err(err) => {
                tracing::warn!(error = %err, "failed to register SIGINT handler");
                return;
            }
        };
        let mut sigterm = match signal(SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(err) => {
                tracing::warn!(error = %err, "failed to register SIGTERM handler");
                return;
            }
        };
        let mut sighup = match signal(SignalKind::hangup()) {
            Ok(signal) => signal,
            Err(err) => {
                tracing::warn!(error = %err, "failed to register SIGHUP handler");
                return;
            }
        };
        let mut sigquit = match signal(SignalKind::quit()) {
            Ok(signal) => signal,
            Err(err) => {
                tracing::warn!(error = %err, "failed to register SIGQUIT handler");
                return;
            }
        };

        tokio::select! {
            _ = sigint.recv() => {
                tracing::info!("received SIGINT, shutting down tunnels");
            }
            _ = sigterm.recv() => {
                tracing::info!("received SIGTERM, shutting down tunnels");
            }
            _ = sighup.recv() => {
                tracing::info!("received SIGHUP, shutting down tunnels");
            }
            _ = sigquit.recv() => {
                tracing::info!("received SIGQUIT, shutting down tunnels");
            }
        }

        shutdown.cancel();
        shutdown_tunnels(state).await;
        std::process::exit(0);
    });
}

pub(crate) async fn shutdown_tunnels(state: Arc<RwLock<ProxyState>>) {
    let mut state = state.write().await;
    for name in state.target_names() {
        if let Some(target) = state.get_target_mut(&name) {
            stop_tunnel(target).await;
        }
    }
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
                target.status = TargetStatus::Down;
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
    target.status = TargetStatus::Down;
}

fn kill_process_group(pgid: libc::pid_t, signal: i32) -> std::io::Result<()> {
    let rc = unsafe { libc::killpg(pgid, signal) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
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

pub(crate) fn spawn_tunnel(target: &mut TargetRuntime) -> anyhow::Result<()> {
    if target.ssh.is_none() {
        target.status = TargetStatus::Ready;
        return Ok(());
    }
    let bind = target
        .local_bind
        .as_ref()
        .context("missing local_bind")?;
    let port = target.local_port.context("missing local_port")?;
    let (remote_host, remote_port) = parse_host_port(&target.remote_addr)?;

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
    target.status = TargetStatus::Ready;
    Ok(())
}
