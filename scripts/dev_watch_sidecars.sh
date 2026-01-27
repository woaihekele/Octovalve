#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

if ! cargo watch --version >/dev/null 2>&1; then
  echo "Missing 'cargo watch'. Install it with: cargo install cargo-watch" >&2
  exit 1
fi

cd "${REPO_ROOT}"

# 监听 sidecar 相关 crate；Tauri 后端本身由 `tauri dev` 负责重编译。
cargo watch \
  -w crates/console \
  -w crates/local-proxy \
  -w crates/protocol \
  -w Cargo.toml \
  -w Cargo.lock \
  -x "build -p console -p octovalve-proxy" \
  -s "bash scripts/dev_sync_tauri_sidecars.sh"

