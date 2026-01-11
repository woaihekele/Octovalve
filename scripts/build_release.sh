#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/build_release.sh [--bin-dir [PATH]] [--no-config] [--with-local-proxy-config]

Options:
  --bin-dir [PATH]            Base directory for release outputs (defaults to current dir).
                              A new timestamped subdirectory is created for each run.
  --no-config                 Do not copy config/config.toml
  --with-local-proxy-config   Copy config/local-proxy-config.toml if it exists
USAGE
}

START_PWD="$PWD"
BASE_DIR="$START_PWD"
COPY_CONFIG=1
COPY_LOCAL_PROXY_CONFIG=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bin-dir)
      if [[ $# -ge 2 && ! "${2:-}" =~ ^- ]]; then
        BASE_DIR="$2"
        shift 2
      else
        BASE_DIR="$START_PWD"
        shift
      fi
      ;;
    --no-config)
      COPY_CONFIG=0
      shift
      ;;
    --with-local-proxy-config)
      COPY_LOCAL_PROXY_CONFIG=1
      shift
      ;;
    -h|--help)
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

if [[ "$BASE_DIR" != /* ]]; then
  BASE_DIR="$START_PWD/$BASE_DIR"
fi
BASE_DIR="${BASE_DIR%/}"
TIMESTAMP="$(date +%Y%m%d_%H%M%S)"
OUTPUT_DIR="$BASE_DIR/release_${TIMESTAMP}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

cargo build --release

mkdir -p "$OUTPUT_DIR"
install -m 0755 target/release/console "$OUTPUT_DIR/console"
install -m 0755 target/release/octovalve-proxy "$OUTPUT_DIR/octovalve-proxy"

if [[ "$COPY_CONFIG" -eq 1 ]]; then
  if [[ -f config/config.toml ]]; then
    mkdir -p "$OUTPUT_DIR/config"
    install -m 0644 config/config.toml "$OUTPUT_DIR/config/config.toml"
  else
    echo "config/config.toml not found; skipping."
  fi
fi

if [[ "$COPY_LOCAL_PROXY_CONFIG" -eq 1 ]]; then
  if [[ -f config/local-proxy-config.toml ]]; then
    mkdir -p "$OUTPUT_DIR/config"
    install -m 0644 config/local-proxy-config.toml "$OUTPUT_DIR/config/local-proxy-config.toml"
  else
    echo "config/local-proxy-config.toml not found; skipping."
  fi
fi

echo "Artifacts written to $OUTPUT_DIR"
