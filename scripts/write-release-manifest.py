#!/usr/bin/env python3
"""Build SHA256SUMS + schema-3 release-manifest.json in a dist directory."""

from __future__ import annotations

import hashlib
import json
import os
import pathlib
import re
import sys

TARGET_BY_ASSET = {
    "ik-darwin-arm64.tar.gz": "aarch64-apple-darwin",
    "ik-darwin-amd64.tar.gz": "x86_64-apple-darwin",
    "ik-linux-amd64.tar.gz": "x86_64-unknown-linux-gnu",
    "ik-linux-arm64.tar.gz": "aarch64-unknown-linux-gnu",
}

REPO = pathlib.Path(__file__).resolve().parent.parent


def sha256(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    h.update(path.read_bytes())
    return h.hexdigest()


def first_match(path: pathlib.Path, pattern: str) -> str:
    match = re.search(pattern, path.read_text(), re.MULTILINE)
    if match is None:
        raise SystemExit(f"{path}: missing {pattern}")
    return match.group(1)


def locked_versions() -> tuple[str, str, str]:
    cargo = first_match(REPO / "Cargo.toml", r'^version = "([^"]+)"')
    runtime = first_match(
        REPO / "src" / "protocol.rs",
        r'^pub const RUNTIME_VERSION: &str = "([^"]+)";',
    )
    python = first_match(REPO / "python" / "pyproject.toml", r'^version = "([^"]+)"')
    go = first_match(
        REPO / "go" / "transport.go",
        r'runtimeVersionDefault = "([^"]+)"',
    )
    if len({cargo, runtime, python, go}) != 1:
        raise SystemExit(
            "package/protocol/python/go versions drifted: "
            f"cargo={cargo} runtime={runtime} python={python} go={go}"
        )
    return cargo, runtime, python


def main() -> None:
    cargo, runtime, python = locked_versions()
    root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "dist")
    version = os.environ.get("GITHUB_REF_NAME") or os.environ.get("IK_VERSION") or cargo
    source = os.environ.get("GITHUB_SHA") or os.environ.get("IK_SOURCE_SHA") or ""
    tag_version = version.lstrip("v") if version != "unreleased" else version
    if tag_version not in {cargo, "unreleased"}:
        raise SystemExit(
            f"release tag {version} does not match locked package version {cargo}"
        )
    artifacts = []
    sums = []
    for path in sorted(root.iterdir()):
        if not path.is_file() or path.name in {
            "SHA256SUMS",
            "release-manifest.json",
            "source-sha.txt",
            "version.txt",
        }:
            continue
        digest = sha256(path)
        sums.append(f"{digest}  {path.name}")
        entry = {"file": path.name, "sha256": digest}
        if path.name in TARGET_BY_ASSET:
            entry["target"] = TARGET_BY_ASSET[path.name]
        artifacts.append(entry)
    (root / "SHA256SUMS").write_text("\n".join(sums) + ("\n" if sums else ""))
    manifest = {
        "product": "Ikaros",
        "repository": "invariant-kernel",
        "version": tag_version,
        "tag": version if version.startswith("v") else (f"v{version}" if version != "unreleased" else version),
        "source_sha": source,
        "protocol_major": 1,
        "runtime_version": runtime,
        "minimum_supported_protocol": 1,
        "schema_version": 4,
        "python_client_version": python,
        "go_client_source_revision": source,
        "artifacts": artifacts,
    }
    (root / "release-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    if source:
        (root / "source-sha.txt").write_text(source + "\n")
    (root / "version.txt").write_text(manifest["tag"] + "\n")
    print(json.dumps(manifest, indent=2))


if __name__ == "__main__":
    main()
