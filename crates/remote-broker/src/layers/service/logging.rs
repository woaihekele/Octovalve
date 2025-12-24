use std::io;
use std::path::PathBuf;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

pub(crate) fn init_tracing(
    audit_dir: &PathBuf,
    log_to_stderr: bool,
) -> anyhow::Result<tracing_appender::non_blocking::WorkerGuard> {
    std::fs::create_dir_all(audit_dir)?;
    let file_appender = tracing_appender::rolling::daily(audit_dir, "audit.log");
    let (file_writer, file_guard) = tracing_appender::non_blocking(file_appender);

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_writer)
        .with_target(false)
        .json();

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let registry = tracing_subscriber::registry().with(filter).with(file_layer);

    if log_to_stderr {
        let stderr_layer = tracing_subscriber::fmt::layer()
            .with_writer(io::stderr)
            .with_target(false);
        registry.with(stderr_layer).init();
    } else {
        registry.init();
    }

    Ok(file_guard)
}
