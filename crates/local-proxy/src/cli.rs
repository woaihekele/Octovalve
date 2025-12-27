use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "octovalve-proxy",
    version,
    about = "MCP stdio proxy to Octovalve remote broker"
)]
pub(crate) struct Args {
    #[arg(long, default_value = "config/local-proxy-config.toml")]
    pub(crate) config: PathBuf,
    #[arg(long, default_value = "octovalve-proxy")]
    pub(crate) client_id: String,
    #[arg(long, default_value = "127.0.0.1:19310")]
    pub(crate) tunnel_daemon_addr: String,
    #[arg(long, default_value_t = 30_000)]
    pub(crate) timeout_ms: u64,
    #[arg(long, default_value_t = 1024 * 1024)]
    pub(crate) max_output_bytes: u64,
}
