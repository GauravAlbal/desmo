#!/usr/bin/env python3
"""Qualify the packed Node and Python artifacts as an unrelated user would run them (IK-11).

    python3 scripts/qualify_artifacts.py --dist dist/v0.5.0 --out report.json [--node /path/to/node]

For this host's target only. The binary is extracted from the release archive; the npm tarballs are
packed from that exact binary (`scripts/pack_npm.py`); everything then runs in empty directories
outside the checkout with a scrubbed environment: no `IK_BINARY`, no `ik` on PATH, no Rust
toolchain, no source path. The Python wheel is installed offline into a fresh venv. Negative
controls prove the failure modes are loud. Linux and other targets are NOT covered unless this is
run on them; the report says which host it ran on.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import sqlite3
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
sys.path.insert(0, str(HERE))

import pack_npm  # noqa: E402

TARGETS = {("Darwin", "arm64"): "darwin-arm64", ("Darwin", "x86_64"): "darwin-x64", ("Linux", "aarch64"): "linux-arm64", ("Linux", "x86_64"): "linux-x64"}

OPERATION_PROBE = """
import { Kernel } from "@desmo/runtime";
import { fileURLToPath } from "node:url";
const bundled = fileURLToPath(new URL("../assets/verification_attempt.ik.toml", import.meta.resolve("@desmo/runtime")));
const kernel = new Kernel(process.argv[2]);
const machine = await kernel.machine(bundled);
const doc = await machine.create("a1");
const opId = kernel.newOperationId();
const started = await doc.execute({ name: "start" }, { operationId: opId });
const found = await machine.lookupOperation("a1", doc.capability, opId, { command: { name: "start" } });
const again = await doc.execute({ name: "start" }, { operationId: opId });
console.log(JSON.stringify({ state: started.namedState(), lookup: found.outcome, repeat: again.constructor.name }));
"""


def scrubbed_env(node_dir: Path, extra: dict[str, str] | None = None) -> dict[str, str]:
    env = {"HOME": os.environ["HOME"], "PATH": f"{node_dir}:/usr/bin:/bin", "TMPDIR": tempfile.gettempdir(), "LANG": "C"}
    env.update(extra or {})
    return env


def run(argv: list[str], cwd: Path, env: dict[str, str], check: bool = True) -> subprocess.CompletedProcess[str]:
    done = subprocess.run(argv, cwd=cwd, env=env, capture_output=True, text=True)
    if check and done.returncode != 0:
        raise SystemExit(f"FAILED {' '.join(argv)}\n{done.stdout}\n{done.stderr}")
    return done


def host_target() -> str:
    key = (platform.system(), platform.machine())
    if key not in TARGETS:
        raise SystemExit(f"unsupported host {key}")
    return TARGETS[key]


def extract_binary(dist: Path, target: str, work: Path) -> Path:
    archive = dist / f"ik-{target}.tar.gz"
    with tarfile.open(archive, "r:gz") as packed:
        member = packed.getmember("ik")
        data = packed.extractfile(member)
        if data is None:
            raise SystemExit("archive has no ik")
        work.mkdir(parents=True, exist_ok=True)
        binary = work / "ik"
        binary.write_bytes(data.read())
    binary.chmod(0o755)
    return binary


def install_project(project: Path, tarballs: list[Path], env: dict[str, str]) -> None:
    project.mkdir(parents=True)
    (project / "package.json").write_text(json.dumps({"name": "consumer", "private": True, "type": "module"}))
    run(["npm", "install", "--no-audit", "--no-fund", "--ignore-scripts", *map(str, tarballs)], project, env)


def installed_checks(project: Path, env: dict[str, str], binary: Path) -> dict[str, Any]:
    bin_dir = project / "node_modules" / ".bin"
    demo = run([str(bin_dir / "desmo"), "demo", "stale-worker"], project, env).stdout
    codegen = run([str(bin_dir / "desmo"), "codegen", str(project / "node_modules/@desmo/runtime/assets/verification_attempt.ik.toml")], project, env).stdout
    probe_file = project / "probe.mjs"
    probe_file.write_text(OPERATION_PROBE)
    probe = json.loads(run(["node", str(probe_file), str(project / "ops.db")], project, env).stdout)
    located = run(["node", "-e", "import('@desmo/runtime').then(m=>console.log(m.locateBinary()))"], project, env).stdout.strip()
    return {
        "demo_names_stale_refusal": "finish refused" in demo and "Canonical state was not changed" in demo,
        "codegen_emits_types": "export" in codegen,
        "operation_probe": probe,
        "binary_resolved_from_platform_package": "node_modules/@desmo/" in located,
        "resolved_binary_sha256": hashlib.sha256(Path(located).read_bytes()).hexdigest(),
        "extracted_binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
    }


def generated_app(project: Path, tarballs: list[Path], env: dict[str, str]) -> bool:
    app = project / "app"
    run([str(project / "node_modules/.bin/desmo"), "init", "--template", "job", "--language", "typescript", "--out", str(app)], project, env)
    package = json.loads((app / "package.json").read_text())
    package["dependencies"] = {}
    (app / "package.json").write_text(json.dumps(package))
    run(["npm", "install", "--no-audit", "--no-fund", "--ignore-scripts", *map(str, tarballs)], app, env)
    return run(["npm", "test"], app, env, check=False).returncode == 0


def python_checks(dist: Path, binary: Path, work: Path, env: dict[str, str]) -> dict[str, Any]:
    wheel = next(dist.glob("invariant_kernel-*.whl"))
    venv = work / "pyvenv"
    run([sys.executable, "-m", "venv", str(venv)], work, env)
    py = venv / "bin" / "python"
    run([str(py), "-m", "pip", "install", "--no-index", "--no-deps", "--quiet", str(wheel)], work, {**env, "PIP_NO_INDEX": "1"})
    code = (
        "import sys; from invariant_kernel import Kernel\n"
        "k = Kernel(sys.argv[2], binary=sys.argv[1]); m = k.machine(sys.argv[3])\n"
        "d = m.create('p1'); op = k.new_operation_id(); d.command('start')\n"
        "print(m.lookup_operation('p1', d.capability, op, command={'name':'start'}).outcome)\n"
    )
    machine = REPO / "machines" / "verification_attempt.ik.toml"
    result = run([str(py), "-I", "-c", code, str(binary), str(work / "py.db"), str(machine)], work, env)
    return {"wheel": wheel.name, "lookup_of_unsent_operation": result.stdout.strip()}


def negative_controls(project: Path, tarballs: list[Path], work: Path, env: dict[str, str]) -> dict[str, Any]:
    controls: dict[str, Any] = {}
    bare = work / "no-platform"
    install_project(bare, [tarballs[0]], env)
    missing = run(["node", "-e", "import('@desmo/runtime').then(m=>m.locateBinary())"], bare, env, check=False)
    controls["no_platform_package_no_binary"] = {"exit": missing.returncode, "mentions_remedy": "ik binary not found" in missing.stderr}
    asset = project / "node_modules/@desmo/runtime/assets"
    shutil.move(asset, work / "assets.bak")
    gone = run(["node", "-e", "import('node:fs').then(fs=>fs.readFileSync(new URL('../assets/verification_attempt.ik.toml', import.meta.resolve('@desmo/runtime'))))"], project, env, check=False)
    shutil.move(work / "assets.bak", asset)
    controls["missing_asset"] = {"exit": gone.returncode, "enoent": "ENOENT" in gone.stderr}
    original = tarballs[0].read_bytes()
    tampered = work / "tampered.tgz"
    tampered.write_bytes(original[:-1] + bytes([original[-1] ^ 1]))
    controls["tampered_tarball_digest_differs"] = hashlib.sha256(original).hexdigest() != hashlib.sha256(tampered.read_bytes()).hexdigest()
    return controls


STORE_SCRIPT = """
import sys
from invariant_kernel import Kernel
binary, store, machine, mode = sys.argv[1:5]
kernel = Kernel(store, binary=binary)
m = kernel.machine(machine)
if mode == "write":
    d = m.create("compat-1")
    d = d.start()
    print(d.generation)
