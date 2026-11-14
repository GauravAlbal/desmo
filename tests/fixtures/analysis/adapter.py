#!/usr/bin/python3
"""Conformance fixture for the analysis host.

Behavior is selected by controls.behavior in the request document. This process
does not read a behavior variable from the environment. A mock fixture proves
the host, not an analysis engine.
"""

import argparse
import json
import os
import subprocess
import sys
import time

MARKER = os.path.join(os.path.dirname(os.path.abspath(sys.argv[0])), "executed.marker")
open(MARKER, "w").close()


def parse_args(argv):
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("command")
    parser.add_argument("--request", required=True)
    parser.add_argument("--out", required=True)
    parsed = parser.parse_args(argv[1:])
    require_run(parsed.command)
    return parsed.request, parsed.out


def require_run(command):
    if command != "run":
        sys.exit(2)


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


def prop(req, finding, evidence, complete, reason, reproduction, artifact=None):
    return {
        "goal_id": required_id(req),
        "property_class": req["property_class"],
        "finding": finding,
        "evidence_class": evidence,
        "coverage": {"complete": complete, "reason": reason},
        "reproduction": {"status": reproduction, "artifact": artifact},
    }


def write_json(path, value):
    tmp = path + ".tmp"
    with open(tmp, "w") as handle:
        json.dump(value, handle)
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(tmp, path)


def record_env(out):
    path = os.path.join(out, "observed-env.txt")
    with open(path, "w") as handle:
        handle.write("\n".join(sorted(os.environ.keys())))
        handle.write("\n")


def spawn_sleeper(pid_file):
    child = subprocess.Popen(["/bin/sleep", "120"])
    if pid_file:
        write_pid_file(pid_file, child.pid)
    return child


def write_pid_file(pid_file, child_pid):
    with open(pid_file, "w") as handle:
        handle.write("%s\n%s\n" % (os.getpid(), child_pid))
        handle.flush()
        os.fsync(handle.fileno())


def correct_result(req):
    document = base(req, "completed")
    document["properties"] = [
        prop(req, "no_violation_found", "sampled_execution", False, "sample budget exhausted", "not_attempted")
    ]
    return document


def read_request(path):
    with open(path) as handle:
        return json.load(handle)


def controls_of(req):
    return req.get("controls") or {}


def load_context(request_path, out):
    os.makedirs(out, exist_ok=True)
    record_env(out)
    req = read_request(request_path)
    controls = controls_of(req)
    return {
        "req": req,
        "out": out,
        "result_path": os.path.join(out, "result.json"),
        "pid_file": controls.get("pid_file") or "",
        "behavior": controls.get("behavior", "correct"),
        "controls": controls,
    }


def do_correct(ctx):
    write_json(ctx["result_path"], correct_result(ctx["req"]))


def do_violation(ctx):
    document = base(ctx["req"], "completed")
    document["properties"] = [
        prop(ctx["req"], "violation", "sampled_execution", True, "counterexample in sample", "not_attempted")
    ]
    write_json(ctx["result_path"], document)
    sys.exit(2)


def do_reachability_witness(ctx):
    document = base(ctx["req"], "completed")
    document["properties"] = [
        prop(ctx["req"], "witness_found", "sampled_execution", False, "witness in sample", "model_only")
    ]
    write_json(ctx["result_path"], document)


def do_no_witness_incomplete(ctx):
    document = base(ctx["req"], "completed")
    document["properties"] = [
        prop(ctx["req"], "no_witness_found", "bounded_paths", False, "frontier remains", "not_attempted")
    ]
    write_json(ctx["result_path"], document)


def do_timeout(ctx):
    write_json(ctx["result_path"], correct_result(ctx["req"]))
    spawn_sleeper(ctx["pid_file"])
    time.sleep(120)


def do_unsupported(ctx):
    write_json(ctx["result_path"], base(ctx["req"], "unsupported"))


def do_missing_tool(ctx):
    write_json(ctx["result_path"], base(ctx["req"], "tool_unavailable"))


def do_missing_finding(ctx):
    write_json(ctx["result_path"], base(ctx["req"], "completed"))


def do_wrong_request_id(ctx):
    document = correct_result(ctx["req"])
    document["request_id"] = ctx["req"]["request_id"] + "-stale"
    write_json(ctx["result_path"], document)


def do_stale_subject_hash(ctx):
    document = correct_result(ctx["req"])
    document["subject"]["identity"]["spec_hash"] = "stale-spec-hash"
    write_json(ctx["result_path"], document)


def do_malformed_json(ctx):
    with open(ctx["result_path"], "w") as handle:
        handle.write("{")


def do_truncated_artifact(ctx):
    document = correct_result(ctx["req"])
    document["artifacts"] = ["evidence.json"]
    with open(os.path.join(ctx["out"], "evidence.json"), "w") as handle:
        handle.write("{")
    write_json(ctx["result_path"], document)


