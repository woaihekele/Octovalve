#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/build_release.sh [--bin-dir [PATH]] [--no-config] [--with-local-proxy-config]

Options:
  --bin-dir [PATH]            Destination directory for release binaries (defaults to current dir)
  --no-config                 Do not copy config/config.toml
  --with-local-proxy-config   Copy config/local-proxy-config.toml if it exists
EOF
}

START_PWD="$PWD"
BIN_DIR="$START_PWD"
COPY_CONFIG=1
COPY_LOCAL_PROXY_CONFIG=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bin-dir)
      if [[ $# -ge 2 && ! "${2:-}" =~ ^- ]]; then
        BIN_DIR="$2"
        shift 2
      else
        BIN_DIR="$START_PWD"
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

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

cargo build --release

mkdir -p "$BIN_DIR"
install -m 0755 target/release/remote-broker "$BIN_DIR/remote-broker"
install -m 0755 target/release/local-proxy "$BIN_DIR/local-proxy"

if [[ "$COPY_CONFIG" -eq 1 ]]; then
  if [[ -f config/config.toml ]]; then
    mkdir -p "$BIN_DIR/config"
    install -m 0644 config/config.toml "$BIN_DIR/config/config.toml"
  else
    echo "config/config.toml not found; skipping."
  fi
fi

if [[ "$COPY_LOCAL_PROXY_CONFIG" -eq 1 ]]; then
  if [[ -f config/local-proxy-config.toml ]]; then
    mkdir -p "$BIN_DIR/config"
    install -m 0644 config/local-proxy-config.toml "$BIN_DIR/config/local-proxy-config.toml"
  else
    echo "config/local-proxy-config.toml not found; skipping."
  fi
fi
