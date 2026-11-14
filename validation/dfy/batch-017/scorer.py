"""Scorer for batch dfy-017 families N24 and N25 (bounded string predicate / identifier grammar)."""
import json
import os
import pathlib
import signal
import subprocess

HELD = pathlib.Path("<held>")
D = pathlib.Path("<work>")
NB = "<node-bin>"
NODE = NB + "/node"
BATCH = "batch-017"
EVAL = HELD / "oracle-eval"
FAMS = ("N24", "N25")
REPS = ("r1", "r2")
RUNS = os.environ.get("B17_RUNS", "runs-b17")
IKBIN = str(D / BATCH / "ik-packed")
ENV = dict(os.environ, DFY_CAND="cand18", DFY_BATCH=BATCH, DFY_RUNS=RUNS, DFY_IKBIN=IKBIN, IK_BINARY=IKBIN,
           PATH=NB + ":" + os.environ["PATH"])


def _rules(fam):
    return json.loads((HELD / f"RULES-{fam}.json").read_text())


def _tuple(data, key):
    v = data.get(key)
    return tuple(v) if v else ()


BUILD_RULES = {}
CHANGE_RULES = {}
INPUT_VALIDITY = {}
IDENTIFIER_RULES = {}
GRAMMAR_RULES = {}
CHARSET_RULES = {}
BENIGN_ARGS = {}
GRAMMAR_TABLE = {}
for _fam in FAMS:
    _R = _rules(_fam)
    BUILD_RULES[_fam] = _tuple(_R, "BUILD_RULES")
    CHANGE_RULES[_fam] = _tuple(_R, "CHANGE_RULES")
    INPUT_VALIDITY[_fam] = _tuple(_R, "INPUT_VALIDITY")
    IDENTIFIER_RULES[_fam] = _tuple(_R, "IDENTIFIER_RULES")
    GRAMMAR_RULES[_fam] = _tuple(_R, "GRAMMAR_RULES")
    CHARSET_RULES[_fam] = _tuple(_R, "CHARSET_RULES")
    BENIGN_ARGS[_fam] = _tuple(_R, "BENIGN_ARGS")
    GRAMMAR_TABLE[_fam] = _R.get("GRAMMAR_TABLE") or {}
STAGE_CRITERIA = {"build": (1, 2), "change": (1, 2, 3, 4)}


def runp(args, env, timeout, cwd=None):
    p = subprocess.Popen(args, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                         start_new_session=True)
    try:
        out, err = p.communicate(timeout=timeout)
        return p.returncode, out, err, False
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGKILL)
        out, err = p.communicate()
        return -9, out or "", err or "", True


def project(fam, rep, stage):
    return D / RUNS / fam / rep / stage / "project"


def eval_command(fam):
    return [NODE, "--test", "--test-timeout=300000", "--test-reporter=tap", str(EVAL / fam / "tests" / "evaluate.test.mjs")]


def parse_row(line):
    line = line.strip()
    line = line[2:].strip() if line.startswith("# ") else line
    if not (line.startswith("{") and '"rule"' in line):
        return None
    try:
        return json.loads(line)
    except ValueError:
        return None


def is_rule_level(row):
    return "attack" not in row and "valid" not in row


def is_delivered_row(row):
    return all(k in row for k in ("rule", "criterion", "pass")) and row.get("toy", "delivered") == "delivered" and is_rule_level(row)


def rule_lines(out):
    rows = (parse_row(line) for line in out.splitlines())
    return [r for r in rows if r is not None and is_delivered_row(r)]


def required_rows(rows, fam, stage):
    names = BUILD_RULES[fam] if stage == "build" else CHANGE_RULES[fam]
    crits = STAGE_CRITERIA[stage]
    keep = [r for r in rows if r["rule"] in names and r["criterion"] in crits]
    return names, keep


def eval_env(fam, rep, stage):
    env = dict(ENV, PROJECT_DIR=str(project(fam, rep, stage)), STUB_DIR=str(D / "oracle" / fam / "stub"))
    if stage == "change":
        env["OLD_PROJECT_DIR"] = str(project(fam, rep, "build"))
    return env


def per_rule_results(names, rows):
    return {n: {str(r["criterion"]): bool(r["pass"]) for r in rows if r["rule"] == n} for n in names}


def all_pass(rows, per_rule):
    return bool(rows) and all(bool(r["pass"]) for r in rows) and all(per_rule.values())


def score_one(fam, rep, stage):
    rc, out, err, timed_out = runp(eval_command(fam), eval_env(fam, rep, stage), 1500)
    names, rows = required_rows(rule_lines(out), fam, stage)
    per_rule = per_rule_results(names, rows)
    passed = all_pass(rows, per_rule)
    c2fail = sorted(r for r, v in per_rule.items() if v.get("2") is False)
    return {"pass": passed, "per_rule": per_rule, "runner_timeout": timed_out, "rows_seen": len(rows),
            "c2fail": c2fail, "tap_tail": "" if passed else out[-2500:], "rc": rc}


def score_all(results):
    for fam in FAMS:
        for rep in REPS:
            for stage in ("build", "change"):
                rec = score_one(fam, rep, stage)
                results[f"{fam}/{rep}/{stage}"] = rec
                print(fam, rep, stage, "pass" if rec["pass"] else "FAIL", json.dumps(rec["per_rule"]))
