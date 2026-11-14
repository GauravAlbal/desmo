#!/usr/bin/env python3
"""Pack the Node runtime package and one platform package from a built `ik` binary (IK-11).

    python3 scripts/pack_npm.py --binary target/release/ik --target darwin-arm64 --out dist/npm

Stages copies (the checkout is not modified), omits the runtime package's optionalDependencies so npm
never asks a registry about platform packages (the platform package is installed explicitly from
its tarball), puts the binary in the platform package, runs `npm pack`, and
writes `npm-manifest.json` with the sha256 of every tarball, the versions, and whether license
text files were present. The binary is never built here: the caller passes the exact bytes to ship.
Requires `typescript/dist` to be built (`npx tsc -p typescript`).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[1]
TS = REPO / "typescript"
PLATFORMS = ["darwin-arm64", "darwin-x64", "linux-arm64", "linux-x64"]
LICENSE_FILES = ["LICENSE", "LICENSE-MIT", "LICENSE-APACHE"]


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def stage_runtime(stage: Path) -> None:
    stage.mkdir(parents=True)
    shutil.copytree(TS / "dist", stage / "dist", ignore=shutil.ignore_patterns("*.test.js", "*.test.d.ts", "*.map"))
    shutil.copytree(TS / "assets", stage / "assets")
    shutil.copy(TS / "README.md", stage / "README.md")
    skill = REPO / "skills" / "desmo"
    if not (skill / "SKILL.md").is_file():
        raise SystemExit(f"packaged skill missing: {skill}")
    shutil.copytree(skill, stage / "skills" / "desmo")
    for name in LICENSE_FILES:
        if (REPO / name).exists():
            shutil.copy(REPO / name, stage / name)
    package = json.loads((TS / "package.json").read_text())
    package["files"] = ["dist", "README.md", "assets", "skills", *[n for n in LICENSE_FILES if (REPO / n).exists()]]
    package.pop("optionalDependencies", None)
    package.pop("devDependencies", None)
    package.pop("scripts", None)
    (stage / "package.json").write_text(json.dumps(package, indent=2) + "\n")


def stage_platform(stage: Path, target: str, binary: Path) -> None:
    shutil.copytree(TS / "platform" / target, stage)
    (stage / "bin").mkdir()
    shutil.copy(binary, stage / "bin" / "ik")
    (stage / "bin" / "ik").chmod(0o755)
    for name in LICENSE_FILES:
        if (REPO / name).exists():
            shutil.copy(REPO / name, stage / name)


def npm_pack(stage: Path, out: Path) -> Path:
    done = subprocess.run(["npm", "pack", "--silent", "--pack-destination", str(out)], cwd=stage, check=True, capture_output=True, text=True)
    return out / done.stdout.strip().splitlines()[-1]


def pack(binary: Path, target: str, out: Path) -> dict[str, Any]:
    if target not in PLATFORMS:
        raise SystemExit(f"unknown target {target}")
    version = json.loads((TS / "package.json").read_text())["version"]
    platform_version = json.loads((TS / "platform" / target / "package.json").read_text())["version"]
    if platform_version != version:
        raise SystemExit(f"platform package {platform_version} != runtime package {version}")
    out.mkdir(parents=True, exist_ok=True)
    work = out / ".stage"
    shutil.rmtree(work, ignore_errors=True)
    stage_runtime(work / "runtime")
    stage_platform(work / "platform", target, binary)
    runtime_tgz, platform_tgz = npm_pack(work / "runtime", out), npm_pack(work / "platform", out)
    shutil.rmtree(work)
    return {
        "schema": "ikaros.npm-manifest.v1",
        "version": version,
        "target": target,
        "binary_sha256": sha256(binary),
        "tarballs": {runtime_tgz.name: sha256(runtime_tgz), platform_tgz.name: sha256(platform_tgz)},
        "license_files_present": [n for n in LICENSE_FILES if (REPO / n).exists()],
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    manifest = pack(args.binary.resolve(), args.target, args.out.resolve())
    (args.out / "npm-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    sums = "".join(f"{digest}  {name}\n" for name, digest in sorted(manifest["tarballs"].items()))
    (args.out / "SHA256SUMS").write_text(sums)
    print(json.dumps(manifest, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
