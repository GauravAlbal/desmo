#!/usr/bin/env python3
"""Move every runtime/package/client version expectation from one version to another.

    python3 scripts/bump_version.py --from 0.4.1 --to 0.5.0
    python3 scripts/bump_version.py --from 0.4.1 --to 0.5.0 --check   # list stale files, exit 1

Desmo enforces strict runtime-version equality across the binary, the Python and Go clients, the
Node package and the conformance traces, so a version move touches all of them at once. Only the
files matched by TARGETS are rewritten; history (CHANGELOG, released-version docs, LATEST_RELEASE,
dist/) is never touched, and `Cargo.lock` is refreshed by cargo, not by this tool. Matches are
whole version tokens, so `0.4.14` is not `0.4.1`.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
TARGETS = [
    "Cargo.toml",
    "SOURCE_VERSION_POLICY",
    "install.sh",
    "src/protocol.rs",
    "tests/*.rs",
    "tests/fixtures/profile/golden/*.json",
    "go/*.go",
    "python/pyproject.toml",
    "python/README.md",
    "python/src/invariant_kernel/*.py",
    "python/tests/*.py",
    "scripts/release",
    "scripts/verify-release.py",
    "examples/publish-approval/scenarios.py",
    "typescript/package.json",
    "typescript/package-lock.json",
    "typescript/platform/*/package.json",
    "typescript/src/*.ts",
    "conformance/traces/*.json",
]


def token(version: str) -> re.Pattern[str]:
    return re.compile(rf"(?<![\d.]){re.escape(version)}(?![\d])")


def targets(root: Path) -> list[Path]:
    seen: dict[Path, None] = {}
    for pattern in TARGETS:
        for path in sorted(root.glob(pattern)):
            if path.is_file():
                seen[path] = None
    return list(seen)


def stale(root: Path, old: str) -> list[Path]:
    pattern = token(old)
    return [p for p in targets(root) if pattern.search(p.read_text(encoding="utf-8"))]


def rewrite(root: Path, old: str, new: str) -> list[Path]:
    pattern = token(old)
    changed = []
    for path in stale(root, old):
        path.write_text(pattern.sub(new, path.read_text(encoding="utf-8")), encoding="utf-8")
        changed.append(path)
    return changed


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--from", dest="old", required=True)
    parser.add_argument("--to", dest="new", required=True)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    if args.old == args.new:
        raise SystemExit("--from and --to are identical")
    if args.check:
        left = stale(REPO, args.old)
        for path in left:
            print(f"stale {args.old}: {path.relative_to(REPO)}", file=sys.stderr)
        return 1 if left else 0
    for path in rewrite(REPO, args.old, args.new):
        print(f"{path.relative_to(REPO)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
