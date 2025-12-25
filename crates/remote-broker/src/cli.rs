use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "remote-broker",
    version,
    about = "Remote command broker with approval TUI"
)]
pub(crate) struct Args {
    #[arg(long, default_value = "127.0.0.1:19307")]
    pub(crate) listen_addr: String,
    #[arg(long, default_value = "config/config.toml")]
    pub(crate) config: PathBuf,
    #[arg(long, default_value = "logs")]
    pub(crate) audit_dir: PathBuf,
    #[arg(long)]
    pub(crate) auto_approve: bool,
    #[arg(long, default_value_t = false)]
    pub(crate) log_to_stderr: bool,
}
