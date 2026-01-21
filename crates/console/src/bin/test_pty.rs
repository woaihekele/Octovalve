use anyhow::Context;
use clap::Parser;
use std::collections::BTreeMap;
use std::process::{Command, Stdio};

#[path = "../shell_utils.rs"]
mod shell_utils;

use shell_utils::{
    apply_ssh_options, build_env_prefix, env_language_locale, env_locale, shell_escape,
};

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
    let remote_cmd = build_remote_command(
        &args.command,
        args.cwd.as_deref(),
        &env_pairs,
        locale.as_deref(),
    );
    if args.print {
        println!("remote_cmd={remote_cmd}");
    }

    let mut cmd = Command::new("ssh");
    if args.tty {
        cmd.arg("-tt");
    } else {
        cmd.arg("-T");
    }
    apply_ssh_options(&mut cmd, false);
    apply_locale_env(&mut cmd, locale.as_deref());
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
    locale: Option<&str>,
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

fn apply_locale_env(cmd: &mut Command, locale: Option<&str>) {
    let Some(locale) = locale else {
        return;
    };
    cmd.env("LANG", locale);
    cmd.env("LC_CTYPE", locale);
    cmd.env("LC_ALL", locale);
    cmd.arg("-o").arg("SendEnv=LANG,LC_CTYPE,LC_ALL");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_remote_command_includes_locale() {
        let cmd = build_remote_command("whoami", None, &BTreeMap::new(), Some("en_US.utf8"));
        assert!(cmd.contains("LANG="));
        assert!(cmd.contains("LC_CTYPE="));
        assert!(cmd.contains("LC_ALL="));
        assert!(cmd.contains("bash --noprofile -lc "));
    }

    #[test]
    fn apply_locale_env_sets_sendenv() {
        let mut cmd = Command::new("ssh");
        apply_locale_env(&mut cmd, Some("en_US.utf8"));
        let args: Vec<String> = cmd
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect();
        assert!(args.contains(&"-o".to_string()));
        assert!(args.contains(&"SendEnv=LANG,LC_CTYPE,LC_ALL".to_string()));
        let envs: Vec<(String, String)> = cmd
            .get_envs()
            .filter_map(|(key, value)| {
                value.map(|value| {
                    (
                        key.to_string_lossy().to_string(),
                        value.to_string_lossy().to_string(),
                    )
                })
            })
            .collect();
        assert!(envs.iter().any(|(k, v)| k == "LANG" && v == "en_US.utf8"));
        assert!(envs
            .iter()
            .any(|(k, v)| k == "LC_CTYPE" && v == "en_US.utf8"));
        assert!(envs.iter().any(|(k, v)| k == "LC_ALL" && v == "en_US.utf8"));
    }
}
