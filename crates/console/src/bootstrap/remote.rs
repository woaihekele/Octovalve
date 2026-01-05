use std::io::Read;
use std::path::Path;
use std::time::Instant;

use anyhow::Context;
use tokio::time::Duration;
use tracing::{info, warn};

use crate::tunnel::TargetRuntime;

use super::platform::{resolve_remote_path, select_local_bin};
use super::ssh::{run_scp, run_ssh, run_ssh_capture, run_ssh_with_timeout};
use super::utils::{join_remote, regex_escape, shell_escape};
use super::BootstrapConfig;

const REMOTE_STOP_TIMEOUT: Duration = Duration::from_secs(10);

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

    let skip_bin_upload =
        match run_bootstrap_step(target, "remote_md5", || remote_md5_hex(target, &remote_bin))
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
    run_bootstrap_step(target, "upload_config_mv", || {
        run_ssh(target, &config_move_cmd)
    })
    .await?;

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
            "setsid {} --listen-addr {} --control-addr {} --headless --config {} --audit-dir {} --log-to-stderr </dev/null >> {} 2>&1 &",
            shell_escape(&remote_bin),
            shell_escape(&bootstrap.remote_listen_addr),
            shell_escape(&bootstrap.remote_control_addr),
            shell_escape(&remote_config),
            shell_escape(&remote_audit_dir),
            shell_escape(&remote_log),
        );
        run_bootstrap_step(target, "start_remote_broker", || {
            run_ssh(target, &start_cmd)
        })
        .await?;
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
    let wait_cmd = format!(
        "i=0; while [ $i -lt 25 ]; do pgrep -f {} >/dev/null 2>&1 || exit 0; i=$((i+1)); sleep 0.2; done; exit 1",
        pgrep_pattern
    );
    run_ssh_with_timeout(target, &wait_cmd, REMOTE_STOP_TIMEOUT).await?;
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

fn local_md5_hex(path: &Path) -> anyhow::Result<String> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
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
