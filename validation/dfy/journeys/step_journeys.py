#!/usr/bin/env python3
"""Run the development-journey state model against a packed ik binary.

Python 3 stdlib only. Asserts codes, rule ids, and status fields — never wording.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Tuple

HERE = Path(__file__).resolve().parent
MODEL_PATH = HERE / "journeys.json"
DEFAULT_IK = HERE.parents[1] / "batch-017" / "ik-packed"
BODY_KEYS = (
    "aggregate_id",
    "capability",
    "command",
    "operation_id",
    "observation",
    "effect",
    "commands",
)
ALLOWED_HANDOFF = (
    "exercised",
    "incomplete",
    "no_evidence_cited",
    "cited_evidence_not_found",
)
Session = Dict[str, Any]


def die(msg: str, code: int = 2) -> None:
    sys.stderr.write(msg + "\n")
    raise SystemExit(code)


def load_model() -> Dict[str, Any]:
    return json.loads(MODEL_PATH.read_text(encoding="utf-8"))


def resolve_ik() -> Path:
    env = os.environ.get("IK_BINARY")
    path = Path(env) if env else DEFAULT_IK
    if path.is_file():
        return path
    die("ik binary not found at %s (set IK_BINARY)" % path)
    return path


def as_index(part: str) -> Optional[int]:
    if part.isdigit():
        return int(part)
    return None


def take_index(cur: Any, idx: int) -> Any:
    if not isinstance(cur, list):
        return None
    if idx >= len(cur):
        return None
    return cur[idx]


def take_key(cur: Any, key: str) -> Any:
    if not isinstance(cur, dict):
        return None
    if key not in cur:
        return None
    return cur[key]


def step_path(cur: Any, part: str) -> Any:
    if cur is None:
        return None
    idx = as_index(part)
    if idx is not None:
        return take_index(cur, idx)
    return take_key(cur, part)


def get_path(obj: Any, path: str) -> Any:
    cur = obj
    for part in path.split("."):
        cur = step_path(cur, part)
    return cur


def subst_one(out: str, key: str, item: Any) -> str:
    token = "$" + key
    if token not in out:
        return out
    if not isinstance(item, str):
        return out
    return out.replace(token, item)


def subst_tokens(out: str, ctx: Dict[str, Any]) -> str:
    for key, item in ctx.items():
        out = subst_one(out, key, item)
    return out


def render_str(value: str, ctx: Dict[str, Any]) -> Any:
    if not value.startswith("$"):
        return subst_tokens(value, ctx)
    name = value[1:]
    if name not in ctx:
        return subst_tokens(value, ctx)
    return ctx[name]


def render_list(value: List[Any], ctx: Dict[str, Any]) -> List[Any]:
    return [render(v, ctx) for v in value]


def render_dict(value: Dict[str, Any], ctx: Dict[str, Any]) -> Dict[str, Any]:
    return {k: render(v, ctx) for k, v in value.items()}


RENDER_TYPES: Dict[type, Callable[[Any, Dict[str, Any]], Any]] = {
    str: render_str,
    list: render_list,
    dict: render_dict,
}


def render(value: Any, ctx: Dict[str, Any]) -> Any:
    fn = RENDER_TYPES.get(type(value))
    if fn is None:
        return value
    return fn(value, ctx)


def loads_or_none(text: str) -> Any:
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return None


def parse_json(text: str) -> Any:
    text = text.strip()
    if not text:
        return None
    return loads_or_none(text)


def redact_str(value: str, mapping: List[Tuple[str, str]]) -> str:
    out = value
    for old, new in mapping:
        out = out.replace(old, new)
    return out


def redact_list(value: List[Any], mapping: List[Tuple[str, str]]) -> List[Any]:
    return [redact(v, mapping) for v in value]


def redact_dict(value: Dict[str, Any], mapping: List[Tuple[str, str]]) -> Dict[str, Any]:
    return {k: redact(v, mapping) for k, v in value.items()}


REDACT_TYPES: Dict[type, Callable[[Any, List[Tuple[str, str]]], Any]] = {
    str: redact_str,
    list: redact_list,
    dict: redact_dict,
}


def redact(value: Any, mapping: List[Tuple[str, str]]) -> Any:
    fn = REDACT_TYPES.get(type(value))
    if fn is None:
        return value
    return fn(value, mapping)


def run_ik(
    ik: Path,
    argv: List[str],
    cwd: Path,
    stdin: Optional[str] = None,
) -> subprocess.CompletedProcess:
    return subprocess.run(
        [str(ik), *argv],
        input=stdin,
        text=True,
        capture_output=True,
        cwd=str(cwd),
    )


def copy_body_key(req: Dict[str, Any], body: Dict[str, Any], key: str) -> None:
    if key not in body:
        return
    req[key] = body[key]


def envelope(op: str, ident: Dict[str, Any], body: Dict[str, Any], runtime: str) -> str:
    req = {
        "protocol": {"major": 1, "minor": 0},
        "runtime_version": runtime,
        "op": op,
        "machine": ident,
        "request_id": body.get("request_id", "r"),
    }
    for key in BODY_KEYS:
        copy_body_key(req, body, key)
    return json.dumps(req, separators=(",", ":"))


def mismatch_eq(path: str, got: Any, want: Any) -> List[str]:
    if got == want:
        return []
    return ["%s: %r != %r" % (path, got, want)]


def present_ok(got: Any) -> bool:
    if got is None:
        return False
    if got == []:
        return False
    if got == "":
        return False
    return True


def missing_path(path: str, got: Any) -> List[str]:
    if present_ok(got):
        return []
    return ["missing %s" % path]


def assert_exit(expect: Dict[str, Any], parsed: Any, proc: subprocess.CompletedProcess) -> List[str]:
    want = expect.get("exit")
    if want is None:
        return []
    if proc.returncode == want:
        return []
    return ["exit %s != %s" % (proc.returncode, want)]


def assert_stdout_line(expect: Dict[str, Any], parsed: Any, proc: subprocess.CompletedProcess) -> List[str]:
    line = expect.get("stdout_line")
    if line is None:
        return []
    if proc.stdout.strip() == line:
        return []
    return ["stdout_line mismatch"]


def needs_json(expect: Dict[str, Any]) -> bool:
    if expect.get("json_eq"):
        return True
    if expect.get("json_present"):
        return True
    if expect.get("step_eq"):
        return True
    return False


def assert_json_required(expect: Dict[str, Any], parsed: Any, proc: subprocess.CompletedProcess) -> List[str]:
    if parsed is not None:
        return []
    if not needs_json(expect):
        return []
    return ["stdout is not JSON"]


def list_json_eq(parsed: Any, pairs: Dict[str, Any]) -> List[str]:
    failures: List[str] = []
    for path, want in pairs.items():
        failures.extend(mismatch_eq(path, get_path(parsed, path), want))
    return failures


def assert_json_eq(expect: Dict[str, Any], parsed: Any, proc: subprocess.CompletedProcess) -> List[str]:
    if parsed is None:
        return []
    return list_json_eq(parsed, expect.get("json_eq") or {})


def list_json_present(parsed: Any, paths: List[str]) -> List[str]:
    failures: List[str] = []
    for path in paths:
        failures.extend(missing_path(path, get_path(parsed, path)))
    return failures


def assert_json_present(expect: Dict[str, Any], parsed: Any, proc: subprocess.CompletedProcess) -> List[str]:
    if parsed is None:
        return []
    return list_json_present(parsed, expect.get("json_present") or [])


ASSERTIONS = (
    assert_exit,
    assert_stdout_line,
    assert_json_required,
    assert_json_eq,
    assert_json_present,
)


def collect_failures(expect: Dict[str, Any], parsed: Any, proc: subprocess.CompletedProcess) -> List[str]:
    failures: List[str] = []
    for fn in ASSERTIONS:
        failures.extend(fn(expect, parsed, proc))
    return failures


def step_by_label(result: Dict[str, Any]) -> Dict[Any, Any]:
    by_label: Dict[Any, Any] = {}
    for row in result.get("steps") or []:
        by_label[row.get("label")] = row
    return by_label


def one_step_eq(by_label: Dict[Any, Any], path: str, want: Any) -> List[str]:
    label, rest = path.split(".", 1)
    row = by_label.get(label)
    if row is None:
        return ["no step label %s" % label]
    return mismatch_eq(path, get_path(row, rest), want)


def step_eq_failures(result: Dict[str, Any], expect: Dict[str, str]) -> List[str]:
    by_label = step_by_label(result)
    failures: List[str] = []
    for path, want in expect.items():
        failures.extend(one_step_eq(by_label, path, want))
    return failures


def add_step_eq(failures: List[str], parsed: Any, expect: Dict[str, Any]) -> None:
    if parsed is None:
        return
    failures.extend(step_eq_failures(parsed, expect.get("step_eq") or {}))


def record(
    step_id: str,
    argv: List[str],
    proc: subprocess.CompletedProcess,
    parsed: Any,
    failures: List[str],
    extra: Optional[Dict[str, Any]] = None,
) -> Dict[str, Any]:
    row = {
        "id": step_id,
        "argv": argv,
        "exit": proc.returncode,
        "stdout": proc.stdout,
        "stderr": proc.stderr,
        "parsed": parsed,
        "pass": not failures,
        "failures": failures,
    }
    if extra:
        row.update(extra)
    return row


def cached_ident(ctx: Dict[str, Any], key: Any) -> Optional[Dict[str, Any]]:
    if not key:
        return None
    val = ctx.get(key)
    if isinstance(val, dict):
        return val
    return None


def die_identity(src: Any, proc: subprocess.CompletedProcess) -> None:
    die("identity probe failed for %s: %s" % (src, proc.stderr or proc.stdout))


def probe_ident(
    ik: Path,
    cwd: Path,
    ctx: Dict[str, Any],
    step: Dict[str, Any],
    key: Any,
) -> Dict[str, Any]:
    src = step.get("identity_from")
    if not src:
        die("step %s missing identity" % step.get("id"))
    proc = run_ik(ik, src, cwd)
    ident = parse_json(proc.stdout)
    if proc.returncode != 0:
        die_identity(src, proc)
    if not isinstance(ident, dict):
        die_identity(src, proc)
    if key:
        ctx[key] = ident
    return ident


def ensure_ident(ik: Path, cwd: Path, ctx: Dict[str, Any], step: Dict[str, Any]) -> Dict[str, Any]:
    key = step.get("identity")
    cached = cached_ident(ctx, key)
    if cached is not None:
        return cached
    return probe_ident(ik, cwd, ctx, step, key)


def run_store_op(
    ik: Path,
    cwd: Path,
    ctx: Dict[str, Any],
    runtime: str,
    store: Path,
    machine: str,
    ident: Dict[str, Any],
    op: str,
    body: Dict[str, Any],
) -> Tuple[subprocess.CompletedProcess, Any]:
    stdin = envelope(op, ident, render(body, ctx), runtime)
    proc = run_ik(ik, ["--store", str(store), "--machine", machine, op], cwd, stdin=stdin)
    return proc, parse_json(proc.stdout)


def save_parsed(session: Session, step: Dict[str, Any], parsed: Any, failures: List[str]) -> None:
    if failures:
        return
    if not isinstance(parsed, dict):
        return
    key = step.get("save")
    if not key:
        return
    session["ctx"][key] = parsed


def save_any(session: Session, step: Dict[str, Any], parsed: Any) -> None:
    key = step.get("save")
    if not key:
        return
    if parsed is None:
        return
    session["ctx"][key] = parsed


def save_cap(session: Session, spec: Dict[str, Any], parsed: Any) -> None:
    key = spec.get("save_cap")
    if not key:
        return
    if not isinstance(parsed, dict):
        return
    cap = get_path(parsed, "result.capability")
    if cap is None:
        return
    session["ctx"][key] = cap


def ensure_out_dir(argv: List[str]) -> None:
    if "--out" not in argv:
        return
    Path(argv[argv.index("--out") + 1]).mkdir(parents=True, exist_ok=True)


def load_result(step: Dict[str, Any], session: Session) -> Any:
    result_rel = step.get("result")
    if not result_rel:
        return None
    result_path = Path(render(result_rel, session["ctx"]))
    if not result_path.is_file():
        return None
    return json.loads(result_path.read_text(encoding="utf-8"))


def handle_cli(session: Session, step: Dict[str, Any]) -> None:
    argv = render(step["argv"], session["ctx"])
    proc = run_ik(session["ik"], argv, session["cwd"])
    parsed = parse_json(proc.stdout)
    failures = collect_failures(step["expect"], parsed, proc)
    save_parsed(session, step, parsed, failures)
    session["rows"].append(record(step["id"], argv, proc, parsed, failures))


def handle_scenario(session: Session, step: Dict[str, Any]) -> None:
    argv = render(step["argv"], session["ctx"])
    ensure_out_dir(argv)
    proc = run_ik(session["ik"], argv, session["cwd"])
    parsed = load_result(step, session)
    failures = collect_failures(step["expect"], parsed, proc)
    add_step_eq(failures, parsed, step["expect"])
    save_any(session, step, parsed)
    session["rows"].append(record(step["id"], argv, proc, parsed, failures, {"stdout": proc.stdout}))


def store_argv(step: Dict[str, Any], op: str) -> List[str]:
    return ["--store", step["store"], "--machine", step["machine"], op]


def handle_store(session: Session, step: Dict[str, Any]) -> None:
    ident = ensure_ident(session["ik"], session["cwd"], session["ctx"], step)
    store = session["cwd"] / step["store"]
    proc, parsed = run_store_op(
        session["ik"],
        session["cwd"],
        session["ctx"],
        session["runtime"],
        store,
        step["machine"],
        ident,
        step["op"],
        step["body"],
    )
    failures = collect_failures(step["expect"], parsed, proc)
    save_cap(session, step, parsed)
    session["rows"].append(record(step["id"], store_argv(step, step["op"]), proc, parsed, failures))


def last_or_version(session: Session, last_proc: Optional[subprocess.CompletedProcess]) -> subprocess.CompletedProcess:
    if last_proc is not None:
        return last_proc
    return run_ik(session["ik"], ["--version"], session["cwd"])


def extend_op_fail(seq_fail: List[str], op: str, op_fail: List[str]) -> None:
    for item in op_fail:
        seq_fail.append("%s: %s" % (op, item))


def run_one_seq(
    session: Session,
    step: Dict[str, Any],
    ident: Dict[str, Any],
    store: Path,
    op_spec: Dict[str, Any],
    seq_rows: List[Dict[str, Any]],
    seq_fail: List[str],
) -> Tuple[subprocess.CompletedProcess, Any]:
    proc, parsed = run_store_op(
        session["ik"],
        session["cwd"],
        session["ctx"],
        session["runtime"],
        store,
        step["machine"],
        ident,
        op_spec["op"],
        op_spec["body"],
    )
    op_fail = collect_failures(op_spec["expect"], parsed, proc)
    save_cap(session, op_spec, parsed)
    seq_rows.append(
        {
            "op": op_spec["op"],
            "exit": proc.returncode,
            "parsed": parsed,
            "failures": op_fail,
            "pass": not op_fail,
        }
    )
    extend_op_fail(seq_fail, op_spec["op"], op_fail)
    return proc, parsed


def handle_store_seq(session: Session, step: Dict[str, Any]) -> None:
    ident = ensure_ident(session["ik"], session["cwd"], session["ctx"], step)
    store = session["cwd"] / step["store"]
    seq_rows: List[Dict[str, Any]] = []
    seq_fail: List[str] = []
    last_proc = None
    last_parsed = None
    for op_spec in step["ops"]:
        last_proc, last_parsed = run_one_seq(session, step, ident, store, op_spec, seq_rows, seq_fail)
    dummy = last_or_version(session, last_proc)
    session["rows"].append(
        record(
            step["id"],
            store_argv(step, "create|command|show"),
            dummy,
            {"ops": seq_rows, "last": last_parsed},
            seq_fail,
        )
    )


def extend_prefixed(failures: List[str], prefix: str, items: List[str]) -> None:
    for item in items:
        failures.append("%s: %s" % (prefix, item))


def run_report(session: Session, extra: Dict[str, Any], expect: Dict[str, Any], failures: List[str]) -> subprocess.CompletedProcess:
    argv = ["report", str(session["tmp"] / "out" / "sc-lost"), "--json"]
    proc = run_ik(session["ik"], argv, session["cwd"])
    report = parse_json(proc.stdout)
    extra["report"] = report
    extra["report_exit"] = proc.returncode
    extra["report_stderr"] = proc.stderr
    extra["report_stdout"] = proc.stdout
    extend_prefixed(failures, "report", collect_failures(expect["report"], report, proc))
    return proc


def finding_id_of(finding: Dict[str, Any]) -> str:
    fid = finding.get("id") or ""
    if isinstance(fid, str):
        return fid
    return ""


def first_finding_suffix(report: Dict[str, Any], suffix: str) -> Optional[str]:
    for finding in report.get("findings") or []:
        fid = finding_id_of(finding)
        if fid.endswith(suffix):
            return fid
    return None


def first_finding_contains(report: Dict[str, Any], needle: str) -> Optional[str]:
    for finding in report.get("findings") or []:
        fid = finding_id_of(finding)
        if needle in fid:
            return fid
    return None


def finding_id_from(report: Any) -> Optional[str]:
    if not isinstance(report, dict):
        return None
    found = first_finding_suffix(report, "/found")
    if found is not None:
        return found
    return first_finding_contains(report, "/tag")


def evidence_list(finding_id: Optional[str]) -> List[str]:
    if finding_id:
        return [finding_id]
    return []


def write_behavior(cwd: Path, finding_id: Optional[str]) -> Path:
    behavior = {
        "behavior_version": 1,
        "title": "lost reply is history",
        "outcomes": [
            {
                "id": "R1",
                "text": "lookup of the dropped commit is committed_exact",
                "evidence": evidence_list(finding_id),
            }
        ],
        "unchanged": [],
        "assumptions": [],
    }
    path = cwd / "behavior.json"
    path.write_text(json.dumps(behavior), encoding="utf-8")
    return path


def check_handoff_status(failures: List[str], handoff: Any) -> None:
    if not isinstance(handoff, dict):
        return
    outcomes = handoff.get("outcomes") or []
    if not outcomes:
        return
    status = outcomes[0].get("status")
    if status in ALLOWED_HANDOFF:
        return
    failures.append("handoff: unexpected claim status %r" % status)


def run_handoff(session: Session, extra: Dict[str, Any], expect: Dict[str, Any], failures: List[str]) -> None:
    finding_id = finding_id_from(extra.get("report"))
    extra["finding_id"] = finding_id
    behavior_path = write_behavior(session["cwd"], finding_id)
    argv = [
        "handoff",
        str(behavior_path),
        "--evidence",
        str(session["tmp"] / "out" / "sc-lost"),
        "--kind",
        "recovery",
        "--json",
    ]
    proc_h = run_ik(session["ik"], argv, session["cwd"])
    handoff = parse_json(proc_h.stdout)
    extra["handoff"] = handoff
    extra["handoff_exit"] = proc_h.returncode
    extra["handoff_stderr"] = proc_h.stderr
    extend_prefixed(failures, "handoff", collect_failures(expect["handoff"], handoff, proc_h))
    check_handoff_status(failures, handoff)


def tagged_store_op(
    session: Session,
    ident: Dict[str, Any],
    op: str,
    body: Dict[str, Any],
) -> Tuple[subprocess.CompletedProcess, Any]:
    store = session["cwd"] / "tagged.sqlite"
    return run_store_op(
        session["ik"],
        session["cwd"],
        session["ctx"],
        session["runtime"],
        store,
        "fixtures/tagged_item.ik.toml",
        ident,
        op,
        body,
    )


def run_replay_verify(session: Session, extra: Dict[str, Any], expect: Dict[str, Any], failures: List[str]) -> None:
    ident = session["ctx"].get("ident_tagged")
    if not isinstance(ident, dict):
        failures.append("replay: missing ident_tagged")
        return
    proc_r, replay = tagged_store_op(session, ident, "replay", {"aggregate_id": "item-1", "request_id": "rp"})
    extra["replay"] = replay
    extend_prefixed(failures, "replay", collect_failures(expect["replay"], replay, proc_r))
    proc_v, verify = tagged_store_op(session, ident, "verify", {"aggregate_id": "item-1", "request_id": "vf"})
    extra["verify"] = verify
    extend_prefixed(failures, "verify", collect_failures(expect["verify"], verify, proc_v))


def handle_recover(session: Session, step: Dict[str, Any]) -> None:
    expect = step["expect"]
    extra: Dict[str, Any] = {}
    failures: List[str] = []
    proc = run_report(session, extra, expect, failures)
    run_handoff(session, extra, expect, failures)
    run_replay_verify(session, extra, expect, failures)
    session["rows"].append(
        record(
            step["id"],
            ["report", "handoff", "replay", "verify"],
            proc,
            extra,
            failures,
            extra,
        )
    )


HANDLERS: Dict[str, Callable[[Session, Dict[str, Any]], None]] = {
    "cli": handle_cli,
    "scenario": handle_scenario,
    "store": handle_store,
    "store_seq": handle_store_seq,
    "recover": handle_recover,
}


def run_step(session: Session, step: Dict[str, Any]) -> None:
    fn = HANDLERS.get(step["kind"])
    if fn is None:
        die("unknown step kind %s" % step["kind"])
    fn(session, step)


def run_all_steps(session: Session) -> None:
    for step in session["model"]["steps"]:
        run_step(session, step)


def add_path_token(pairs: List[Tuple[str, str]], path: Path, token: str) -> None:
    text = str(path)
    pairs.append((text, token))
    real = os.path.realpath(text)
    if real == text:
        return
    pairs.append((real, token))


def mapping_for(tmp: Path, ik: Path) -> List[Tuple[str, str]]:
    pairs: List[Tuple[str, str]] = []
    add_path_token(pairs, tmp, "$TMP")
    add_path_token(pairs, HERE, "$JOURNEYS")
    add_path_token(pairs, ik, "$IK")
    add_path_token(pairs, Path.home(), "$HOME")
    return pairs


def make_session() -> Session:
    model = load_model()
    ik = resolve_ik()
    tmp = Path(tempfile.mkdtemp(prefix="ik-journeys-"))
    (tmp / "out").mkdir()
    shutil.copytree(HERE / "fixtures", tmp / "work" / "fixtures")
    return {
        "model": model,
        "ik": ik,
        "runtime": model["runtime_version"],
        "tmp": tmp,
        "cwd": tmp / "work",
        "ctx": {"OUT": str(tmp / "out"), "HERE": str(HERE)},
        "rows": [],
        "mapping": mapping_for(tmp, ik),
    }


def count_passed(rows: List[Dict[str, Any]]) -> int:
    n = 0
    for row in rows:
        n += int(row["pass"])
    return n


def append_failed_id(ids: List[str], row: Dict[str, Any]) -> None:
    if row["pass"]:
        return
    ids.append(row["id"])


def failed_ids(rows: List[Dict[str, Any]]) -> List[str]:
    ids: List[str] = []
    for row in rows:
        append_failed_id(ids, row)
    return ids


def write_transcript(session: Session) -> Tuple[int, List[str], int]:
    rows = session["rows"]
    passed = count_passed(rows)
    failed = failed_ids(rows)
    doc = {
        "schema": "ik.journey-transcript.v1",
        "ik": "$IK",
        "runtime": session["runtime"],
        "steps": redact(rows, session["mapping"]),
        "passed": passed,
        "total": len(rows),
        "failed": failed,
    }
    (HERE / "transcript.json").write_text(json.dumps(doc, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return passed, failed, len(rows)


def emit_failure(row: Dict[str, Any]) -> None:
    if row["pass"]:
        return
    sys.stderr.write("  %s: %s\n" % (row["id"], "; ".join(row["failures"])))


def print_failures(rows: List[Dict[str, Any]]) -> None:
    for row in rows:
        emit_failure(row)


def finish(session: Session, passed: int, failed: List[str], total: int) -> int:
    if not failed:
        sys.stdout.write("PASS %s/%s\n" % (passed, total))
        return 0
    sys.stderr.write("FAILED %s (%s/%s)\n" % (", ".join(failed), passed, total))
    print_failures(session["rows"])
    return 1


def run_session_body(session: Session) -> int:
    run_all_steps(session)
    passed, failed, total = write_transcript(session)
    return finish(session, passed, failed, total)


def run_session(session: Session) -> int:
    try:
        return run_session_body(session)
    finally:
        shutil.rmtree(session["tmp"], ignore_errors=True)


def main() -> int:
    session = make_session()
    return run_session(session)


if __name__ == "__main__":
    raise SystemExit(main())
