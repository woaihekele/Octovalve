use clap::Parser;
use std::path::PathBuf;
use tunnel_protocol::DEFAULT_TUNNEL_DAEMON_ADDR;

#[derive(Parser, Debug)]
#[command(name = "tunnel-daemon", version, about = "SSH tunnel manager for Octovalve")]
pub(crate) struct Args {
    #[arg(long, default_value = "config/local-proxy-config.toml")]
    pub(crate) config: PathBuf,
    #[arg(long, default_value = DEFAULT_TUNNEL_DAEMON_ADDR)]
    pub(crate) listen_addr: String,
    #[arg(long)]
    pub(crate) control_dir: Option<String>,
    #[arg(long, default_value_t = false)]
    pub(crate) log_to_stderr: bool,
}
