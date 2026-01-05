mod platform;
mod remote;
mod ssh;
mod utils;

use std::fmt;
use std::path::PathBuf;

pub(crate) use remote::{bootstrap_remote_broker, stop_remote_broker};

#[derive(Clone, Debug)]
pub(crate) struct BootstrapConfig {
    pub(crate) local_bin: PathBuf,
    pub(crate) local_bin_linux_x86_64: Option<PathBuf>,
    pub(crate) local_config: PathBuf,
    pub(crate) remote_dir: String,
    pub(crate) remote_listen_addr: String,
    pub(crate) remote_control_addr: String,
    pub(crate) remote_audit_dir: String,
}

#[derive(Debug)]
pub(crate) struct UnsupportedRemotePlatform {
    pub(crate) os: String,
    pub(crate) arch: String,
}

impl fmt::Display for UnsupportedRemotePlatform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unsupported remote platform: {} {}; only linux x86_64 is supported",
            self.os, self.arch
        )
    }
}

impl std::error::Error for UnsupportedRemotePlatform {}
