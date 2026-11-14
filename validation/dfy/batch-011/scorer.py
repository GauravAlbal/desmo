"""Scoring helpers for the batch dfy-011 held-out qualification.

The lead decrypts the evaluator archives between agent waves; this module only runs them. It holds no secrets.
Per-stage rules (also stated in RULES.md):
  BUILD   passes when every R passes AND the runtime-use check passes (happy path, store sufficiency).
  CHANGE  passes when every R and C passes, the runtime-use check passes on the changed project in old-instance mode
          (pre-change instances created by the trajectory's own build), AND continuity passes.
  Evaluation files run under a per-test timeout and a process-group kill, then the family's own summarizer parses the TAP.
"""
import json
import os
import pathlib
import signal
import subprocess
import tempfile

D = pathlib.Path("<work>")
NB = "<home>/.nvm/versions/node/v22.17.1/bin"
NODE = NB + "/node"
RUNS = "runs-b11"
BATCH = "batch-011"
FAMS = ("N13", "N14", "N15")
REPS = ("r1", "r2")
CASES = ("a-lost-investigate", "b-lost-recover", "c-control-recover")
ENV = dict(os.environ, DFY_CAND="cand11", DFY_BATCH=BATCH, DFY_RUNS=RUNS, DFY_IKBIN=str(D / BATCH / "ik-packed"),
           IK_BINARY=str(D / BATCH / "ik-packed"), PATH=NB + ":" + os.environ["PATH"])


def runp(args, env, timeout, cwd=None):
    p = subprocess.Popen(args, cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
    try:
        out, err = p.communicate(timeout=timeout)
        return p.returncode, out, err, False
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGKILL)
        out, err = p.communicate()
        return -9, out or "", err or "", True


def parse(out):
    try:
        return json.loads(out[out.index("{"):out.rindex("}") + 1])
    except ValueError:
        return {"unparsed": out[-300:]}


def failed_reqs(d):
    r = d.get("requirements")
    return [k for k, v in r.items() if v != "pass"] if r else ["UNPARSED"]


def project(fam, rep, stage):
    return D / RUNS / fam / rep / stage / "project"


def authority(fam, proj, old=None):
    env = dict(ENV, PROJECT_DIR=str(proj), STUB_DIR=str(D / "projects" / fam / "stub"))
    if old:
        env["OLD_PROJECT_DIR"] = str(old)
    rc, out, err, to = runp([NODE, str(D / "projects" / fam / "authority" / "runtime_use.mjs")], env, 900, cwd=str(D / "projects" / fam))
    d = parse(out)
    return {"ok": bool(d.get("ok")) and rc == 0, "reasons": d.get("reasons", [d.get("unparsed", err[-200:])]), "timeout": to}


def tap_eval(fam, rep, stage):
    root = D / "oracle" / fam / "evaluator"
    env = dict(ENV, PROJECT_DIR=str(project(fam, rep, stage)), STUB_DIR=str(D / "oracle" / fam / "stub"))
    if stage == "change":
        env["OLD_PROJECT_DIR"] = str(project(fam, rep, "build"))
    files = [str(root / "accept.test.mjs")] + ([str(root / "change.test.mjs")] if stage == "change" else [])
    rc, out, err, timed_out = runp([NODE, "--test", "--test-timeout=120000", "--test-reporter=tap"] + files, env, 1500)
    tap = pathlib.Path(tempfile.mkdtemp(prefix="tap", dir="/tmp")) / "out.tap"
    tap.write_text(out)
    rc2, sout, serr, _ = runp([NODE, str(root / "summarize.mjs"), str(tap), "change" if stage == "change" else "initial"], env, 60)
    d = parse(sout)
    d["_runner_timeout"] = timed_out
    return d


def score_one(fam, rep, stage):
    d = tap_eval(fam, rep, stage)
    old = project(fam, rep, "build") if stage == "change" else None
    a = authority(fam, project(fam, rep, stage), old)
    f = failed_reqs(d)
    return {"failed": f, "authority": a, "pass": (not f) and a["ok"], "runner_timeout_after_tests": d.get("_runner_timeout")}


def score_stage(stage, results):
    for fam in FAMS:
        for rep in REPS:
            rec = score_one(fam, rep, stage)
            results[f"{fam}/{rep}/{stage}"] = rec
            print(fam, rep, stage, "failed", rec["failed"], "authority", rec["authority"]["ok"], rec["authority"]["reasons"])


def continuity_one(fam, rep):
    rc, out, err, to = runp(["/usr/bin/python3", "continuity_check.py", fam, rep], dict(ENV, DFY_RUNS=RUNS), 900, cwd=str(D))
    try:
        c = json.loads((out.strip().splitlines() or ["{}"])[-1])
    except ValueError:
        c = {"pass": False, "error": (out + err)[-300:]}
    keep = {k: c.get(k) for k in ("before", "after", "effects", "error")}
    return {"pass": bool(c.get("pass")), "detail": c if c.get("pass") else keep}


def continuity_all(results):
    for fam in FAMS:
        for rep in REPS:
            results[f"{fam}/{rep}/continuity"] = continuity_one(fam, rep)
            print(fam, rep, "continuity", results[f"{fam}/{rep}/continuity"]["pass"])


POSITIVE = {"N13": "controls/reference-toy-changed", "N14": "authority/reference-ikaros", "N15": "authority/reference-ikaros"}
OLD = {"N13": "controls/reference-toy", "N14": "authority/reference-ikaros-old", "N15": "controls/reference-toy"}


def selftest():
    """Instrument self-test run inside every unseal window before scoring: positives pass, negatives are detected."""
    ok = True
    for fam in FAMS:
        root = D / "projects" / fam
        for label, target, old, want in (("positive", POSITIVE[fam], OLD[fam], True), ("bypass-json", "authority/bypass-json", OLD[fam], False),
                                         ("shadow-state", "authority/shadow-state", OLD[fam], False), ("sidecar-authority", "authority/sidecar-authority", OLD[fam], False)):
            r = authority(fam, root / target, root / old)
            good = r["ok"] == want
            ok = ok and good
            print("selftest", fam, label, "ok" if good else "WRONG", r["ok"])
    return ok
