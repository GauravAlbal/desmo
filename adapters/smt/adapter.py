#!/usr/bin/python3
"""Pinned Z3 / cvc5 runner for SMT-LIB2 queries emitted by the kernel encoder.

The adapter does not translate a machine. It executes one query file, maps
sat/unsat/unknown onto the three-result JSON, and never touches the network.
"""

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import time

PLACEHOLDER = "0" * 64


def parse_args(argv):
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("command")
    parser.add_argument("--request")
    parser.add_argument("--out")
    parser.add_argument("--toolchain")
    return parser.parse_args(argv[1:])


def read_json(path):
    with open(path) as handle:
        return json.load(handle)


def write_text(path, text):
    parent = os.path.dirname(path)
    if parent:
        os.makedirs(parent, exist_ok=True)
    with open(path, "w") as handle:
        handle.write(text)


def write_json(path, value):
    write_text(path, json.dumps(value, indent=2) + "\n")


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_toolchain(path, script):
    chosen = path or os.path.join(os.path.dirname(os.path.abspath(script)), "toolchain.json")
    with open(chosen) as handle:
        data = json.load(handle)
    return {key: os.path.expanduser(val) if key in ("z3", "cvc5") else val for key, val in data.items()}


def present(path):
    return bool(path) and os.path.isfile(path) and os.access(path, os.X_OK)


def probe_one(path, flag):
    try:
        done = subprocess.run([path, flag], capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.TimeoutExpired):
        return None
    text = (done.stdout or "") + (done.stderr or "")
    return text.strip().splitlines()[0] if text.strip() else None


def digest_or_placeholder(path):
    if not present(path):
        return PLACEHOLDER
    return sha256_file(path)


def pin_ok(path, expected):
    if not expected or expected == PLACEHOLDER:
        return True
    return digest_or_placeholder(path) == expected


def cmd_describe(toolchain):
    z3 = toolchain.get("z3")
    cvc5 = toolchain.get("cvc5")
    body = {
        "adapter": "smt-lib2",
        "z3": probe_one(z3, "--version") if present(z3) else None,
        "cvc5": probe_one(cvc5, "--version") if present(cvc5) else None,
        "network": False,
    }
    json.dump(body, sys.stdout)
    sys.stdout.write("\n")
    if body["z3"] is None and body["cvc5"] is None:
        sys.exit(1)


def goal_rows(req):
    return [{"id": goal["id"], "content_digest": goal["content_digest"]} for goal in req["goals"]]


def input_rows(req):
    return [{"path": item["path"], "sha256": item["sha256"]} for item in req["inputs"]]


def required_id(req):
    for goal in req["goals"]:
        if goal.get("required"):
            return goal["id"]
    return req["goals"][0]["id"]


def base(req, execution):
    return {
        "protocol": "ik.analysis.result.v1",
        "request_id": req["request_id"],
        "run_id": req["run_id"],
        "subject": json.loads(json.dumps(req["subject"])),
        "goals": goal_rows(req),
        "profile_digest": req["profile_digest"],
        "inputs": input_rows(req),
        "execution_status": execution,
        "properties": [],
        "artifacts": [],
    }


def write_status(req, out, execution):
    write_json(os.path.join(out, "result.json"), base(req, execution))


def first_input(req):
    inputs = req.get("inputs") or []
    if not inputs:
        return None
    return inputs[0].get("path")


def query_path(req):
    bundle = req.get("bundle_dir") or ""
    relative = (req.get("controls") or {}).get("query") or first_input(req)
    if not relative:
        return None
    return os.path.join(bundle, relative)


def read_query(req):
    path = query_path(req)
    if not path or not os.path.isfile(path):
        return None
    with open(path) as handle:
        return handle.read()


def solver_cmd(name, path, timeout, query, horn):
    if name == "z3":
        cmd = [path, "-T:%s" % timeout]
        if horn:
            cmd.append("fp.engine=spacer")
        cmd.append(query)
        return cmd
    return [path, "--lang=smt2", "--tlimit=%s" % (timeout * 1000)]


def invoke_solver(name, path, query, timeout, horn):
    cmd = solver_cmd(name, path, timeout, query, horn)
    if name == "z3":
        return subprocess.run(
            cmd,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=timeout + 5,
        )
    with open(query, "rb") as handle:
        return subprocess.run(
            cmd,
            stdin=handle,
            capture_output=True,
            text=True,
            timeout=timeout + 5,
        )


def run_solver(name, path, smt2, timeout):
    started = time.time()
    query = None
    try:
        handle, query = tempfile.mkstemp(prefix="ik-smt-", suffix=".smt2")
        os.write(handle, smt2.encode())
        os.close(handle)
        done = invoke_solver(name, path, query, timeout, "set-logic HORN" in smt2)
    except (OSError, subprocess.TimeoutExpired) as err:
        return {
            "name": name,
            "reply": "unknown",
            "raw": str(err),
            "ms": int((time.time() - started) * 1000),
        }
    finally:
        if query:
            try:
                os.remove(query)
            except OSError:
                pass
    raw = (done.stdout or "") + (done.stderr or "")
    return {
        "name": name,
        "reply": classify(raw),
        "raw": raw,
        "ms": int((time.time() - started) * 1000),
    }


