use anyhow::Context;
use clap::Parser;
use std::collections::BTreeMap;
use std::process::{Command, Stdio};

#[derive(Parser, Debug)]
#[command(name = "test_pty", about = "Test remote command execution via SSH")]
struct Args {
    #[arg(long)]
    ssh: String,
    #[arg(long)]
    command: String,
    #[arg(long)]
    cwd: Option<String>,
    #[arg(long)]
    tty: bool,
    #[arg(long)]
    locale: Option<String>,
    #[arg(long)]
    print: bool,
    #[arg(long, value_name = "KEY=VALUE")]
    env: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let env_pairs = parse_env_pairs(&args.env)?;
    let locale = resolve_locale(args.locale);
    let remote_cmd = build_remote_command(&args.command, args.cwd.as_deref(), &env_pairs, locale);
    if args.print {
        println!("remote_cmd={remote_cmd}");
    }

    let mut cmd = Command::new("ssh");
    if args.tty {
        cmd.arg("-tt");
    } else {
        cmd.arg("-T");
    }
    cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
    cmd.arg("-o").arg("ConnectTimeout=10");
    cmd.arg("-o").arg("BatchMode=yes");
    cmd.arg(args.ssh);
    cmd.arg(remote_cmd);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let output = cmd.output().context("execute ssh")?;
    println!("---- stdout ----");
    print!("{}", String::from_utf8_lossy(&output.stdout));
    println!("---- stderr ----");
    print!("{}", String::from_utf8_lossy(&output.stderr));
    println!("exit_code={}", output.status.code().unwrap_or(-1));
    Ok(())
}

fn resolve_locale(explicit: Option<String>) -> Option<String> {
    if let Some(value) = explicit {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Some(locale) = env_locale("OCTOVALVE_TERMINAL_LOCALE") {
        return Some(locale);
    }
    if let Some(locale) = env_language_locale("OCTOVALVE_APP_LANGUAGE") {
        return Some(locale);
    }
    if let Some(locale) = env_language_locale("LANG") {
        return Some(locale);
    }
    Some("en_US.utf8".to_string())
}

fn env_locale(key: &str) -> Option<String> {
    let value = std::env::var(key).ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn env_language_locale(key: &str) -> Option<String> {
    let value = std::env::var(key).ok()?;
    let trimmed = value.trim().to_lowercase();
    if trimmed.starts_with("zh") {
        return Some("zh_CN.utf8".to_string());
    }
    if trimmed.starts_with("en") {
        return Some("en_US.utf8".to_string());
    }
    None
}

fn parse_env_pairs(values: &[String]) -> anyhow::Result<BTreeMap<String, String>> {
    let mut pairs = BTreeMap::new();
    for item in values {
        let (key, value) = item
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("invalid env pair: {item}"))?;
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        pairs.insert(key.to_string(), value.trim().to_string());
    }
    Ok(pairs)
}

fn build_remote_command(
    raw_command: &str,
    cwd: Option<&str>,
    env_pairs: &BTreeMap<String, String>,
    locale: Option<String>,
) -> String {
    let mut shell_prefix = String::new();
    if let Some(locale) = locale {
        let escaped = shell_escape(&locale);
        shell_prefix.push_str(&format!(
            "LANG={escaped} LC_CTYPE={escaped} LC_ALL={escaped} "
        ));
    }

    let env_prefix = build_env_prefix(env_pairs);
    let mut command = String::new();
    if let Some(cwd) = cwd.filter(|value| !value.trim().is_empty()) {
        command.push_str("cd ");
        command.push_str(&shell_escape(cwd));
        command.push_str(" && ");
    }
    if !env_prefix.is_empty() {
        command.push_str(&env_prefix);
        command.push(' ');
    }
    command.push_str(raw_command.trim());
    format!(
        "{shell_prefix}bash --noprofile -lc {}",
        shell_escape(&command)
    )
}

fn build_env_prefix(pairs: &BTreeMap<String, String>) -> String {
    let mut parts = Vec::new();
    for (key, value) in pairs {
        if key.trim().is_empty() {
            continue;
        }
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        parts.push(format!("{key}={}", shell_escape(value)));
    }
    parts.join(" ")
}

fn shell_escape(value: &str) -> String {
    let mut escaped = String::from("'");
    for ch in value.chars() {
        if ch == '\'' {
            escaped.push_str("'\"'\"'");
        } else {
            escaped.push(ch);
        }
    }
    escaped.push('\'');
    escaped
}
