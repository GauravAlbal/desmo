"""Scorer for batch dfy-016 (focused check: expressible part of an inexpressible contract grammar). Holds no secrets; the lead decrypts evaluators between agent waves.

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
BATCH = "batch-016"
EVAL = D / "oracle-eval"
FAMS = ("N22", "N23")
REPS = ("r1", "r2")
RUNS = os.environ.get("B16_RUNS", "runs-b16")
IKBIN = str(D / BATCH / "ik-packed")
ENV = dict(os.environ, DFY_CAND="cand17", DFY_BATCH=BATCH, DFY_RUNS=RUNS, DFY_IKBIN=IKBIN, IK_BINARY=IKBIN,
           PATH=NB + ":" + os.environ["PATH"])

BUILD_RULES = {
    "N22": ('cart_total', 'air_at', 'air_gate', 'band_set', 'mic_bound', 'producer_id', 'host_id', 'studio_matches'),
    "N23": ('fill_units', 'due_at', 'receive_gate', 'item_set', 'carton_bound', 'clerk_id', 'reviewer_id', 'received_n'),
}
CHANGE_RULES = {
    "N22": ('cart_total', 'air_at', 'air_gate', 'band_set', 'mic_bound', 'producer_id', 'host_id', 'studio_matches', 'extra_cart', 'kill_clear', 'kill_note', 'engineer_id'),
    "N23": ('fill_units', 'due_at', 'receive_gate', 'item_set', 'carton_bound', 'clerk_id', 'reviewer_id', 'received_n', 'extra_buffer', 'void_clear', 'void_note', 'pharmacist_id'),
}
INPUT_VALIDITY = {"N22": ('band_set', 'mic_bound', 'producer_id', 'host_id', 'kill_note', 'engineer_id'), "N23": ('item_set', 'carton_bound', 'clerk_id', 'reviewer_id', 'void_note', 'pharmacist_id')}
IDENTIFIER_RULES = {"N22": ('producer_id', 'host_id', 'engineer_id'), "N23": ('clerk_id', 'reviewer_id', 'pharmacist_id')}
BENIGN_ARGS = {"N22": ('show_tag', 'voice_color', 'desk_mark'), "N23": ('ticket_tag', 'shelf_code', 'lot_code')}
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
