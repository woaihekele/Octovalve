# Remote Command Broker

This repo provides a local MCP stdio proxy that forwards `run_command` requests to a remote
broker with manual approval in a TUI. The remote broker listens on 127.0.0.1 and is intended
to be exposed via an SSH tunnel.

## Components
- local-proxy: MCP stdio server that exposes `run_command` and forwards requests.
- remote-broker: TUI approval service that executes whitelisted commands.
- protocol: shared request/response types between the proxy and broker.

## Requirements
- Rust 1.88 (see `rust-toolchain.toml`).

## Quick Start
1) Create a `config.toml` on the remote host:

```toml
[whitelist]
allowed = ["ls", "tail", "/usr/bin/grep"]
arg_rules = { grep = "^[A-Za-z0-9_.-]+$" }

[limits]
timeout_secs = 30
max_output_bytes = 1048576
```

2) Run the remote broker on the remote host:

```bash
cargo run -p remote-broker -- \
  --listen-addr 127.0.0.1:9000 \
  --config config.toml \
  --audit-dir logs
```

3) Open an SSH tunnel from your local machine:

```bash
ssh -L 9000:127.0.0.1:9000 user@remote-host
```

4) Run the local proxy:

```bash
cargo run -p local-proxy -- --remote-addr 127.0.0.1:9000
```

5) Configure your MCP client to start `local-proxy` via stdio.

## CLI Options
remote-broker:
- `--listen-addr` (default: `127.0.0.1:9000`)
- `--config` (default: `config.toml`)
- `--audit-dir` (default: `logs`)

local-proxy:
- `--remote-addr` (default: `127.0.0.1:9000`)
- `--client-id` (default: `local-proxy`)
- `--timeout-ms` (default: `30000`)
- `--max-output-bytes` (default: `1048576`)

## Security Notes
- There is no built-in auth. Use SSH tunnels and keep the broker on loopback.
- Commands are validated against a whitelist and optional argument regex rules.
- The broker never uses `sh -c` and executes commands directly.
- Run the broker as a non-root user and review audit logs.

## Tests

```bash
cargo test
```

## Docs
- Implementation plan: `docs/implementation-plan.md`
