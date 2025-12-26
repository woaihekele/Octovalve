use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "console", version, about = "Octovalve console service")]
pub(crate) struct Args {
    #[arg(long, default_value = "config/local-proxy-config.toml")]
    pub(crate) config: PathBuf,
    #[arg(long, default_value = "127.0.0.1:19309")]
    pub(crate) listen_addr: String,
    #[arg(long, default_value_t = false)]
    pub(crate) log_to_stderr: bool,
    #[arg(long, default_value = "target/release/remote-broker")]
    pub(crate) broker_bin: PathBuf,
    #[arg(long, default_value = "config/config.toml")]
    pub(crate) broker_config: PathBuf,
    #[arg(long, default_value = "~/.octovalve")]
    pub(crate) remote_dir: String,
    #[arg(long, default_value = "127.0.0.1:19307")]
    pub(crate) remote_listen_addr: String,
    #[arg(long, default_value = "127.0.0.1:19308")]
    pub(crate) remote_control_addr: String,
    #[arg(long, default_value = "~/.octovalve/logs")]
    pub(crate) remote_audit_dir: String,
    #[arg(long)]
    pub(crate) tunnel_daemon_addr: Option<String>,
    #[arg(long, default_value = "console")]
    pub(crate) tunnel_client_id: String,
}