def do_illegal_traversal(ctx):
    parent = os.path.dirname(ctx["out"])
    with open(os.path.join(parent, "escaped.txt"), "w") as handle:
        handle.write("escaped\n")
    document = correct_result(ctx["req"])
    document["artifacts"] = ["../escaped.txt"]
    write_json(ctx["result_path"], document)


def do_illegal_symlink(ctx):
    os.symlink("/etc/passwd", os.path.join(ctx["out"], "linked"))
    document = correct_result(ctx["req"])
    document["artifacts"] = ["linked"]
    write_json(ctx["result_path"], document)


def do_exit_0_no_result(_ctx):
    sys.stdout.write("no-result\n")
    sys.stdout.flush()


def do_kill_mid_output(ctx):
    with open(ctx["result_path"], "w") as handle:
        handle.write('{"protocol":')
        handle.flush()
    sys.stdout.write("partial-stdout\n")
    sys.stdout.flush()
    spawn_sleeper(ctx["pid_file"])
    time.sleep(120)


def do_witness_no_replay(ctx):
    write_json(os.path.join(ctx["out"], "witness.json"), {"replayable": False, "steps": []})
    document = witnessed(ctx, False, "claimed concrete replay")
    write_json(ctx["result_path"], document)


def do_witness_replay_ok(ctx):
    write_json(os.path.join(ctx["out"], "witness.json"), {"replayable": True, "steps": ["step-1"]})
    document = witnessed(ctx, True, "replayed finite prefix")
    write_json(ctx["result_path"], document)


def witnessed(ctx, replayable, reason):
    document = base(ctx["req"], "completed")
    document["properties"] = [
        prop(
            ctx["req"],
            "witness_found",
            "observed_trace",
            True,
            reason,
            "concrete_reproduced",
            "witness.json",
        )
    ]
    return document


def do_holds(ctx):
    document = base(ctx["req"], "completed")
    document["properties"] = [
        prop(ctx["req"], "holds_in_scope", "exhaustive_finite_model", True, "closed finite graph", "model_only")
    ]
    write_json(ctx["result_path"], document)


def do_holds_from_sample(ctx):
    document = base(ctx["req"], "completed")
    document["properties"] = [
        prop(ctx["req"], "holds_in_scope", "sampled_execution", True, "sample claimed as proof", "not_attempted")
    ]
    write_json(ctx["result_path"], document)


def input_target(ctx):
    rel = ctx["controls"].get("input_path") or ctx["req"]["inputs"][0]["path"]
    return os.path.join(ctx["req"]["bundle_dir"], rel)


def do_modify_input(ctx):
    with open(input_target(ctx), "ab") as handle:
        handle.write(b"\nmodified\n")
    write_json(ctx["result_path"], correct_result(ctx["req"]))


def do_backend_accept(ctx):
    document = correct_result(ctx["req"])
    document["verdict"] = "ACCEPT"
    write_json(ctx["result_path"], document)


def do_crash(_ctx):
    sys.exit(1)


def do_flood_stdout(ctx):
    spawn_sleeper(ctx["pid_file"])
    sys.stdout.write("x" * 100000)
    sys.stdout.flush()
    time.sleep(120)


def unknown_behavior():
    sys.stderr.write("unknown behavior\n")
    sys.exit(2)


BEHAVIORS = {
    "correct": do_correct,
    "violation": do_violation,
    "reachability_witness": do_reachability_witness,
    "no_witness_incomplete": do_no_witness_incomplete,
    "timeout": do_timeout,
    "unsupported": do_unsupported,
    "missing_tool": do_missing_tool,
    "missing_finding": do_missing_finding,
    "wrong_request_id": do_wrong_request_id,
    "stale_subject_hash": do_stale_subject_hash,
    "malformed_json": do_malformed_json,
    "truncated_artifact": do_truncated_artifact,
    "illegal_traversal": do_illegal_traversal,
    "illegal_symlink": do_illegal_symlink,
    "exit_0_no_result": do_exit_0_no_result,
    "kill_mid_output": do_kill_mid_output,
    "witness_no_replay": do_witness_no_replay,
    "witness_replay_ok": do_witness_replay_ok,
    "holds": do_holds,
    "holds_from_sample": do_holds_from_sample,
    "modify_input": do_modify_input,
    "backend_accept": do_backend_accept,
    "crash": do_crash,
    "flood_stdout": do_flood_stdout,
}


def dispatch(ctx):
    handler = BEHAVIORS.get(ctx["behavior"])
    if handler is None:
        unknown_behavior()
    handler(ctx)


def main():
    request_path, out = parse_args(sys.argv)
    dispatch(load_context(request_path, out))


if __name__ == "__main__":
    main()
