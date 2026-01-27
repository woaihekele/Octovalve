#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

WATCH_PID=""
WATCH_PGID=""

cleanup() {
  if [[ -n "${WATCH_PGID}" ]]; then
    kill -TERM "-${WATCH_PGID}" 2>/dev/null || true
  elif [[ -n "${WATCH_PID}" ]]; then
    kill -TERM "${WATCH_PID}" 2>/dev/null || true
  fi
}

trap cleanup EXIT INT TERM

echo "[dev] starting sidecar watcher..."
bash "${REPO_ROOT}/scripts/dev_watch_sidecars.sh" &
WATCH_PID="$!"
WATCH_PGID="$(ps -o pgid= "${WATCH_PID}" | tr -d ' ' || true)"

echo "[dev] starting tauri dev..."
export OCTOVALVE_DEV_KILL_STRAY_CONSOLE=1
npm -C "${REPO_ROOT}/console-ui" run tauri dev
