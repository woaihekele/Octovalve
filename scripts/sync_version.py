#!/usr/bin/env python3
from __future__ import annotations

import pathlib
import re
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]


def load_workspace_version() -> str:
  cargo_toml = ROOT / "Cargo.toml"
  text = cargo_toml.read_text()
  section_match = re.search(
    r'^\[workspace\.package\]\s*$([\s\S]*?)(?:^\[|\Z)',
    text,
    re.MULTILINE,
  )
  if not section_match:
    raise RuntimeError("Missing [workspace.package] section in Cargo.toml")
  section = section_match.group(1)
  version_match = re.search(r'^\s*version\s*=\s*"([^"]+)"', section, re.MULTILINE)
  if not version_match:
    raise RuntimeError("Missing [workspace.package].version in Cargo.toml")
  return version_match.group(1)


def replace_pattern(path: pathlib.Path, pattern: re.Pattern[str], replacement: str, count: int = 1) -> None:
  text = path.read_text()
  updated, changed = pattern.subn(replacement, text, count=count)
  if changed == 0:
    raise RuntimeError(f"Pattern not found in {path}")
  path.write_text(updated)

def maybe_replace_pattern(path: pathlib.Path, pattern: re.Pattern[str], replacement: str, count: int = 1) -> None:
  if not path.exists():
    return
  replace_pattern(path, pattern, replacement, count=count)


def main() -> int:
  version = load_workspace_version()

  replace_pattern(
    ROOT / "console-ui/package.json",
    re.compile(r'("version"\s*:\s*")([^"]+)(")'),
    rf'\g<1>{version}\g<3>',
  )
  replace_pattern(
    ROOT / "console-ui/src-tauri/tauri.conf.json",
    re.compile(r'("version"\s*:\s*")([^"]+)(")'),
    rf'\g<1>{version}\g<3>',
  )
  replace_pattern(
    ROOT / "console-ui/src-tauri/Cargo.toml",
    re.compile(r'^(version\s*=\s*")([^"]+)(")$', re.MULTILINE),
    rf'\g<1>{version}\g<3>',
  )
  replace_pattern(
    ROOT / "console-ui/package-lock.json",
    re.compile(r'("version"\s*:\s*")([^"]+)(")'),
    rf'\g<1>{version}\g<3>',
  )
  replace_pattern(
    ROOT / "console-ui/package-lock.json",
    re.compile(r'("packages"\s*:\s*\{\s*""\s*:\s*\{[\s\S]*?"version"\s*:\s*")([^"]+)(")', re.MULTILINE),
    rf'\g<1>{version}\g<3>',
  )
  maybe_replace_pattern(
    ROOT / "docs/acp-client-integration.md",
    re.compile(r'("version"\s*:\s*")(\d+\.\d+\.\d+)(")'),
    rf'\g<1>{version}\g<3>',
    count=0,
  )

  return 0


if __name__ == "__main__":
  sys.exit(main())
