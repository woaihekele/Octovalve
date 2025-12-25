# Repository Guidelines

## Project Structure & Module Organization
- Workspace layout lives under `crates/`:
  - `crates/remote-broker`: TUI approval server (UI, service, policy, execution layers).
  - `crates/local-proxy`: MCP stdio server that forwards `run_command` to remote brokers.
  - `crates/protocol`: shared request/response types.
- Docs and plans live in `docs/` (example configs, design notes).
- Top-level `config.toml` is the reference whitelist/limits for `remote-broker`.

## Build, Test, and Development Commands
- Build all: `cargo build`
- Run tests: `cargo test`
- Format (required before commit): `cargo fmt`
- Run services locally:
  - `cargo run -p remote-broker -- --listen-addr 127.0.0.1:19307 --config config.toml --audit-dir logs`
  - `cargo run -p local-proxy -- --config /path/to/local-proxy-config.toml`

## Coding Style & Naming Conventions
- Rust 2021 edition; format with `cargo fmt` before every commit.
- Keep modules layered in `remote-broker` (`ui/`, `service/`, `policy/`, `execution/`, `shared/`).
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
- Do not commit or sync `docs/local-proxy-config.toml` (contains private/local settings).
- Keep `remote-broker` listening on `127.0.0.1` and access via SSH tunnels.
- For search tasks, prefer `rg` / `rg --files` if available.
