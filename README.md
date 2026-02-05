# Octovalve Local Approval Command Proxy

English | [中文](README.zh-CN.md)

Octovalve provides a local MCP stdio proxy plus a local approval/execution service. Your model connects only to the local stdio server (`octovalve-proxy`). The proxy forwards `run_command` requests to the local `console`, where a human approves/denies each request and the console executes the approved command on the target machine via SSH, then returns the results back to the MCP client.

## Components
- `octovalve-proxy`: MCP stdio server that exposes the `run_command` tool and forwards requests.
- `console`: local approval + execution service that tracks targets and runs commands via SSH.
- `protocol`: shared request/response types between local components.
- `console-ui`: optional local UI (Tauri + Vue).

## Requirements
- Rust 1.88 (see `rust-toolchain.toml`).

## Quick Start
1) Prepare the approval policy config (whitelist/limits): `config/config.toml`

```toml
auto_approve_allowed = true

[whitelist]
allowed = [
  "ls",
  "cat",
  "head",
  "tail",
  "sed",
  "rg",
  "grep",
  "wc",
  "sort",
  "uniq",
  "find",
  "ps",
  "uname",
  "df",
  "free",
]
denied = ["rm", "shutdown"]
arg_rules = { grep = "^[A-Za-z0-9_./-]+$" }

[limits]
timeout_secs = 30
max_output_bytes = 1048576
```

2) Prepare the target config (see `config/local-proxy-config.toml` for an example):

```toml
default_target = "example-target"

[defaults]
timeout_ms = 30000
max_output_bytes = 1048576
# terminal_locale = "zh_CN.UTF-8"
# ssh_args = ["-o", "ServerAliveInterval=30", "-o", "ServerAliveCountMax=3"]

[[targets]]
name = "example-target"
desc = "primary dev host"
ssh = "devops@192.168.2.162"
# ssh_password = "your password"
# tty = true
```

Note: `ssh` must include the username (`user@host`); it will not auto-fill a default user.

3) Start the console (local approval + SSH execution):

```bash
cargo run -p console -- \
  --config config/local-proxy-config.toml \
  --broker-config config/config.toml
```

By default, console listens on:
- HTTP/WS: `127.0.0.1:19309`
- command channel: `127.0.0.1:19310`

4) Start the local proxy:

```bash
cargo run -p octovalve-proxy -- --config config/local-proxy-config.toml
```

5) Point your MCP client to `octovalve-proxy` (stdio).

Example for Codex CLI (`~/.codex/config.toml`):

```toml
[mcp_servers.octovalve]
command = "~/octovalve/target/release/octovalve-proxy"
args = ["--config", "~/.octovalve/local-proxy-config.toml",
        "--client-id", "codex-1"]
env = { RUST_LOG = "info" }
```

During development, you can use `cargo run`:

```toml
[mcp_servers.octovalve]
command = "cargo"
args = ["run", "-p", "octovalve-proxy", "--",
        "--config", "~/.octovalve/local-proxy-config.toml",
        "--client-id", "codex-1"]
env = { RUST_LOG = "info" }
```

## `run_command` Parameters
- `command`: command string.
- `intent`: required; why you want to run this command (for auditing).
- `target`: required; target name (defined in `octovalve-proxy` config).
- `mode`: `shell` (runs via `/bin/bash -lc`).
- Optional: `cwd`, `timeout_ms`, `max_output_bytes`, `env`.

## Common Read-Only Commands (Recommended for Whitelist)
Search/locate:
- `rg -n "pattern" path`
- `rg --files -g "*.rs"`
- `grep -R -n "pattern" path`

Browse/inspect:
- `ls`, `ls -la`
- `cat`, `head -n 20`, `tail -n 20`
- `sed -n '1,120p' file`

Count/filter:
- `wc -l`, `sort`, `uniq -c`
- `find path -type f -name "*.rs"`

System/environment (read-only):
- `ps -ef`, `uname -a`, `df -h`, `free -m`

## `list_targets`
Returns the locally configured target list with fields like `name/desc/last_seen/ssh/status/last_error`.

## Console API (Optional)
- `GET /health`: health check
- `GET /targets`: target list (`name/desc/ssh/status/pending_count`)
- `GET /targets/:name/snapshot`: get a target snapshot
- `POST /targets/:name/approve` / `deny`: approve/deny
- `GET /ws`: WebSocket push
  - `targets_snapshot`: initial full targets snapshot
  - `target_updated`: single-target update

## Console UI (Tauri)
The optional desktop UI lives under `console-ui/` (Tauri + Vue 3).

Prerequisites:
- Node.js + npm

Dev/build:
```bash
cd console-ui
npm install

# Start Tauri dev mode (runs Vite first)
npm run tauri dev

# Recommended: one command for Tauri + sidecars (auto rebuild/sync/restart)
npm run dev:tauri

# Optional: auto build/sync sidecars (octovalve-console / octovalve-proxy)
# Run in another terminal; when sidecar code changes, it rebuilds and syncs into the Tauri dev output.
npm run dev:sidecars

# Build desktop app
npm run tauri:build:dmg

# macOS universal build (arm64 + x86_64)
# Requires Xcode Command Line Tools (for lipo)
rustup target add aarch64-apple-darwin x86_64-apple-darwin
npm run tauri:build:universal:dmg
```

Runtime notes:
- On startup, the app automatically launches `console` (sidecar).
- On first launch, it creates `~/.octovalve/local-proxy-config.toml.example`.
  - Copy to `local-proxy-config.toml`, edit it, then restart the app.
- `remote-broker-config.toml` is stored under the app config directory (approval policy).
  - Default macOS path: `~/Library/Application Support/com.octovalve.console/`

## Password Login
Prefer SSH keys. If you must use a password, configure `ssh_password` per target.
`console`/`octovalve-proxy` inject the password via a temporary `SSH_ASKPASS` script (`~/.octovalve/ssh-askpass.sh`) without requiring `sshpass`.
If the server requires keyboard-interactive/2FA, `SSH_ASKPASS` cannot complete the flow; use SSH keys or change the auth method.

## CLI Options
`octovalve-proxy`:
- `--config` (default: `config/local-proxy-config.toml`)
- `--client-id` (default: `octovalve-proxy`)
- `--command-addr` (default: `127.0.0.1:19310`)
- `--timeout-ms` (default: `30000`)
- `--max-output-bytes` (default: `1048576`)

`console`:
- `--config` (targets config; same format as `config/local-proxy-config.toml`)
- `--listen-addr` (default: `127.0.0.1:19309`)
- `--command-listen-addr` (default: `127.0.0.1:19310`)
- `--broker-config` (approval policy config; default: `config/config.toml`)
- `--local-audit-dir` (default: `~/.octovalve/logs/local`)
- `--log-to-stderr` (default: off)

## Security Notes
- No built-in authentication; keep console bound to `127.0.0.1`.
- SSH uses `BatchMode=yes` to avoid interactive prompts. If you want auto-accept on first connect, add `StrictHostKeyChecking=accept-new` to `ssh_args`.
- Only `shell` mode is supported (`/bin/bash -lc`).
- Run as a non-root user and monitor audit logs.

## Output Persistence
Each request writes full request/result metadata and outputs under the local audit directory (default: `~/.octovalve/logs/local/<target>`):
- `<id>.request.json`
- `<id>.result.json`
- `<id>.stdout`
- `<id>.stderr`

`request.json` includes `intent`, `mode`, `raw_command`, `pipeline`, etc.

## License
Licensed under the Apache License, Version 2.0. See `LICENSE`.
