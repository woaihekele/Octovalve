#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

usage() {
  cat <<'USAGE'
Usage: prepare_tauri_sidecars.sh [--target <triple>]

Targets:
  - default: host triple from rustc
  - universal-apple-darwin: build arm64 + x86_64 and lipo into a universal binary
USAGE
}

TARGET_TRIPLE=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --target|-t)
      if [[ $# -lt 2 ]]; then
        echo "Missing value for --target"
        usage
        exit 1
      fi
      TARGET_TRIPLE="$2"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1"
      usage
      exit 1
      ;;
  esac
done

HOST_TRIPLE="$(rustc -vV | awk -F': ' '/^host: /{print $2}')"
if [[ -z "${HOST_TRIPLE}" ]]; then
  echo "Failed to detect Rust target triple."
  exit 1
fi

if [[ -z "${TARGET_TRIPLE}" ]]; then
  TARGET_TRIPLE="${OCTOVALVE_SIDECAR_TARGET:-${TAURI_TARGET_TRIPLE:-${TAURI_BUILD_TARGET:-${TAURI_TARGET:-${HOST_TRIPLE}}}}}"
fi

cd "${REPO_ROOT}"

TARGET_DIR="${CARGO_TARGET_DIR:-${REPO_ROOT}/target}"
if [[ "${TARGET_DIR}" != /* ]]; then
  TARGET_DIR="${REPO_ROOT}/${TARGET_DIR}"
fi

BIN_DIR="${TARGET_DIR}/release"
EXT=""
if [[ "${TARGET_TRIPLE}" == *windows* ]]; then
  EXT=".exe"
fi

build_sidecars_for_target() {
  local target_triple="$1"
  local force_target="${2:-false}"
  local src_dir="${TARGET_DIR}/release"
  local cargo_target_args=()

  if [[ "${force_target}" == "true" || "${target_triple}" != "${HOST_TRIPLE}" ]]; then
    cargo_target_args=(--target "${target_triple}")
    src_dir="${TARGET_DIR}/${target_triple}/release"
  fi

  if [[ ${#cargo_target_args[@]} -gt 0 ]]; then
    cargo build --release --manifest-path "${REPO_ROOT}/Cargo.toml" \
      -p console "${cargo_target_args[@]}"

    cargo build --release --manifest-path "${REPO_ROOT}/Cargo.toml" \
      -p octovalve-proxy "${cargo_target_args[@]}"
  else
    cargo build --release --manifest-path "${REPO_ROOT}/Cargo.toml" \
      -p console

    cargo build --release --manifest-path "${REPO_ROOT}/Cargo.toml" \
      -p octovalve-proxy
  fi

  local suffix="-${target_triple}"
  for bin in octovalve-console octovalve-proxy; do
    local src="${src_dir}/${bin}${EXT}"
    local dst="${BIN_DIR}/${bin}${suffix}${EXT}"
    if [[ ! -f "${src}" ]]; then
      echo "Missing sidecar binary: ${src}"
      exit 1
    fi
    install -m 0755 "${src}" "${dst}"
  done
}

if [[ "${TARGET_TRIPLE}" == "universal-apple-darwin" ]]; then
  if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "universal-apple-darwin build requires macOS."
    exit 1
  fi
  if ! command -v lipo >/dev/null 2>&1; then
    echo "Missing lipo; install Xcode command line tools."
    exit 1
  fi

  for arch_target in aarch64-apple-darwin x86_64-apple-darwin; do
    build_sidecars_for_target "${arch_target}" "true"
  done

  for bin in octovalve-console octovalve-proxy; do
    src_arm="${TARGET_DIR}/aarch64-apple-darwin/release/${bin}${EXT}"
    src_x86="${TARGET_DIR}/x86_64-apple-darwin/release/${bin}${EXT}"
    dst="${BIN_DIR}/${bin}-universal-apple-darwin"

    if [[ ! -f "${src_arm}" ]]; then
      echo "Missing sidecar binary: ${src_arm}"
      exit 1
    fi
    if [[ ! -f "${src_x86}" ]]; then
      echo "Missing sidecar binary: ${src_x86}"
      exit 1
    fi

    lipo -create -output "${dst}" "${src_arm}" "${src_x86}"
    chmod 0755 "${dst}"
  done

  exit 0
fi

build_sidecars_for_target "${TARGET_TRIPLE}"