else:
    d = m.show("compat-1")
    print(d.generation, d.state, m.verify("compat-1")["verified"])
"""


def venv_with_wheel(wheel: Path, where: Path, env: dict[str, str]) -> Path:
    run([sys.executable, "-m", "venv", str(where)], where.parent, env)
    py = where / "bin" / "python"
    run([str(py), "-m", "pip", "install", "--no-index", "--no-deps", "--quiet", str(wheel)], where.parent, {**env, "PIP_NO_INDEX": "1"})
    return py


def store_meta(path: Path) -> str:
    # mode=rw refuses a missing file; query_only forbids writes. WAL stores need a writable handle.
    with sqlite3.connect(path.as_uri() + "?mode=rw", uri=True) as db:
        db.execute("PRAGMA query_only = ON")
        return dict(db.execute("SELECT key, value FROM meta").fetchall())["schema_version"]


def store_compat(previous: Path, dist: Path, binary: Path, work: Path, env: dict[str, str]) -> dict[str, Any]:
    """A store written by the previous release survives forward migration; the old binary then refuses it."""
    old_binary = extract_binary(previous, host_target(), work / "prev")
    old_py = venv_with_wheel(next(previous.glob("invariant_kernel-*.whl")), work / "oldvenv", env)
    new_py = venv_with_wheel(next(dist.glob("invariant_kernel-*.whl")), work / "newvenv", env)
    machine = str(REPO / "machines" / "verification_attempt.ik.toml")
    store, backup = work / "compat.db", work / "compat.backup.db"
    run([str(old_py), "-I", "-c", STORE_SCRIPT, str(old_binary), str(store), machine, "write"], work, env)
    before = store_meta(store)
    shutil.copy(store, backup)
    opened = run([str(new_py), "-I", "-c", STORE_SCRIPT, str(binary), str(store), machine, "read"], work, env).stdout.split()
    after = store_meta(store)
    refused = run([str(old_py), "-I", "-c", STORE_SCRIPT, str(old_binary), str(store), machine, "read"], work, env, check=False)
    restored = run([str(old_py), "-I", "-c", STORE_SCRIPT, str(old_binary), str(backup), machine, "read"], work, env, check=False)
    return {
        "previous_schema": before,
        "schema_after_new_binary_opens": after,
        "new_binary_reads_old_store": opened,
        "old_binary_on_migrated_store_exit": refused.returncode,
        "old_binary_refusal_text": refused.stderr.strip().splitlines()[-1][:200] if refused.stderr.strip() else "",
        "old_binary_on_restored_backup_exit": restored.returncode,
    }


def qualify(dist: Path, node_dir: Path, work: Path, previous: Path | None) -> dict[str, Any]:
    target = host_target()
    env = scrubbed_env(node_dir)
    binary = extract_binary(dist, target, work)
    manifest = pack_npm.pack(binary, target, work / "npm")
    tarballs = [work / "npm" / name for name in manifest["tarballs"]]
    project = work / "consumer"
    install_project(project, tarballs, env)
    report = {
        "host": {"system": platform.system(), "machine": platform.machine(), "target": target},
        "node": run(["node", "--version"], work, env).stdout.strip(),
        "npm_manifest": manifest,
        "installed": installed_checks(project, env, binary),
        "generated_app_tests_pass": generated_app(project, tarballs, env),
        "python": python_checks(dist, binary, work, env),
        "negative_controls": negative_controls(project, tarballs, work, env),
    }
    if previous is not None:
        report["store_compatibility"] = store_compat(previous, dist, binary, work, env)
    return report


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--dist", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--node", help="path to a node binary (default: the one on PATH)")
    parser.add_argument("--previous-dist", type=Path, help="previous release dist for store compatibility")
    args = parser.parse_args()
    node = Path(args.node) if args.node else Path(shutil.which("node") or "")
    previous = args.previous_dist.resolve() if args.previous_dist else None
    with tempfile.TemporaryDirectory(prefix="desmo-qualify-") as temp:
        report = qualify(args.dist.resolve(), node.parent, Path(temp), previous)
    args.out.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
