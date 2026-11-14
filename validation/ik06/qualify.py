#!/usr/bin/env python3
"""Validate and execute the IK-06 claim -> enforcement -> evidence manifest.

    python3 validation/ik06/qualify.py --check            # static: rows exist, every mutant covered
    python3 validation/ik06/qualify.py --self-test        # the validator rejects broken manifests
    python3 validation/ik06/qualify.py --run --out validation/ik06/results.json

`--run` executes every evidence and mutant-kill row through its real runner and records the raw
command, exit code and the counts parsed from the runner's own output. A row passes only if the
runner exited 0, at least one test ran, and none were skipped or ignored. A kill test cannot pass
without its mutant's seam having fired: `ik::law_mutant::with_law_mutant` panics otherwise.
Metadata alone never satisfies a row.
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import platform
import re
import subprocess
import sys
import time
from pathlib import Path
from typing import Any, Optional

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
MANIFEST = HERE / "claims.json"
ENUM_SRC = REPO / "src" / "law_mutant.rs"


def load_manifest(path: Path = MANIFEST) -> dict[str, Any]:
    return json.loads(path.read_text())


def enum_variants(text: str) -> list[str]:
    body = text.split("pub enum LawMutant {", 1)[1].split("\n}", 1)[0]
    lines = [line.strip() for line in body.splitlines()]
    return [line.rstrip(",") for line in lines if line and not line.startswith("//")]


def test_patterns(runner: str, name: str) -> list[str]:
    leaf = re.escape(name.split("::")[-1])
    return {
        "rust": [rf"\bfn {leaf}\b", rf"^\s*{leaf}:\s"],
        "pytest": [rf"\bdef {leaf}\b"],
        "node": [rf'\btest\("{re.escape(name)}"'],
        "go": [rf"\bfunc {leaf}\b"],
    }[runner]


def find_test(text: str, runner: str, name: str) -> Optional[int]:
    """Line index of the test declaration, or None if absent."""
    lines = text.splitlines()
    for pattern in test_patterns(runner, name):
        for index, line in enumerate(lines):
            if re.search(pattern, line):
                return index
    return None


def is_ignored(text: str, runner: str, index: int) -> bool:
    window = text.splitlines()[max(0, index - 4) : index]
    marks = {"rust": "#[ignore", "pytest": "pytest.mark.skip", "node": "skip", "go": "t.Skip"}
    return any(marks[runner] in line for line in window)


def row_problems(row: dict[str, Any], root: Path) -> list[str]:
    path = root / row["file"]
    if not path.exists():
        return [f"{row['file']}: file missing"]
    text = path.read_text()
    index = find_test(text, row["runner"], row["test"])
    if index is None:
        return [f"{row['file']}::{row['test']}: test not found"]
    if is_ignored(text, row["runner"], index):
        return [f"{row['file']}::{row['test']}: ignored/skipped tests are not evidence"]
    return []


def enforcement_problems(item: dict[str, Any], root: Path) -> list[str]:
    path = root / item["file"]
    if not path.exists() or item["contains"] not in path.read_text():
        return [f"enforcement {item['file']} does not contain {item['contains']!r}"]
    return []


def claim_problems(claim: dict[str, Any], root: Path) -> list[str]:
    problems = []
    if not claim.get("evidence"):
        problems.append(f"{claim['id']}: no evidence rows")
    if not claim.get("enforcement"):
        problems.append(f"{claim['id']}: no enforcement rows")
    for item in claim.get("enforcement", []):
        problems += [f"{claim['id']}: {p}" for p in enforcement_problems(item, root)]
    rows = list(claim.get("evidence", [])) + [m["kill"] for m in claim.get("mutants", [])]
    for row in rows:
        problems += [f"{claim['id']}: {p}" for p in row_problems(row, root)]
    return problems


def coverage_problems(manifest: dict[str, Any], variants: list[str]) -> list[str]:
    declared = [m["name"] for c in manifest["claims"] for m in c.get("mutants", [])]
    problems = [f"mutant {name} is not covered by any claim" for name in variants if name not in declared]
    problems += [f"manifest names unknown mutant {name}" for name in declared if name not in variants]
    problems += [f"mutant {name} is claimed twice" for name in set(declared) if declared.count(name) > 1]
    return problems


def validate(manifest: dict[str, Any], root: Path = REPO, variants: Optional[list[str]] = None) -> list[str]:
    variants = variants if variants is not None else enum_variants((root / "src" / "law_mutant.rs").read_text())
    problems = coverage_problems(manifest, variants)
    for claim in manifest["claims"]:
        problems += claim_problems(claim, root)
    return problems


def self_test() -> list[str]:
    """Each broken manifest must be rejected; returns the controls that were NOT detected."""
    base = load_manifest()
    undetected = []

    def expect(label: str, mutate: Any) -> None:
        broken = copy.deepcopy(base)
        mutate(broken)
        if not validate(broken):
            undetected.append(label)

    expect("removed evidence row", lambda m: m["claims"][0].__setitem__("evidence", []))
    expect("removed mutant entry", lambda m: m["claims"][0]["mutants"].pop())
    expect("renamed test", lambda m: m["claims"][0]["evidence"][0].__setitem__("test", "no_such_test"))
    expect("unknown mutant", lambda m: m["claims"][0]["mutants"].append({"name": "Ghost", "kill": m["claims"][0]["mutants"][0]["kill"]}))
    expect("dropped enforcement", lambda m: m["claims"][0].__setitem__("enforcement", []))
    expect("wrong enforcement text", lambda m: m["claims"][0]["enforcement"][0].__setitem__("contains", "fn nope"))
    if find_test("#[ignore]\n#[test]\nfn t() {}", "rust", "t") is None or not is_ignored("#[ignore]\n#[test]\nfn t() {}", "rust", 2):
        undetected.append("ignored test accepted")
    if parse_counts("rust", "test result: ok. 0 passed; 0 failed; 0 ignored")["passed"] != 0:
        undetected.append("zero-test run counted")
    return undetected


def parse_counts(runner: str, output: str) -> dict[str, int]:
    if runner == "rust":
        found = re.findall(r"(\d+) passed; (\d+) failed; (\d+) ignored", output)
        return {
            "passed": sum(int(p) for p, _, _ in found),
            "failed": sum(int(f) for _, f, _ in found),
            "skipped": sum(int(i) for _, _, i in found),
        }
    if runner == "pytest":
        return {"passed": _count(output, "passed"), "failed": _count(output, "failed"), "skipped": _count(output, "skipped")}
    if runner == "node":
        return {"passed": _count(output, "pass", prefix=True), "failed": _count(output, "fail", prefix=True), "skipped": _count(output, "skipped", prefix=True)}
    return {"passed": len(re.findall(r"^--- PASS", output, re.M)), "failed": len(re.findall(r"^--- FAIL", output, re.M)), "skipped": len(re.findall(r"^--- SKIP", output, re.M))}


def _count(output: str, word: str, prefix: bool = False) -> int:
    pattern = rf"ℹ {word} (\d+)" if prefix else rf"(\d+) {word}"
    match = re.search(pattern, output)
    return int(match.group(1)) if match else 0


def command_for(row: dict[str, Any], ik: str) -> tuple[list[str], Path, dict[str, str]]:
    runner, name, file = row["runner"], row["test"], Path(row["file"])
    env = dict(os.environ, IK_BINARY=ik, PYTEST_DISABLE_PLUGIN_AUTOLOAD="1")
    if runner == "rust":
        argv = ["cargo", "test", "--offline", "--all-features", "--test", file.stem, "--", "--exact", name]
        return argv, REPO, env
    if runner == "pytest":
        return [sys.executable, "-m", "pytest", "-q", f"{row['file']}::{name}"], REPO, env
    if runner == "node":
        dist = f"dist/{file.stem}.js"
        return ["node", "--test", f"--test-name-pattern=^{re.escape(name)}$", dist], REPO / "typescript", env
    return ["go", "test", "-count=1", "-v", "-run", f"^{name}$", "./..."], REPO / "go", env


def run_row(row: dict[str, Any], ik: str) -> dict[str, Any]:
    argv, cwd, env = command_for(row, ik)
    started = time.time()
    proc = subprocess.run(argv, cwd=cwd, env=env, capture_output=True, text=True)
    output = proc.stdout + proc.stderr
    counts = parse_counts(row["runner"], output)
    ok = proc.returncode == 0 and counts["passed"] >= 1 and counts["failed"] == 0 and counts["skipped"] == 0
    return {
        "row": f"{row['file']}::{row['test']}",
        "command": " ".join(argv),
        "cwd": str(cwd.relative_to(REPO)) if cwd != REPO else ".",
        "exit_code": proc.returncode,
        "counts": counts,
        "seconds": round(time.time() - started, 2),
        "passed": ok,
    }


def provenance() -> dict[str, Any]:
    def out(*argv: str) -> str:
        return subprocess.run(argv, cwd=REPO, capture_output=True, text=True).stdout.strip()

    return {
        "source_revision": out("git", "rev-parse", "HEAD"),
        "dirty": bool(out("git", "status", "--porcelain", "--", "src", "tests", "python", "typescript", "go")),
        "rustc": out("rustc", "--version"),
        "platform": platform.platform(),
        "python": platform.python_version(),
    }


def compile_typescript(manifest: dict[str, Any]) -> None:
    """Node rows execute `typescript/dist`; rebuild it from source first so a stale dist cannot pass."""
    rows = [r for c in manifest["claims"] for r in c["evidence"] if r["runner"] == "node"]
    if rows:
        subprocess.run(["npx", "tsc", "-p", "tsconfig.json"], cwd=REPO / "typescript", check=True)


def run_all(manifest: dict[str, Any], ik: str) -> dict[str, Any]:
    compile_typescript(manifest)
    results = []
    for claim in manifest["claims"]:
        rows = [("evidence", r) for r in claim["evidence"]] + [(f"kill:{m['name']}", m["kill"]) for m in claim["mutants"]]
        for kind, row in rows:
            result = run_row(row, ik)
            results.append({"claim": claim["id"], "kind": kind, **result})
            print(("PASS " if result["passed"] else "FAIL ") + f"{claim['id']} {kind} {result['row']}", file=sys.stderr)
    return {
        "schema": "ikaros.qualification-results.v1",
        "provenance": provenance(),
        "scope": manifest["scope"],
        "gaps": manifest["gaps"],
        "results": results,
        "all_passed": all(r["passed"] for r in results),
    }


def report_problems(manifest: dict[str, Any]) -> int:
    problems = validate(manifest)
    for problem in problems:
        print("PROBLEM", problem, file=sys.stderr)
    return 1 if problems else 0


def report_self_test() -> int:
    undetected = self_test()
    print("undetected controls:", undetected, file=sys.stderr)
    return 1 if undetected else 0


def write_run(manifest: dict[str, Any], ik: str, out: Optional[Path]) -> int:
    report = run_all(manifest, ik)
    (out or HERE / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    return 0 if report["all_passed"] else 1


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--run", action="store_true")
    parser.add_argument("--ik", default=str(REPO / "target" / "debug" / "ik"))
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    manifest = load_manifest()
    if (args.check or args.run) and report_problems(manifest):
        return 1
    if args.self_test:
        return report_self_test()
    return write_run(manifest, args.ik, args.out) if args.run else 0


if __name__ == "__main__":
    sys.exit(main())
