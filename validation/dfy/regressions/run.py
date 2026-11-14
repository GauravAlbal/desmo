#!/usr/bin/env python3
"""Execute regression fixtures against IK_BINARY. Exit 1 on any mismatch."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent
SEAMS = (
    "n13-recall-span",
    "n14-fill-arith",
    "n15-promote-cents",
)


def fail(msg: str) -> None:
    print(f"FAIL: {msg}", file=sys.stderr)
    raise SystemExit(1)


def run_cmd(binary: str, args: list[str], cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [binary, *args],
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def find_machine(seam_dir: Path) -> Path:
    machines = sorted(seam_dir.glob("*.ik.toml"))
    if len(machines) != 1:
        fail(f"{seam_dir.name}: expected one .ik.toml, found {len(machines)}")
    return machines[0]


def write_results(seam_dir: Path, blocks: list[str]) -> None:
    text = "\n\n".join(blocks).rstrip() + "\n"
    (seam_dir / "RESULTS.md").write_text(text, encoding="utf-8")


def extra_n15_asserts(result_path: Path) -> list[str]:
    data = json.loads(result_path.read_text(encoding="utf-8"))
    notes = []
    by_label = {step.get("label"): step for step in data.get("steps", [])}
    promote = by_label.get("promote")
    if promote is None:
        fail("n15-promote-cents: promote step missing from result")
    facts = (promote.get("state_after") or {}).get("facts") or {}
    expected = {
        "deposit_id": "",
        "deposit_cents": 0,
        "accepted_ms": 0,
        "deposit_deadline_ms": 0,
    }
    for key, want in expected.items():
        got = facts.get(key)
        if got != want:
            fail(f"n15-promote-cents: after promote {key}={got!r}, expected {want!r}")
    notes.append(f"after promote facts: {json.dumps(facts, sort_keys=True)}")
    reconcile = by_label.get("reconcile")
    if reconcile is None:
        fail("n15-promote-cents: reconcile step missing from result")
    rec_outcome = ((reconcile.get("response") or {}).get("outcome"))
    if rec_outcome != "committed" or reconcile.get("verdict") != "pass":
        fail(f"n15-promote-cents: reconcile did not commit: {reconcile}")
    notes.append("reconcile after promote committed (not leftover-deposit)")
    return notes


def bail(seam_dir: Path, blocks: list[str], msg: str) -> None:
    write_results(seam_dir, blocks)
    fail(msg)


def step_line(step: dict) -> str:
    facts = (step.get("state_after") or {}).get("facts")
    facts_txt = json.dumps(facts, sort_keys=True) if facts is not None else None
    return (
        f"- {step.get('label')}: expect={step.get('expectation') or step.get('expect')} "
        f"outcome={(step.get('response') or {}).get('outcome')} "
        f"verdict={step.get('verdict')} facts={facts_txt}"
    )


def block_for(scenario: Path, sc: subprocess.CompletedProcess[str], tail: str = "") -> str:
    out = (sc.stdout + sc.stderr).strip()
    return f"## scenario `{scenario.name}`\nexit={sc.returncode}\n```\n{out}\n```{tail}"


def load_result(seam_dir: Path, blocks: list[str], name: str, scenario: Path, sc, result_path: Path) -> dict:
    if sc.returncode != 0:
        blocks.append(block_for(scenario, sc))
        bail(seam_dir, blocks, f"{name}: scenario {scenario.name} exit {sc.returncode}")
    if not result_path.is_file():
        bail(seam_dir, blocks, f"{name}: {scenario.name} missing result.json")
    return json.loads(result_path.read_text(encoding="utf-8"))


def run_scenario(binary: str, name: str, scenario: Path, scratch: Path, blocks: list[str]) -> None:
    seam_dir = ROOT / name
    out_dir = scratch / name / scenario.stem
    shutil.rmtree(out_dir, ignore_errors=True)
    out_dir.mkdir(parents=True, exist_ok=True)
    sc = run_cmd(
        binary,
        ["scenario", "run", find_machine(seam_dir).name, f"scenarios/{scenario.name}", "--out", str(out_dir)],
        cwd=seam_dir,
    )
    result_path = out_dir / "result.json"
    result = load_result(seam_dir, blocks, name, scenario, sc, result_path)
    met = result.get("all_expectations_met")
    if met is not True:
        blocks.append(block_for(scenario, sc, f"\nall_expectations_met={met!r}"))
        bail(seam_dir, blocks, f"{name}: {scenario.name} expectations not met")
    lines = [step_line(step) for step in result.get("steps", [])]
    if (name, scenario.stem) == ("n15-promote-cents", "promote-reconcile"):
        lines += extra_n15_asserts(result_path)
    blocks.append(block_for(scenario, sc, "\n" + "\n".join(lines)))
    print(f"ok {name}/{scenario.name}: {(sc.stdout + sc.stderr).strip()}")


def run_stage(binary: str, seam_dir: Path, blocks: list[str], title: str, args: list[str], ok_codes: tuple) -> subprocess.CompletedProcess[str]:
    done = run_cmd(binary, args, cwd=seam_dir)
    out = (done.stdout + done.stderr).strip()
    blocks.append(f"## {title}\nexit={done.returncode}\n```\n{out}\n```")
    if done.returncode not in ok_codes:
        bail(seam_dir, blocks, f"{seam_dir.name}: {title} exit {done.returncode}")
    return done


def run_seam(binary: str, name: str, scratch: Path) -> None:
    seam_dir = ROOT / name
    machine = find_machine(seam_dir)
    corpus = machine.with_suffix(".cases.json")
    blocks = [f"# {name}", f"machine: `{machine.name}`"]
    run_stage(binary, seam_dir, blocks, "identity", ["identity", machine.name], (0,))
    check_args = ["check", machine.name] + (["--corpus", corpus.name] if corpus.is_file() else [])
    check = run_stage(binary, seam_dir, blocks, "check", check_args, (0, 1))
    scenarios = sorted((seam_dir / "scenarios").glob("*.json"))
    if not scenarios:
        bail(seam_dir, blocks, f"{name}: no scenarios")
    for scenario in scenarios:
        run_scenario(binary, name, scenario, scratch, blocks)
    write_results(seam_dir, blocks)
    print(f"ok {name} check exit={check.returncode}")


def main() -> None:
    binary = os.environ.get("IK_BINARY")
    if not binary:
        fail("set IK_BINARY to the ik executable")
    if not Path(binary).is_file():
        fail(f"IK_BINARY is not a file: {binary}")
    scratch_root = Path(tempfile.mkdtemp(prefix="dfy-regressions-", dir="/tmp"))
    try:
        for name in SEAMS:
            run_seam(binary, name, scratch_root)
    finally:
        shutil.rmtree(scratch_root, ignore_errors=True)
    print("all seams matched")


if __name__ == "__main__":
    main()
