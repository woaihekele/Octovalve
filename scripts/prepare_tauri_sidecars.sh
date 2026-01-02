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

rm -f "${BIN_DIR}/remote-broker${EXT}" "${BIN_DIR}/remote-broker-${SUFFIX}${EXT}"
rm -rf "${REPO_ROOT}/console-ui/src-tauri/target/release/remote-broker"

prepare_linux_broker() {
  local target_triple="$1"
  local arch_label="$2"
  local env_var="$3"
  local res_dir="${REPO_ROOT}/console-ui/src-tauri/remote-broker/${arch_label}"
  local res_path="${res_dir}/remote-broker"

  if [[ -n "${!env_var:-}" ]]; then
    mkdir -p "${res_dir}"
    install -m 0755 "${!env_var}" "${res_path}"
    return
  fi

  if ! command -v cargo-zigbuild >/dev/null 2>&1; then
    echo "cargo-zigbuild is required to build ${arch_label} broker. Install it or set ${env_var}."
    exit 1
  fi
  if ! command -v zig >/dev/null 2>&1; then
    echo "zig is required to build ${arch_label} broker. Install it or set ${env_var}."
    exit 1
  fi

  cargo zigbuild --release --manifest-path "${REPO_ROOT}/Cargo.toml" \
    -p remote-broker --target "${target_triple}"
  local built="${REPO_ROOT}/target/${target_triple}/release/remote-broker${EXT}"
  if [[ ! -f "${built}" ]]; then
    echo "Missing built broker: ${built}"
    exit 1
  fi
  mkdir -p "${res_dir}"
  install -m 0755 "${built}" "${res_path}"
}

prepare_linux_broker "x86_64-unknown-linux-musl" "linux-x86_64" "OCTOVALVE_LINUX_BROKER_X86_64"
