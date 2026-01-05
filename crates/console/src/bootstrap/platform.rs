use std::path::PathBuf;

use crate::tunnel::TargetRuntime;

use super::{BootstrapConfig, UnsupportedRemotePlatform};
use super::ssh::run_ssh_capture;
use super::utils::join_remote;

pub(crate) async fn resolve_remote_path(
    target: &TargetRuntime,
    path: &str,
) -> anyhow::Result<String> {
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

pub(crate) async fn select_local_bin(
    target: &TargetRuntime,
    bootstrap: &BootstrapConfig,
) -> anyhow::Result<PathBuf> {
    let (os, arch) = detect_remote_platform(target).await?;
    if os == "linux" && (arch == "x86_64" || arch == "amd64") {
        Ok(bootstrap
            .local_bin_linux_x86_64
            .as_ref()
            .unwrap_or(&bootstrap.local_bin)
            .clone())
    } else {
        Err(UnsupportedRemotePlatform { os, arch }.into())
    }
}

async fn detect_remote_platform(target: &TargetRuntime) -> anyhow::Result<(String, String)> {
    let output = run_ssh_capture(target, "uname -s && uname -m").await?;
    let mut lines = output.lines();
    let os = lines.next().unwrap_or("unknown").trim().to_lowercase();
    let arch = lines.next().unwrap_or("unknown").trim().to_lowercase();
    Ok((os, arch))
}
