#!/usr/bin/env python3
"""Exercise staged release bytes, not checkout imports. Never downloads or publishes."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import venv
import zipfile

REPO = Path(__file__).resolve().parent.parent
VERSION = "0.6.0"
# Independently accepted v0.2.1 darwin-arm64 artifact, source
# 568525461ed4b73cc4874e2735cc0786bc23781c, artifact SHA256
# 81edee455b8d139210508e1f12e525540f043deba165f6bc824969b7f24c79a5.
SCHEMA1_HASH = "aae4f03a6e4169bdc86021914bc57db4bc391878651fa61fcd2a9d0d215b4a7f"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def run(args: list[str], *, cwd: Path, env: dict[str, str]) -> str:
    result = subprocess.run(args, cwd=cwd, env=env, text=True, capture_output=True)
    require(result.returncode == 0,
            f"{args!r} failed ({result.returncode})\n{result.stdout}\n{result.stderr}")
    return result.stdout


def safe_name(name: str) -> bool:
    path = PurePosixPath(name)
    return bool(name) and not path.is_absolute() and ".." not in path.parts and "\\" not in name


def _verify_manifest(dist: Path, revision: str) -> dict:
    manifest = json.loads((dist / "release-manifest.json").read_text())
    expected = {
        "product": "Ikaros", "repository": "invariant-kernel",
        "version": VERSION, "tag": f"v{VERSION}", "runtime_version": VERSION,
        "python_client_version": VERSION, "protocol_major": 1,
        "minimum_supported_protocol": 1, "schema_version": 4,
        "source_sha": revision, "go_client_source_revision": revision,
    }
    for key, value in expected.items():
        require(manifest.get(key) == value, f"manifest {key}: expected {value!r}, got {manifest.get(key)!r}")
    require((dist / "source-sha.txt").read_text() == revision + "\n", "source-sha.txt differs from explicit revision")
    require((dist / "version.txt").read_text() == f"v{VERSION}\n", "version.txt differs from release tag")
    return manifest


def _verify_artifact_digests(dist: Path, artifacts: list) -> dict[str, str]:
    require(isinstance(artifacts, list) and bool(artifacts), "manifest artifacts missing")
    digests: dict[str, str] = {}
    for entry in artifacts:
        name, digest = entry["file"], entry["sha256"]
        require(safe_name(name) and PurePosixPath(name).name == name, f"unsafe artifact name {name!r}")
        require(name not in digests, f"duplicate artifact {name}")
        require(bool(re.fullmatch(r"[0-9a-f]{64}", digest)), f"invalid artifact digest {name}")
        path = dist / name
        require(path.is_file() and not path.is_symlink(), f"artifact is not a regular file: {name}")
        require(hashlib.sha256(path.read_bytes()).hexdigest() == digest, f"artifact digest mismatch: {name}")
        digests[name] = digest
    return digests


def _verify_checksums(dist: Path, digests: dict[str, str]) -> None:
    sums: dict[str, str] = {}
    for line in (dist / "SHA256SUMS").read_text().splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  ([^/\\]+)", line)
        require(match is not None, f"invalid SHA256SUMS row {line!r}")
        digest, name = match.groups()
        require(name not in sums, f"duplicate SHA256SUMS row {name}")
        sums[name] = digest
    require(sums == digests, "SHA256SUMS and manifest inventories differ")
    metadata = {"SHA256SUMS", "release-manifest.json", "source-sha.txt", "version.txt"}
    require({p.name for p in dist.iterdir()} == set(digests) | metadata, "unexpected or missing staged release files")


def _partition_artifacts(
    dist: Path, digests: dict[str, str]
) -> tuple[list[Path], list[Path], list[str]]:
    archives, wheels, sdists = [], [], []
    for name in digests:
        if name.endswith(".whl"):
            wheels.append(dist / name)
        elif name.endswith(".tar.gz"):
            if name.startswith("ik-"):
                archives.append(dist / name)
            else:
                sdists.append(name)
    return archives, wheels, sdists


def _verify_host_artifacts(
    dist: Path, artifacts: list, digests: dict[str, str], env: dict[str, str]
) -> tuple[Path, Path]:
    archives, wheels, sdists = _partition_artifacts(dist, digests)
    require(len(archives) == len(wheels) == len(sdists) == 1, "expected one host archive, one wheel and one sdist")
    target = run(["rustc", "-vV"], cwd=REPO, env=env)
    host = next(line.removeprefix("host: ") for line in target.splitlines() if line.startswith("host: "))
    entry = next(entry for entry in artifacts if entry["file"] == archives[0].name)
    require(entry.get("target") == host, "archive target must be this host's supported Rust triple")
    return archives[0], wheels[0]


def verify_inventory(dist: Path, revision: str, env: dict[str, str]) -> tuple[Path, Path]:
    require(bool(re.fullmatch(r"[0-9a-f]{40}", revision)), "expected source revision must be a full Git SHA")
    head = run(["git", "rev-parse", "HEAD"], cwd=REPO, env=env).strip()
    require(head == revision, "explicit source revision does not equal repository HEAD")
    # LATEST_RELEASE is intentionally copied by scripts/release only after qualification.
    run(["git", "diff", "--exit-code", "HEAD", "--", ".", ":(exclude)LATEST_RELEASE"], cwd=REPO, env=env)
    manifest = _verify_manifest(dist, revision)
    artifacts = manifest["artifacts"]
    digests = _verify_artifact_digests(dist, artifacts)
    _verify_checksums(dist, digests)
    return _verify_host_artifacts(dist, artifacts, digests, env)


# Runs ONLY with the fresh venv interpreter in isolated mode, from a temporary cwd.
# No pytest/conftest/source path is shared with this consumer process.
INSTALLED_PROBE = r'''
import importlib.metadata
import json
from pathlib import Path
import sqlite3
import sys
import invariant_kernel
from invariant_kernel import Kernel, InvariantFailedError

binary, repo, install, work, accepted_hash = map(Path, sys.argv[1:])
# The hash is an opaque argument, not a filesystem path.
accepted_hash = str(accepted_hash)
assert Path(invariant_kernel.__file__).resolve().is_relative_to(install.resolve()), invariant_kernel.__file__
distribution = importlib.metadata.distribution("invariant-kernel")
assert distribution.version == "0.6.0", distribution.version
assert Path(distribution.locate_file("")).resolve().is_relative_to(install.resolve())
assert all(Path(item).name not in {"ik", "ik.exe"} for item in distribution.files)
assert not any(str(repo / "python" / "src") == entry for entry in sys.path), sys.path

store = work / "installed-consumer.db"
kernel = Kernel(store, binary=str(binary))
legacy = kernel.machine(repo / "machines" / "verification_attempt.ik.toml")
assert legacy.identity.to_wire() == {
    "machine_id": "verification_attempt", "version": 1, "spec_hash": accepted_hash,
}, legacy.identity.to_wire()
region_path = repo / "machines" / "region_parallel.ik.toml"
checked = kernel.check(region_path)
assert checked.verdict == "refused" and checked.ok is False, checked.text
assert "VERDICT" in checked.text and "REFUSED" in checked.text, checked.text
machine = kernel.machine(region_path)
created = machine.create("packaged-region")
initial = {"state": "playing__unmuted", "region_playback": "playing", "region_volume": "unmuted"}
assert created.raw["state"] == initial, created.raw
assert created.generation == 0 and created.raw["capability"]["generation"] == 0

def rows(database=store):
    # SQLite may need WAL/SHM housekeeping before its first read on macOS.
    # mode=rw refuses a missing database; query_only forbids authoritative writes.
    with sqlite3.connect(database.as_uri() + "?mode=rw", uri=True) as db:
        db.execute("PRAGMA query_only = ON")
        db.execute("BEGIN")
        return {table: db.execute("SELECT * FROM " + table + " ORDER BY " + order).fetchall()
                for table, order in [("meta", "key"), ("aggregates", "aggregate_id"),
                                     ("journal", "aggregate_id, sequence"), ("receipts", "aggregate_id, sequence")]}

before = rows()
meta = dict(before["meta"])
assert meta["schema_version"] == "4", meta
assert meta["runtime_version"] == "0.6.0", meta
try:
    created.command("mute")
except InvariantFailedError as error:
    assert error.code == "IK_INVARIANT_FAILED", error.diagnostic
    assert error.rule == "no_playing_muted", error.diagnostic
else:
    raise AssertionError("packaged binary committed unsafe playing+muted")
assert rows() == before, "refused command changed durable rows"
# Reopen the actual binary/store before verify/replay; a cached handle is not authority.
reopened = Kernel(store, binary=str(binary)).machine(region_path)
shown = reopened.show("packaged-region")
assert shown.raw["state"] == initial and shown.generation == 0, shown.raw
assert shown.raw["capability"] == created.raw["capability"]
verified = reopened.verify("packaged-region")
assert verified["aggregates"] == 1 and verified["journal_ok"] is True, verified
assert verified["projection_ok"] is True and verified["verified"] is True, verified
replayed = reopened.replay("packaged-region")
assert replayed["aggregates"] == [{"aggregate_id": "packaged-region", "sequence": 0,
    "generation": 0, "state": initial, "rebuilt": False}], replayed
assert rows() == before, "verify/replay must not conceal unsafe durable writes"
paused = created.command("pause")
assert paused.raw["state"] == {"state": "paused__unmuted", "region_playback": "paused", "region_volume": "unmuted"}, paused.raw
assert paused.generation == 1 and paused.raw["receipt"]["sequence"] == 1, paused.raw
assert paused.raw["capability"]["generation"] == 1

# Region lowering also crosses the preserved persistent-session/transaction seam.
session_store = work / "installed-session.db"
with Kernel(session_store, binary=str(binary), session=True) as persistent:
    session_machine = persistent.machine(region_path)
    session_created = session_machine.create("packaged-session")
    session_before = rows(session_store)
    try:
        session_created.transaction([{"name": "pause"}, {"name": "mute"}, {"name": "play"}])
    except InvariantFailedError as error:
        assert error.code == "IK_INVARIANT_FAILED" and error.rule == "no_playing_muted", error.diagnostic
    else:
        raise AssertionError("packaged session committed a partially unsafe transaction")
    assert rows(session_store) == session_before, "failed transaction partially committed"
    session_shown = session_machine.show("packaged-session")
    assert session_shown.raw["state"] == initial and session_shown.generation == 0, session_shown.raw
    assert session_shown.raw["capability"] == session_created.raw["capability"]
    batch = session_created.transaction([{"name": "pause"}, {"name": "mute"}])
    expected = {"state": "paused__muted", "region_playback": "paused", "region_volume": "muted"}
    assert batch.raw["state"] == expected and batch.generation == 2, batch.raw
    assert session_machine.verify("packaged-session")["verified"] is True
    rebuilt = session_machine.replay("packaged-session")["aggregates"][0]
    assert rebuilt["state"] == expected and rebuilt["generation"] == 2, rebuilt
print(json.dumps({"python_version": distribution.version, "schema1_hash": legacy.identity.to_wire()["spec_hash"],
                  "store_schema": meta["schema_version"], "refusal_code": "IK_INVARIANT_FAILED",
                  "safe_generation": paused.generation, "session_transaction_generation": batch.generation}))
'''


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("dist", type=Path)
    parser.add_argument("--source-revision", required=True)
    args = parser.parse_args()
    require(sys.version_info >= (3, 9), "release verification requires Python >=3.9")
    env = dict(os.environ)
    for key in ("PYTHONPATH", "PYTHONHOME", "PYTHONSTARTUP", "IK_DIST", "IK_BIN", "GOFLAGS"):
        env.pop(key, None)
    env.update({"PIP_NO_INDEX": "1", "PIP_NO_INPUT": "1", "PIP_DISABLE_PIP_VERSION_CHECK": "1",
                "PIP_CONFIG_FILE": os.devnull, "GOPROXY": "off", "GOSUMDB": "off",
                "GOTOOLCHAIN": "local", "GOWORK": "off", "GOENV": "off", "GOFLAGS": ""})
    archive, wheel = verify_inventory(args.dist.resolve(), args.source_revision, env)
    with tempfile.TemporaryDirectory(prefix="desmo-release-consumer-") as temp:
        work = Path(temp)
        binary = work / "ik"
        with tarfile.open(archive, "r:gz") as packed:
            members = packed.getmembers()
            require(len(members) == 1, "host archive must contain only ik")
            member = members[0]
            require(member.name == "ik" and member.isfile() and bool(member.mode & 0o111),
                    "host archive must contain a regular executable ik, never links or traversal")
            require(0 < member.size <= 512 * 1024 * 1024, "implausible binary archive size")
            source = packed.extractfile(member)
            require(source is not None, "missing binary archive content")
            with source, binary.open("xb") as target:
                shutil.copyfileobj(source, target)
        binary.chmod(0o700)
        version = run([str(binary), "--version"], cwd=work, env=env).strip()
        require(version == "invariant-kernel 0.6.0 protocol 1.0", f"wrong packaged runtime: {version}")
        identity = json.loads(run([str(binary), "identity", str(REPO / "machines/verification_attempt.ik.toml")], cwd=work, env=env))
        require(identity == {"machine_id": "verification_attempt", "version": 1, "spec_hash": SCHEMA1_HASH},
                "packaged schema-1 identity differs from independently accepted v0.2.1 identity")
        with zipfile.ZipFile(wheel) as packed:
            require(all(safe_name(name) for name in packed.namelist()), "unsafe wheel member path")
            require(all(PurePosixPath(name).name not in {"ik", "ik.exe"} for name in packed.namelist()),
                    "Python wheel must not bundle ik")
        install = work / "venv"
        venv.EnvBuilder(with_pip=True).create(install)
        python = install / "bin/python"
        run([str(python), "-I", "-m", "pip", "install", "--no-index", "--no-deps", "--no-compile", str(wheel)], cwd=work, env=env)
        probe = run([str(python), "-I", "-c", INSTALLED_PROBE, str(binary), str(REPO), str(install), str(work), SCHEMA1_HASH], cwd=work, env=env)
        evidence = json.loads(probe)
        env["IK_BINARY"] = str(binary)
        go_output = run(["go", "test", "-count=1", "./..."], cwd=REPO / "go", env=env)
        require("[no test files]" not in go_output and "[no tests to run]" not in go_output
                and bool(re.search(r"^ok\s", go_output, re.MULTILINE)),
                f"supported Go client tests did not execute: {go_output}")
        evidence.update({"runtime": version, "source_sha": args.source_revision, "go_client": "passed"})
        print(json.dumps(evidence, sort_keys=True))


if __name__ == "__main__":
    main()
