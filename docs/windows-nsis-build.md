# Windows NSIS Build (Tauri)

This document records the current Windows packaging flow for `console-ui`.

## One-Command Packaging (PowerShell)

From repo root:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\build_windows_gui.ps1
```

Optional (skip `npm install` if dependencies are already installed):

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\build_windows_gui.ps1 -SkipNpmInstall
```

## Prerequisites

- Rust toolchain (`rustc`, `cargo`) on Windows (`x86_64-pc-windows-msvc`)
- Node.js + npm
- NSIS (required by Tauri for `--bundles nsis`)

## Current Build Flow

`console-ui/src-tauri/tauri.conf.json` currently sets:

- `build.beforeBuildCommand = "bash ../scripts/prepare_tauri_sidecars.sh && npm run build"`

That means the default build assumes a `bash` executable exists on PATH.

### Path A: Git Bash available (recommended)

From `console-ui/`:

```powershell
npm.cmd install
npx.cmd tauri build --bundles nsis --config tauri.nsis.override.json
```

Create `tauri.nsis.override.json` first:

```json
{"bundle":{"active":true}}
```

### Path B: PowerShell only (no bash on PATH)

1. Build sidecar binaries in the workspace root:

```powershell
cargo build --release -p console -p octovalve-proxy
```

2. Ensure `octovalve-ssh-askpass` target-suffixed sidecar exists:

```powershell
Copy-Item -Force target\release\octovalve-ssh-askpass.exe target\release\octovalve-ssh-askpass-x86_64-pc-windows-msvc.exe
```

3. Build frontend assets in `console-ui/`:

```powershell
npm.cmd run build
```

4. Create an override config that bypasses `bash` and enables bundle output:

```json
{"bundle":{"active":true},"build":{"beforeBuildCommand":"npm run build"}}
```

5. Run Tauri build:

```powershell
npx.cmd tauri build --bundles nsis --config tauri.nsis.override.json
```

## Output

Successful NSIS output is generated at:

- `console-ui/src-tauri/target/release/bundle/nsis/Octovalve_0.1.4_x64-setup.exe`
