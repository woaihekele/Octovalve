#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

SRC_TARGET_DIR="${CARGO_TARGET_DIR:-${REPO_ROOT}/target}"
if [[ "${SRC_TARGET_DIR}" != /* ]]; then
  SRC_TARGET_DIR="${REPO_ROOT}/${SRC_TARGET_DIR}"
fi

# `tauri dev` typically builds the app binary into console-ui/src-tauri/target/debug/.
# `sidecar("...")` resolves the program path relative to the current executable directory
# (see tauri-plugin-shell `relative_command_path`), so we sync rebuilt binaries there.
TAURI_TARGET_DIR="${TAURI_CARGO_TARGET_DIR:-${REPO_ROOT}/console-ui/src-tauri/target}"

PROFILE="${OCTOVALVE_SIDECAR_PROFILE:-debug}"
SRC_DIR="${SRC_TARGET_DIR}/${PROFILE}"
DST_DIR="${TAURI_TARGET_DIR}/${PROFILE}"

EXT=""
if [[ "$(uname -s)" == MINGW* || "$(uname -s)" == MSYS* || "$(uname -s)" == CYGWIN* ]]; then
  EXT=".exe"
fi

mkdir -p "${DST_DIR}"

copy_bin() {
  local name="$1"
  local src="${SRC_DIR}/${name}${EXT}"
  local dst="${DST_DIR}/${name}${EXT}"
  local tmp="${dst}.tmp.$$"

  if [[ ! -f "${src}" ]]; then
    echo "Missing built sidecar: ${src}" >&2
    exit 1
  fi

  # Atomic replace avoids partial reads (and reduces watcher churn).
  cp -f "${src}" "${tmp}"
  chmod 0755 "${tmp}" 2>/dev/null || true
  mv -f "${tmp}" "${dst}"
}

copy_bin "octovalve-console"
copy_bin "octovalve-proxy"

echo "Synced sidecars to: ${DST_DIR}"
