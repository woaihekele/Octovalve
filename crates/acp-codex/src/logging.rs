use std::fmt;
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy, Debug)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
}

type LogSink = Arc<dyn Fn(LogLevel, &str) + Send + Sync + 'static>;

static LOG_SINK: OnceLock<LogSink> = OnceLock::new();

pub fn set_log_sink(sink: LogSink) {
    let _ = LOG_SINK.set(sink);
}

pub fn log_fmt(level: LogLevel, args: fmt::Arguments) {
    let tag = match level {
        LogLevel::Error => "error",
        LogLevel::Warn => "warn",
        LogLevel::Info => "info",
    };
    let line = format!("[acp-codex][{tag}] {args}");
    if let Some(sink) = LOG_SINK.get() {
        (sink)(level, &line);
    } else {
        eprintln!("{line}");
    }
}