def classify(raw):
    for line in raw.splitlines():
        token = line.strip()
        if token in ("sat", "unsat", "unknown"):
            return token
    return "unknown"


def disagreed(z3, cvc5):
    if not z3 or not cvc5:
        return False
    if z3["reply"] == "unknown" or cvc5["reply"] == "unknown":
        return False
    return z3["reply"] != cvc5["reply"]


def map_reply(kind, reply, z3, cvc5):
    if kind == "horn" and reply == "sat":
        return "ProvenImpossible", "solver-asserted horn sat"
    if kind == "horn" and reply == "unsat":
        return "Unknown", "horn unsat; trace not reconstructed"
    if reply == "sat":
        return "Witness", "qf sat; kernel replay is the encoder's job"
    if reply == "unsat":
        return "ProvenImpossible", "qf unsat"
    return "Unknown", (z3 or cvc5 or {}).get("raw") or "no solver reply"


def three_result(kind, z3, cvc5):
    if disagreed(z3, cvc5):
        return "Unknown", "solvers disagree"
    reply = (z3 or cvc5 or {}).get("reply") or "unknown"
    return map_reply(kind, reply, z3, cvc5)


def host_finding(status):
    if status == "Witness":
        return "witness_found", "observed_trace", False, "model-level witness; kernel replay is the encoder's job"
    if status == "ProvenImpossible":
        return "no_witness_found", "bounded_paths", False, "impossibility is recorded in smt-result.json; host has no unbounded-proof finding"
    return "inconclusive", "bounded_paths", False, "unsupported, timeout, or incomplete"


def property_of(req, finding, evidence, complete, reason):
    return {
        "goal_id": required_id(req),
        "property_class": req["property_class"],
        "finding": finding,
        "evidence_class": evidence,
        "coverage": {"complete": complete, "reason": reason},
        "reproduction": {"status": "model_only"},
    }


def listed(out):
    names = ["result.json", "smt-result.json", "native/z3.txt", "native/cvc5.txt"]
    return [name for name in names if os.path.isfile(os.path.join(out, name))]


def write_native(out, name, text):
    path = os.path.join(out, "native", name)
    write_text(path, text or "")
    return path


def query_kind(req, smt2):
    asked = ((req.get("controls") or {}).get("question") or "").strip()
    if asked:
        return asked
    if "set-logic HORN" in smt2:
        return "horn"
    return "qf"


def finish_run(req, out, toolchain, smt2):
    timeout = int((req.get("limits") or {}).get("timeout_seconds") or 30)
    z3 = run_if_present("z3", toolchain, smt2, timeout)
    cvc5 = run_if_present("cvc5", toolchain, smt2, timeout)
    if z3 is None and cvc5 is None:
        write_status(req, out, "tool_unavailable")
        return
    kind = query_kind(req, smt2)
    status, why = three_result(kind, z3, cvc5)
    finding, evidence, complete, reason = host_finding(status)
    if z3:
        write_native(out, "z3.txt", z3["raw"])
    if cvc5:
        write_native(out, "cvc5.txt", cvc5["raw"])
    smt_doc = {
        "result": status,
        "why": why,
        "kind": kind,
        "z3": z3,
        "cvc5": cvc5,
        "disagreement": bool(z3 and cvc5 and z3["reply"] != cvc5["reply"]),
    }
    write_json(os.path.join(out, "smt-result.json"), smt_doc)
    document = base(req, "completed")
    document["properties"] = [property_of(req, finding, evidence, complete, reason)]
    document["artifacts"] = listed(out)
    write_json(os.path.join(out, "result.json"), document)


def run_if_present(name, toolchain, smt2, timeout):
    path = toolchain.get(name)
    expected = toolchain.get("%s_sha256" % name) or PLACEHOLDER
    if not present(path):
        return None
    if not pin_ok(path, expected):
        return {"name": name, "reply": "unknown", "raw": "digest mismatch", "ms": 0}
    return run_solver(name, path, smt2, timeout)


def cmd_run(request_path, out, toolchain):
    os.makedirs(out, exist_ok=True)
    req = read_json(request_path)
    smt2 = read_query(req)
    if smt2 is None:
        write_status(req, out, "unsupported")
        return
    finish_run(req, out, toolchain, smt2)


def main(argv):
    parsed = parse_args(argv)
    toolchain = load_toolchain(parsed.toolchain, argv[0])
    if parsed.command == "describe":
        cmd_describe(toolchain)
        return
    if parsed.command != "run" or not parsed.request or not parsed.out:
        sys.exit(2)
    cmd_run(parsed.request, parsed.out, toolchain)


if __name__ == "__main__":
    main(sys.argv)
