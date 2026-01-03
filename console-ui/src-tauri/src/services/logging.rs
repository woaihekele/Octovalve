use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::SystemTime;

use humantime::format_rfc3339;

pub fn append_log_line(path: &Path, message: &str) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| err.to_string())?;
    let ts = format_rfc3339(SystemTime::now()).to_string();
    writeln!(file, "[{ts}] {message}").map_err(|err| err.to_string())?;
    Ok(())
}

pub fn escape_log_body(body: &str) -> String {
    if body.is_empty() {
        return "<empty>".to_string();
    }
    body.replace('\n', "\\n").replace('\r', "\\r")
}
