#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

TARGET_TRIPLE="$(rustc -vV | awk -F': ' '/^host: /{print $2}')"
if [[ -z "${TARGET_TRIPLE}" ]]; then
  echo "Failed to detect Rust target triple."
  exit 1
fi

cd "${REPO_ROOT}"
cargo build --release --manifest-path "${REPO_ROOT}/Cargo.toml" \
  -p console

cargo build --release --manifest-path "${REPO_ROOT}/Cargo.toml" \
  -p octovalve-proxy

BIN_DIR="${REPO_ROOT}/target/release"
SUFFIX="-${TARGET_TRIPLE}"
EXT=""
if [[ "${TARGET_TRIPLE}" == *windows* ]]; then
  EXT=".exe"
fi

for bin in console octovalve-proxy; do
  src="${BIN_DIR}/${bin}${EXT}"
  dst="${BIN_DIR}/${bin}${SUFFIX}${EXT}"
  if [[ ! -f "${src}" ]]; then
    echo "Missing sidecar binary: ${src}"
    exit 1
  fi
  install -m 0755 "${src}" "${dst}"
done
