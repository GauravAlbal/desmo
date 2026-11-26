#!/usr/bin/env python3
"""SDK transport deleted-wall mutation qualification runner (`validation/sdk_mutants/`).

Modes:
  --check      Validate claims.json schema and ensure every target string appears
               exactly once in its source file.
  --self-test  Verify the runner detects a surviving mutant (exit 0) and accepts a
               killed mutant (exit non-zero).
  --run        Apply each mutant in an isolated temporary copy of the repo and fail
               if any mutant survives.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
CLAIMS_PATH = Path(__file__).resolve().parent / "claims.json"
EXPECTED_SCHEMA = "ik.sdk_mutants.v1"


def load_claims(path: Path = CLAIMS_PATH) -> dict[str, Any]:
    data = json.loads(path.read_text(encoding="utf-8"))
    if data.get("schema") != EXPECTED_SCHEMA:
        raise ValueError(f"unexpected claims schema: {data.get('schema')!r}")
    mutants = data.get("mutants")
    if not isinstance(mutants, list) or not mutants:
        raise ValueError("claims.json must contain a non-empty mutants list")
    return data


def check_claims(claims: dict[str, Any], repo_root: Path = ROOT) -> None:
    seen_ids: set[str] = set()
    sdks: set[str] = set()
    for entry in claims["mutants"]:
        for key in ("id", "sdk", "file", "target", "replacement", "command", "wall"):
            if not isinstance(entry.get(key), str) or not entry[key]:
                raise ValueError(f"mutant {entry.get('id')!r} missing non-empty `{key}`")
        mid = entry["id"]
        if mid in seen_ids:
            raise ValueError(f"duplicate mutant id `{mid}`")
        seen_ids.add(mid)
        sdks.add(entry["sdk"])
        if entry["target"] == entry["replacement"]:
            raise ValueError(f"mutant `{mid}` has identical target and replacement")
        src_path = repo_root / entry["file"]
        if not src_path.is_file():
            raise FileNotFoundError(f"mutant `{mid}` target file not found: {src_path}")
        text = src_path.read_text(encoding="utf-8")
        count = text.count(entry["target"])
        if count != 1:
            raise ValueError(
                f"mutant `{mid}` target matched {count} times in {entry['file']} (expected 1)"
            )
    if sdks != {"ts", "py", "go"}:
        raise ValueError(f"expected mutants covering ts, py, go; got {sorted(sdks)}")


def evaluate_mutant(entry: dict[str, Any], workspace: Path, env: dict[str, str]) -> tuple[bool, int, str]:
    """Mutate one file inside `workspace`, run its test command, restore original bytes.

    Returns `(killed, returncode, tail_output)`.
    """
    target_path = workspace / entry["file"]
    original = target_path.read_text(encoding="utf-8")
    count = original.count(entry["target"])
    if count != 1:
        raise RuntimeError(
            f"mutant `{entry['id']}` matched {count} times in workspace {target_path}"
        )
    mutated = original.replace(entry["target"], entry["replacement"], 1)
    try:
        target_path.write_text(mutated, encoding="utf-8")
        proc = subprocess.run(
            entry["command"],
            shell=True,
            cwd=workspace,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
    finally:
        target_path.write_text(original, encoding="utf-8")

    output_lines = (proc.stdout or "").strip().splitlines()
    tail = "\n".join(output_lines[-10:])
    return (proc.returncode != 0, proc.returncode, tail)


def run_self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="desmo-sdk-mutant-selftest-") as tmp:
        ws = Path(tmp)
        sample = ws / "guard.py"
        sample.write_text("ALLOW = False\n", encoding="utf-8")
        env = dict(os.environ)

        killed, rc, _ = evaluate_mutant(
            {
                "id": "SELFTEST_KILLED",
                "file": "guard.py",
                "target": "ALLOW = False",
                "replacement": "ALLOW = True",
                "command": "python3 -c 'import sys; sys.exit(1)'",
            },
            ws,
            env,
        )
        assert killed and rc != 0, "expected failing test under mutation to count as killed"
        assert sample.read_text(encoding="utf-8") == "ALLOW = False\n", "workspace not restored"

        survived, rc2, _ = evaluate_mutant(
            {
                "id": "SELFTEST_SURVIVED",
                "file": "guard.py",
                "target": "ALLOW = False",
                "replacement": "ALLOW = True",
                "command": "python3 -c 'import sys; sys.exit(0)'",
            },
            ws,
            env,
        )
        assert (not survived) and rc2 == 0, "expected passing test under mutation to count as surviving"
        assert sample.read_text(encoding="utf-8") == "ALLOW = False\n", "workspace not restored"
    print("validation/sdk_mutants self-test OK")


def _copy_ts_ignore(directory: str, entries: list[str]) -> set[str]:
    ignored: set[str] = set()
    for name in entries:
        if name == "node_modules" or name.startswith(".codegen-") or name == ".DS_Store":
            ignored.add(name)
    return ignored


def stage_isolated_workspace(temp_root: Path, repo_root: Path = ROOT) -> Path:
    shutil.copy2(repo_root / "Cargo.toml", temp_root / "Cargo.toml")
    for folder in ("machines", "examples"):
        shutil.copytree(repo_root / folder, temp_root / folder)
    shutil.copytree(
        repo_root / "python",
        temp_root / "python",
        ignore=shutil.ignore_patterns("__pycache__", ".pytest_cache", "*.pyc"),
    )
    shutil.copytree(repo_root / "go", temp_root / "go")
    shutil.copytree(repo_root / "typescript", temp_root / "typescript", ignore=_copy_ts_ignore)
    nm = repo_root / "typescript" / "node_modules"
    if not nm.is_dir():
        raise FileNotFoundError(
            f"{nm} is missing; install TypeScript dependencies (`npm ci` in typescript/) before --run"
        )
    os.symlink(nm, temp_root / "typescript" / "node_modules")
    if (repo_root / "target").exists():
        os.symlink(repo_root / "target", temp_root / "target")
    return temp_root


def run_mutants(claims: dict[str, Any], repo_root: Path = ROOT) -> None:
    ik_bin = os.environ.get("IK_BINARY") or os.environ.get("IK_BIN") or str(
        repo_root / "target" / "debug" / "ik"
    )
    if not os.access(ik_bin, os.X_OK):
        raise FileNotFoundError(f"executable ik binary not found at {ik_bin}")

    with tempfile.TemporaryDirectory(prefix="desmo-sdk-mutants-") as tmp:
        ws = stage_isolated_workspace(Path(tmp), repo_root)
        env = dict(os.environ)
        env["IK_BINARY"] = ik_bin
        env["IK_BIN"] = ik_bin
        env["PYTEST_DISABLE_PLUGIN_AUTOLOAD"] = "1"
        env["PYTHONPATH"] = str(ws / "python" / "src")

        survivors: list[str] = []
        for entry in claims["mutants"]:
            killed, rc, tail = evaluate_mutant(entry, ws, env)
            if killed:
                print(f"KILLED {entry['id']} ({entry['file']}, exit={rc})")
            else:
                print(f"SURVIVED {entry['id']} ({entry['file']}):\n{tail}", file=sys.stderr)
                survivors.append(entry["id"])

        total = len(claims["mutants"])
        if survivors:
            raise RuntimeError(
                f"{len(survivors)}/{total} SDK transport mutants survived: {', '.join(survivors)}"
            )
        print(f"validation/sdk_mutants: {total}/{total} SDK transport mutants killed")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="SDK transport mutation qualification gate")
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--run", action="store_true")
    args = parser.parse_args(argv)
    if not (args.check or args.self_test or args.run):
        parser.print_help()
        return 2
    claims = load_claims()
    if args.check:
        check_claims(claims)
        print(f"validation/sdk_mutants --check OK ({len(claims['mutants'])} mutants)")
    if args.self_test:
        run_self_test()
    if args.run:
        check_claims(claims)
        run_mutants(claims)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
