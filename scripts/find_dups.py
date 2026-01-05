#!/usr/bin/env python3
"""Find duplicate code blocks in source files.

Defaults target Rust source files and report top duplicates.
"""

from __future__ import annotations

import argparse
import hashlib
import re
from pathlib import Path
from typing import Iterable, List, Tuple

DEFAULT_EXCLUDE_DIRS = [".git", "target", "node_modules", "dist", "build"]


def normalize_line(line: str) -> str:
    return re.sub(r"\s+", " ", line.strip())


def is_significant(line: str) -> bool:
    return bool(re.search(r"[A-Za-z0-9_]", line))


def iter_source_files(root: Path, exts: List[str], exclude_dirs: List[str]) -> Iterable[Path]:
    exclude_set = set(exclude_dirs)
    for path in root.rglob("*"):
        if not path.is_file():
            continue
        if path.suffix.lstrip(".") not in exts:
            continue
        if exclude_set.intersection(path.parts):
            continue
        yield path


def load_files(paths: Iterable[Path]) -> List[Tuple[Path, List[str]]]:
    file_lines: List[Tuple[Path, List[str]]] = []
    for path in paths:
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except Exception:
            continue
        lines = text.splitlines()
        norms = [normalize_line(line) for line in lines]
        file_lines.append((path, norms))
    return file_lines


def find_duplicates(
    file_lines: List[Tuple[Path, List[str]]],
    min_lines: int,
    min_significant: int,
    max_pairs_per_hash: int,
    include_same_file: bool,
) -> List[Tuple[int, int, int, int, int]]:
    windows = {}
    for file_idx, (_, norms) in enumerate(file_lines):
        total = len(norms)
        if total < min_lines:
            continue
        sig_flags = [1 if is_significant(line) else 0 for line in norms]
        sig_prefix = [0]
        for flag in sig_flags:
            sig_prefix.append(sig_prefix[-1] + flag)
        for start in range(0, total - min_lines + 1):
            sig_count = sig_prefix[start + min_lines] - sig_prefix[start]
            if sig_count < min_significant:
                continue
            window = "\n".join(norms[start : start + min_lines])
            digest = hashlib.sha1(window.encode("utf-8")).hexdigest()
            windows.setdefault(digest, []).append((file_idx, start))

    matches: List[Tuple[int, int, int, int, int]] = []
    seen = set()
    for occs in windows.values():
        if len(occs) < 2:
            continue
        if len(occs) > max_pairs_per_hash:
            occs = occs[:max_pairs_per_hash]
        for i in range(len(occs)):
            for j in range(i + 1, len(occs)):
                a_idx, a_start = occs[i]
                b_idx, b_start = occs[j]
                if not include_same_file and a_idx == b_idx:
                    continue
                if a_idx == b_idx and abs(a_start - b_start) < min_lines:
                    continue
                _, a_lines = file_lines[a_idx]
                _, b_lines = file_lines[b_idx]
                start_a, start_b = a_start, b_start
                while (
                    start_a > 0
                    and start_b > 0
                    and a_lines[start_a - 1] == b_lines[start_b - 1]
                ):
                    start_a -= 1
                    start_b -= 1
                end_a = a_start + min_lines
                end_b = b_start + min_lines
                while (
                    end_a < len(a_lines)
                    and end_b < len(b_lines)
                    and a_lines[end_a] == b_lines[end_b]
                ):
                    end_a += 1
                    end_b += 1
                length = end_a - start_a
                key = (a_idx, start_a, b_idx, start_b, length)
                if key in seen:
                    continue
                seen.add(key)
                matches.append((length, a_idx, start_a, b_idx, start_b))
    matches.sort(reverse=True)
    return matches


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Find duplicate code blocks.")
    parser.add_argument(
        "--root",
        default=".",
        help="Root directory to scan (default: .)",
    )
    parser.add_argument(
        "--ext",
        action="append",
        default=["rs"],
        help="File extension to include (default: rs). Can be repeated.",
    )
    parser.add_argument(
        "--min-lines",
        type=int,
        default=12,
        help="Minimum number of lines in a duplicate block (default: 12).",
    )
    parser.add_argument(
        "--min-significant",
        type=int,
        default=8,
        help="Minimum significant lines per window (default: 8).",
    )
    parser.add_argument(
        "--max-pairs-per-hash",
        type=int,
        default=20,
        help="Max pair comparisons per hash bucket (default: 20).",
    )
    parser.add_argument(
        "--exclude-dir",
        action="append",
        default=DEFAULT_EXCLUDE_DIRS,
        help="Directory name to exclude. Can be repeated.",
    )
    parser.add_argument(
        "--top",
        type=int,
        default=30,
        help="Number of results to show (default: 30).",
    )
    parser.add_argument(
        "--exclude-same-file",
        action="store_true",
        help="Exclude duplicates within the same file.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root = Path(args.root)
    files = list(iter_source_files(root, args.ext, args.exclude_dir))
    files.sort()
    file_lines = load_files(files)
    matches = find_duplicates(
        file_lines,
        min_lines=args.min_lines,
        min_significant=args.min_significant,
        max_pairs_per_hash=args.max_pairs_per_hash,
        include_same_file=not args.exclude_same_file,
    )
    print(
        f"Scanned {len(files)} files. Found {len(matches)} duplicate blocks "
        f"(>= {args.min_lines} lines)."
    )
    if not matches:
        return 0
    print("Top duplicates:")
    for length, a_idx, a_start, b_idx, b_start in matches[: args.top]:
        a_path = file_lines[a_idx][0].relative_to(root)
        b_path = file_lines[b_idx][0].relative_to(root)
        print(
            f"- {length} lines: {a_path}:{a_start + 1} <-> {b_path}:{b_start + 1}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
