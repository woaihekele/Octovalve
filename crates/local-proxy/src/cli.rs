use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "local-proxy",
    version,
    about = "MCP stdio proxy to remote command broker"
)]
pub(crate) struct Args {
    #[arg(long, default_value = "127.0.0.1:19306")]
    pub(crate) remote_addr: String,
    #[arg(long)]
    pub(crate) config: Option<PathBuf>,
    #[arg(long, default_value = "local-proxy")]
    pub(crate) client_id: String,
    #[arg(long)]
    pub(crate) tunnel_daemon_addr: Option<String>,
    #[arg(long, default_value_t = 30_000)]
    pub(crate) timeout_ms: u64,
    #[arg(long, default_value_t = 1024 * 1024)]
    pub(crate) max_output_bytes: u64,
}
