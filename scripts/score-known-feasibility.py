#!/usr/bin/python3
"""Score a feasibility backend against the known-solution truth suite.

A backend command is invoked once per case:

  CMD --machine PATH --corpus PATH --case-id ID

It must print JSON: an object with `obligations` or a bare list. Each row is
`{id, status, trust}` where status is `witness`, `impossible`, or `unknown`,
and trust is `checked` or `solver`. A `witness` row may include `trace`, a
list of `{command, args}` steps.

Scoring:
  impossible on a REACHABLE obligation is UNSOUND (fatal)
  a witness that does not replay on the kernel is fatal
  unknown is never wrong
  solver-asserted trust is counted separately from checked
  precision is the fraction of UNREACHABLE obligations reported impossible
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


STATUSES = ("witness", "impossible", "unknown")
TRUSTS = ("checked", "solver")
TRUTHS = ("REACHABLE", "UNREACHABLE", "UNKNOWN")


def parse_args(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--truth", required=True)
    parser.add_argument("--backend")
    parser.add_argument("--root")
    parser.add_argument("--ik")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--json", action="store_true")
    return parser.parse_args(argv)


def load_json(path):
    with open(path) as handle:
        return json.load(handle)


def as_rows(payload):
    if isinstance(payload, dict):
        rows = payload.get("obligations")
        if rows is None:
            raise ValueError("backend object missing obligations")
        return rows
    if isinstance(payload, list):
        return payload
    raise ValueError("backend output must be an object or list")


def index_rows(rows):
    out = {}
    for row in rows:
        oid = row.get("id")
        status = row.get("status")
        trust = row.get("trust") or "solver"
        if not oid or status not in STATUSES:
            raise ValueError(f"invalid backend row {row!r}")
        if trust not in TRUSTS:
            raise ValueError(f"invalid trust {trust!r}")
        out[oid] = {
            "id": oid,
            "status": status,
            "trust": trust,
            "trace": row.get("trace") or [],
        }
    return out


def default_unknown(oid):
    return {"id": oid, "status": "unknown", "trust": "checked", "trace": []}


def write_json(path, value):
    parent = os.path.dirname(path)
    if parent:
        os.makedirs(parent, exist_ok=True)
    with open(path, "w") as handle:
        json.dump(value, handle)
        handle.write("\n")


def run_backend(cmd, machine, corpus, case_id):
    argv = cmd + [
        "--machine",
        machine,
        "--corpus",
        corpus,
        "--case-id",
        case_id,
    ]
    done = subprocess.run(argv, check=False, capture_output=True, text=True)
    if done.returncode != 0:
        err = done.stderr.strip() or done.stdout.strip() or f"exit {done.returncode}"
        raise RuntimeError(f"backend failed for {case_id}: {err}")
    return index_rows(as_rows(json.loads(done.stdout)))


def scenario_doc(machine_name, trace):
    steps = []
    for index, step in enumerate(trace):
        steps.append(
            {
                "kind": "command",
                "label": f"s{index}",
                "name": step["command"],
                "args": step.get("args") or {},
                "expect": "committed",
            }
        )
    return {
        "scenario_version": 1,
        "machine": machine_name,
        "aggregate": "truth-1",
        "steps": steps,
    }


def replay_trace(ik, machine, trace):
    if not trace:
        return False, "empty trace"
    if not ik:
        return False, "ik binary not provided"
    root = tempfile.mkdtemp(prefix="ik-feasibility-replay-")
    try:
        shutil.copy(machine, os.path.join(root, "machine.ik.toml"))
        scen = os.path.join(root, "scenario.json")
        write_json(scen, scenario_doc("machine.ik.toml", trace))
        out = os.path.join(root, "out")
        done = subprocess.run(
            [ik, "scenario", "run", os.path.join(root, "machine.ik.toml"), scen, "--out", out],
            check=False,
            capture_output=True,
            text=True,
        )
        if done.returncode == 0:
            return True, "committed"
        detail = (done.stderr or done.stdout or "").strip().splitlines()
        return False, detail[-1] if detail else f"exit {done.returncode}"
    finally:
        shutil.rmtree(root, ignore_errors=True)


def score_obligation(truth_row, backend_row, replay):
    truth = truth_row["truth"]
    status = backend_row["status"]
    trust = backend_row["trust"]
    result = {
        "id": truth_row["id"],
        "truth": truth,
        "status": status,
        "trust": trust,
        "fatal": None,
        "true_positive": False,
        "solver_asserted": trust == "solver" and status != "unknown",
    }
    if status == "impossible" and truth == "REACHABLE":
        result["fatal"] = "UNSOUND: impossible on REACHABLE"
        return result
    if status == "witness":
        result["fatal"] = witness_fatal(truth, backend_row, replay)
        return result
    if status == "impossible" and truth == "UNREACHABLE":
        result["true_positive"] = True
    return result


def witness_fatal(truth, backend_row, replay):
    ok, reason = replay(backend_row.get("trace") or [])
    if not ok:
        return f"witness did not replay: {reason}"
    if truth == "UNREACHABLE":
        return "UNSOUND: replaying witness on UNREACHABLE"
    return None


def score_case(case, backend_rows, replay):
    fatals = []
    rows = []
    unreachable = 0
    proved = 0
    solver = 0
    for obligation in case["obligations"]:
        oid = obligation["id"]
        backend_row = backend_rows.get(oid) or default_unknown(oid)
        scored = score_obligation(obligation, backend_row, replay)
        rows.append(scored)
        if obligation["truth"] == "UNREACHABLE":
            unreachable += 1
            if scored["true_positive"]:
                proved += 1
        if scored["solver_asserted"]:
            solver += 1
        if scored["fatal"]:
            fatals.append(f"{case['id']}:{oid}: {scored['fatal']}")
    return {
        "id": case["id"],
        "fatal": fatals,
        "rows": rows,
        "unreachable": unreachable,
        "proved": proved,
        "solver_asserted": solver,
    }


def summarize(case_results):
    fatals = []
    unreachable = 0
    proved = 0
    solver = 0
    for item in case_results:
        fatals.extend(item["fatal"])
        unreachable += item["unreachable"]
        proved += item["proved"]
        solver += item["solver_asserted"]
    precision = None if unreachable == 0 else proved / unreachable
    return {
        "unsound": len(fatals) > 0,
        "fatals": fatals,
        "unreachable": unreachable,
        "proved": proved,
        "precision": precision,
        "solver_asserted": solver,
        "checked_true_positives": proved,
    }


def fake_wrong(truth):
    rows = []
    marked = False
    for case in truth["cases"]:
        for obligation in case["obligations"]:
            status = "unknown"
            if not marked and obligation["truth"] == "REACHABLE":
                status = "impossible"
                marked = True
            rows.append(
                {"id": obligation["id"], "status": status, "trust": "solver"}
            )
    return rows


def fake_unknown(truth):
    rows = []
    for case in truth["cases"]:
        for obligation in case["obligations"]:
            rows.append(
                {
                    "id": obligation["id"],
                    "status": "unknown",
                    "trust": "checked",
                }
            )
    return rows


def score_all(truth, rows_by_id, replay):
    results = []
    for case in truth["cases"]:
        subset = {
            obligation["id"]: rows_by_id.get(obligation["id"]) or default_unknown(obligation["id"])
            for obligation in case["obligations"]
        }
        results.append(score_case(case, subset, replay))
    return summarize(results), results


def self_test(truth):
    problems = []
    def no_replay(_trace):
        return False, "self-test does not replay"

    wrong = index_rows(fake_wrong(truth))
    summary, _ = score_all(truth, wrong, no_replay)
    if not summary["unsound"]:
        problems.append("wrong backend was not caught")
    unknown = index_rows(fake_unknown(truth))
    summary_u, _ = score_all(truth, unknown, no_replay)
    if summary_u["unsound"]:
        problems.append("always-unknown backend was marked unsound")
    if summary_u["precision"] not in (0, 0.0):
        problems.append(f"always-unknown precision {summary_u['precision']} != 0")
    return problems, summary, summary_u


def resolve_root(args, truth_path):
    if args.root:
        return Path(args.root)
    return Path(truth_path).resolve().parent


def split_cmd(raw):
    import shlex

    return shlex.split(raw)


def live_replay(ik, machine):
    def inner(trace):
        return replay_trace(ik, machine, trace)

    return inner


def run_live(args, truth):
    root = resolve_root(args, args.truth)
    cmd = split_cmd(args.backend)
    ik = args.ik
    results = []
    for case in truth["cases"]:
        machine = str(root / case["machine"])
        corpus = str(root / case["corpus"])
        rows = run_backend(cmd, machine, corpus, case["id"])
        results.append(score_case(case, rows, live_replay(ik, machine)))
    return summarize(results), results


def emit(summary, extra, as_json):
    payload = dict(summary)
    payload.update(extra)
    if as_json:
        json.dump(payload, sys.stdout)
        sys.stdout.write("\n")
        return
    print(f"unsound={payload['unsound']}")
    print(f"precision={payload['precision']}")
    print(f"proved={payload['proved']} unreachable={payload['unreachable']}")
    print(f"solver_asserted={payload['solver_asserted']}")
    for line in payload.get("fatals") or []:
        print(f"fatal: {line}")
    for line in payload.get("self_test_problems") or []:
        print(f"self-test: {line}")


def main(argv):
    args = parse_args(argv)
    truth = load_json(args.truth)
    if args.self_test:
        problems, wrong, unknown = self_test(truth)
        emit(
            {
                "unsound": bool(problems),
                "fatals": problems,
                "precision": unknown["precision"],
                "proved": unknown["proved"],
                "unreachable": unknown["unreachable"],
                "solver_asserted": wrong["solver_asserted"],
            },
            {
                "self_test_problems": problems,
                "wrong_backend_caught": wrong["unsound"],
                "unknown_backend_sound": not unknown["unsound"],
                "unknown_precision": unknown["precision"],
            },
            args.json,
        )
        return 1 if problems else 0
    if not args.backend:
        print("either --backend CMD or --self-test is required", file=sys.stderr)
        return 2
    summary, _results = run_live(args, truth)
    emit(summary, {}, args.json)
    return 1 if summary["unsound"] else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
