"""Scorer for batch dfy-012 (finite transfer check). Holds no secrets; the lead decrypts evaluators between agent waves.

Per replica and stage, runs the family evaluator once against the delivered project and reads its per-rule JSON lines
{toy, rule, criterion, pass}. First scoring is the record. Rule sets per stage are fixed in PREREG.md:
  BUILD  : rules that exist in the original brief, criteria 1 and 2.
  CHANGE : all rules, criteria 1 to 4 (3 for the invalidating rule, 4 for every rule, old instances from the
           trajectory's own build).
A rule is PASS only when every required criterion passes. A criterion 2 that the harness could not route to a command is
reported as 'unroutable' by the TAP text and is counted separately; it is never silently counted as pass.
"""
import json
import os
import pathlib
import signal
import subprocess

D = pathlib.Path("<work>")
NB = "<node-bin>"
NODE = NB + "/node"
RUNS = "runs-b12"
BATCH = "batch-012"
EVAL = D / "oracle-eval"
FAMS = ("N16", "N17")
REPS = ("r1", "r2")
RUNS = os.environ.get("B12_RUNS", "runs-b12")
IKBIN = str(D / BATCH / "ik-packed")
ENV = dict(os.environ, DFY_CAND="cand13", DFY_BATCH=BATCH, DFY_RUNS=RUNS, DFY_IKBIN=IKBIN, IK_BINARY=IKBIN,
           PATH=NB + ":" + os.environ["PATH"])

BUILD_RULES = {
    "N16": ("capture_deadline", "capture_cap", "fee", "refund_gate", "refund_amount"),
    "N17": ("payout_fee", "hold_until", "hold_release_gate", "dual_approval", "approval_revision"),
}
CHANGE_RULES = {
    "N16": BUILD_RULES["N16"] + ("void_clear", "daily_cap"),
    "N17": BUILD_RULES["N17"] + ("risk_clear", "dual_threshold"),
}
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
    if fam == "N16":
        return ["sh", str(EVAL / "N16" / "evaluator" / "run.sh")]
    return [NODE, "--test", "--test-timeout=120000", "--test-reporter=tap", str(EVAL / "N17" / "tests" / "evaluate.test.mjs")]


def parse_row(line):
    """One TAP line to a JSON row, or None when it is not a rule row."""
    line = line.strip()
    line = line[2:].strip() if line.startswith("# ") else line
    if not (line.startswith("{") and '"rule"' in line):
        return None
    try:
        return json.loads(line)
    except ValueError:
        return None


def is_delivered_row(row):
    return all(k in row for k in ("rule", "criterion", "pass")) and row.get("toy", "delivered") == "delivered"


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
    return {"pass": passed, "per_rule": per_rule, "runner_timeout": timed_out, "rows_seen": len(rows),
            "tap_tail": "" if passed else out[-1500:]}


def score_all(results):
    for fam in FAMS:
        for rep in REPS:
            for stage in ("build", "change"):
                rec = score_one(fam, rep, stage)
                results[f"{fam}/{rep}/{stage}"] = rec
                print(fam, rep, stage, "pass" if rec["pass"] else "FAIL", json.dumps(rec["per_rule"]))


def selftest():
    """Positives pass and negatives fail on the toys, using the packed candidate binary."""
    rc, out, err, to = runp(eval_command("N16")[:1] + [str(EVAL / "N16" / "evaluator" / "matrix.mjs")], ENV, 900, cwd=str(EVAL / "N16" / "evaluator"))
    print("N16 matrix rc", rc, out[-300:].replace("\n", " | "))
    rc2, out2, err2, to2 = runp(eval_command("N17"), dict(ENV, PROJECT_DIR=""), 900, cwd=str(EVAL / "N17"))
    print("N17 toys rc", rc2, out2[-300:].replace("\n", " | "))
    return rc == 0 and rc2 == 0
