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
}
