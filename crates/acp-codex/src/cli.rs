use std::env;

use anyhow::{anyhow, Result};

#[derive(Debug, Clone)]
pub struct CliConfig {
    pub approval_policy: Option<String>,
    pub sandbox_mode: Option<String>,
    pub app_server_args: Vec<String>,
}

impl CliConfig {
    pub fn parse() -> Result<Self> {
        let args = env::args().skip(1).collect();
        Self::parse_from(args)
    }

    pub fn parse_from(args: Vec<String>) -> Result<Self> {
        let mut approval_policy = None;
        let mut sandbox_mode = None;
        let mut app_server_args = Vec::new();
        let mut args = args.into_iter().peekable();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--approval-policy" | "--approval_policy" => {
                    let value = args
                        .next()
                        .ok_or_else(|| anyhow!("--approval-policy 缺少值"))?;
                    approval_policy = Some(value.replace('_', "-"));
                }
                "--sandbox-mode" | "--sandbox_mode" => {
                    let value = args
                        .next()
                        .ok_or_else(|| anyhow!("--sandbox-mode 缺少值"))?;
                    sandbox_mode = Some(value.replace('_', "-"));
                }
                "-c" | "--config" => {
                    let value = args.next().ok_or_else(|| anyhow!("-c 缺少配置值"))?;
                    Self::apply_config_override(&value, &mut approval_policy, &mut sandbox_mode);
                    app_server_args.push(arg);
                    app_server_args.push(value);
                }
                _ => {
                    app_server_args.push(arg);
                }
            }
        }

        Ok(Self {
            approval_policy,
            sandbox_mode,
            app_server_args,
        })
    }

    fn apply_config_override(
        value: &str,
        approval_policy: &mut Option<String>,
        sandbox_mode: &mut Option<String>,
    ) {
        let (key, raw_value) = match value.split_once('=') {
            Some(pair) => pair,
            None => return,
        };
        let normalized_value = raw_value.trim().replace('_', "-");
        match key.trim() {
            "approval_policy" if approval_policy.is_none() => {
                *approval_policy = Some(normalized_value);
            }
            "sandbox_mode" if sandbox_mode.is_none() => {
                *sandbox_mode = Some(normalized_value);
            }
            _ => {}
        }
    }
}
