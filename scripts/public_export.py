#!/usr/bin/env python3
"""Build a clean public-source candidate from one explicit commit (IK-12).

    python3 scripts/public_export.py --rev <commit> --out <empty dir> [--config export/allowlist.json]

The export is a verbatim subset of the commit's tracked files: nothing is rewritten, so the bytes
that are qualified are the bytes that are published. Files are excluded by explicit rule with a
reason. Every remaining file is scanned for private-workflow references, secrets and absolute
user paths; a finding fails the export unless the config acknowledges that exact
(path, pattern) with a reason an owner can review. `EXPORT_MANIFEST.json` records the origin
commit, the allowlist digest, every exported file's sha256, every exclusion and every
acknowledgement, so the origin-to-export map is reproducible. This tool does not publish, create
a repository, or change any visibility.
"""

from __future__ import annotations

import argparse
import fnmatch
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[1]
DEFAULT_CONFIG = REPO / "export" / "allowlist.json"


def git(repo: Path, *args: str) -> bytes:
    return subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True).stdout


def tracked_files(repo: Path, rev: str) -> list[str]:
    raw = git(repo, "ls-tree", "-r", "--name-only", "-z", rev)
    return [p for p in raw.decode().split("\0") if p]


def exclusion_reason(path: str, config: dict[str, Any]) -> str | None:
    for rule in config["exclude"]:
        if fnmatch.fnmatchcase(path, rule["glob"]):
            return rule["reason"]
    return None


def scan_blob(path: str, data: bytes, config: dict[str, Any]) -> list[dict[str, str]]:
    if b"\0" in data:
        return []
    text = data.decode("utf-8", errors="replace")
    found = []
    for pattern in config["patterns"]:
        match = re.search(pattern["regex"], text)
        if match:
            found.append({"path": path, "pattern": pattern["id"], "kind": pattern["kind"], "sample": match.group(0)[:40]})
    return found


def is_acknowledged(finding: dict[str, str], config: dict[str, Any]) -> bool:
    if finding["kind"] == "secret":
        return False
    return any(a["path"] == finding["path"] and a["pattern"] == finding["pattern"] for a in config["acknowledged"])


def config_problems(config: dict[str, Any]) -> list[str]:
    problems = [f"acknowledgement {a['path']}:{a['pattern']} has no reason" for a in config["acknowledged"] if not a.get("reason")]
    problems += [f"exclusion {r['glob']} has no reason" for r in config["exclude"] if not r.get("reason")]
    return problems


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def select(repo: Path, rev: str, config: dict[str, Any]) -> tuple[dict[str, bytes], list[dict[str, str]], list[dict[str, str]]]:
    kept: dict[str, bytes] = {}
    excluded: list[dict[str, str]] = []
    findings: list[dict[str, str]] = []
    for path in tracked_files(repo, rev):
        reason = exclusion_reason(path, config)
        if reason:
            excluded.append({"path": path, "reason": reason})
            continue
        data = git(repo, "show", f"{rev}:{path}")
        kept[path] = data
        findings += scan_blob(path, data, config)
    return kept, excluded, findings


def build_manifest(repo: Path, rev: str, config_bytes: bytes, kept: dict[str, bytes], excluded: list[dict[str, str]], findings: list[dict[str, str]], config: dict[str, Any]) -> dict[str, Any]:
    return {
        "schema": "ikaros.public-export-manifest.v1",
        "origin_commit": git(repo, "rev-parse", rev).decode().strip(),
        "allowlist_sha256": digest(config_bytes),
        "files": {path: digest(data) for path, data in sorted(kept.items())},
        "excluded": excluded,
        "acknowledged": [f for f in findings if is_acknowledged(f, config)],
    }


def export(repo: Path, rev: str, out: Path, config_path: Path) -> tuple[dict[str, Any], list[dict[str, str]]]:
    config_bytes = config_path.read_bytes()
    config = json.loads(config_bytes)
    problems = config_problems(config)
    if problems:
        raise SystemExit("\n".join(problems))
    kept, excluded, findings = select(repo, rev, config)
    blocking = [f for f in findings if not is_acknowledged(f, config)]
    manifest = build_manifest(repo, rev, config_bytes, kept, excluded, findings, config)
    if blocking:
        return manifest, blocking
    write_tree(out, kept, manifest)
    return manifest, []


def write_tree(out: Path, kept: dict[str, bytes], manifest: dict[str, Any]) -> None:
    out.mkdir(parents=True, exist_ok=True)
    if any(out.iterdir()):
        raise SystemExit(f"{out} is not empty")
    for path, data in kept.items():
        target = out / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        mode = 0o755 if data.startswith(b"#!") else 0o644
        target.chmod(mode)
    (out / "EXPORT_MANIFEST.json").write_text(json.dumps(manifest, indent=2) + "\n")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--rev", required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    args = parser.parse_args()
    manifest, blocking = export(REPO, args.rev, args.out, args.config)
    for finding in blocking:
        print(f"BLOCKED {finding['path']}: {finding['pattern']} ({finding['kind']}) {finding['sample']!r}", file=sys.stderr)
    if blocking:
        return 1
    print(f"exported {len(manifest['files'])} files from {manifest['origin_commit']} to {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
