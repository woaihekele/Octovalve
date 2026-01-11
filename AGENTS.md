# Repository Guidelines

## Project Structure & Module Organization
- Workspace layout lives under `crates/`:
  - `crates/console`: local approval + SSH execution service (HTTP/WS + command channel).
  - `crates/local-proxy` (bin: `octovalve-proxy`): MCP stdio server that forwards `run_command` to console.
  - `crates/protocol`: shared request/response types.
  - `crates/acp-codex`: ACP compatibility layer for Codex app-server.
- Docs and plans live in `docs/` (design notes).
- Config files live in `config/`.
  - `config/config.toml` is the reference whitelist/limits used by console.

## Build, Test, and Development Commands
- Build all: `cargo build`
- Run tests: `cargo test`
- Format (required before commit): `cargo fmt`
- Run services locally:
  - `cargo run -p console -- --config config/local-proxy-config.toml --broker-config config/config.toml`
  - `cargo run -p octovalve-proxy -- --config /path/to/config/local-proxy-config.toml`

## Coding Style & Naming Conventions
- Rust 2021 edition; format with `cargo fmt` before every commit.
- Prefer descriptive names; avoid abbreviations in public structs/functions.

## Testing Guidelines
- Tests use Rust’s built-in test framework (`#[test]`).
- Keep unit tests close to the module being tested (see `crates/*/src`).
- Run `cargo test` after behavioral changes.

## Commit & Pull Request Guidelines
- Commit message style follows `type: summary` (e.g., `feat: ...`, `fix: ...`, `docs: ...`, `chore: ...`).
- Format before commit (`cargo fmt`) and ensure tests pass when logic changes.
- If a change affects configs, update docs/examples in `README.md` or `docs/`.

## Security & Configuration Tips
- Do not commit or sync `config/local-proxy-config.toml` (contains private/local settings).
- Keep console listening on `127.0.0.1` and use local-proxy from the same machine.
- For search tasks, prefer `rg` / `rg --files` if available.
- When using `run_command`, ensure `cwd` exists on the remote target; a missing `cwd` causes spawn errors with empty stdout/stderr.
- `shell` mode executes via `/bin/bash -lc`.
