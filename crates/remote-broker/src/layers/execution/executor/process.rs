use std::io;
use std::time::Duration;

use tokio::process::Command;

const CANCEL_GRACE: Duration = Duration::from_secs(2);

#[cfg(unix)]
pub(super) fn apply_process_group(cmd: &mut Command) {
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
pub(super) fn apply_process_group(_cmd: &mut Command) {}

#[cfg(unix)]
fn signal_child(child: &mut tokio::process::Child, signal: i32) {
    if let Some(pid) = child.id() {
        unsafe {
            libc::kill(-(pid as i32), signal);
        }
    }
}

#[cfg(not(unix))]
fn signal_child(_child: &mut tokio::process::Child, _signal: i32) {}

pub(super) async fn terminate_child(
    child: &mut tokio::process::Child,
) -> Option<std::process::ExitStatus> {
    signal_child(child, libc::SIGINT);
    match tokio::time::timeout(CANCEL_GRACE, child.wait()).await {
        Ok(status) => return status.ok(),
        Err(_) => {
            signal_child(child, libc::SIGKILL);
            let _ = child.kill().await;
            match tokio::time::timeout(CANCEL_GRACE, child.wait()).await {
                Ok(status) => status.ok(),
                Err(_) => None,
            }
        }
    }
}

pub(super) async fn terminate_children(children: &mut [tokio::process::Child]) {
    for child in children.iter_mut() {
        signal_child(child, libc::SIGINT);
    }
    for child in children.iter_mut() {
        if tokio::time::timeout(CANCEL_GRACE, child.wait())
            .await
            .is_ok()
        {
            continue;
        }
        signal_child(child, libc::SIGKILL);
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}
