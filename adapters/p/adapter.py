#!/usr/bin/python3
"""Prepared P route. Not a qualified checker.

STATUS: prepared, not qualified; no real P run exists.

This process does not spawn p or dotnet. describe looks up p --version on
PATH and reports tool_unavailable when that command is absent. run writes the
same status and does not start a checker. parse reads a fixture file only.
"""

import argparse
import json
import os
import shutil
import sys

STATUS = "prepared, not qualified; no real P run exists"
PROBE = "p --version"


def parse_args(argv):
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("command")
    parser.add_argument("--request")
    parser.add_argument("--out")
    parser.add_argument("--fixture")
    return parser.parse_args(argv[1:])


def probe_found(name):
    return shutil.which(name) is not None


def describe_body():
    found = probe_found("p")
    status = "tool_unavailable"
    if found:
        status = "present_unqualified"
    return {
        "protocol": "ik.analysis.p-describe.v1",
        "executed": False,
        "execution_status": status,
        "engine": {"id": "P", "version": None},
        "probe": PROBE,
        "probe_found": found,
        "dotnet_found": probe_found("dotnet"),
        "checker_executed": False,
        "note": STATUS,
    }


def do_describe(_parsed):
    body = describe_body()
    json.dump(body, sys.stdout)
    sys.stdout.write("\n")
    if body["execution_status"] == "tool_unavailable":
        return 1
    return 0


def goal_rows(req):
    return [{"id": goal["id"], "content_digest": goal["content_digest"]} for goal in req["goals"]]


def input_rows(req):
    return [{"path": item["path"], "sha256": item["sha256"]} for item in req["inputs"]]


def unavailable_result(req):
    return {
        "protocol": "ik.analysis.result.v1",
        "request_id": req["request_id"],
        "run_id": req["run_id"],
        "subject": json.loads(json.dumps(req["subject"])),
        "goals": goal_rows(req),
        "profile_digest": req["profile_digest"],
        "inputs": input_rows(req),
        "execution_status": "tool_unavailable",
        "properties": [],
        "artifacts": [],
    }


def write_json(path, value):
    parent = os.path.dirname(path)
    if parent:
        os.makedirs(parent, exist_ok=True)
    with open(path, "w") as handle:
        handle.write(json.dumps(value) + "\n")


def do_run(parsed):
    if not parsed.request or not parsed.out:
        return 2
    req = json.load(open(parsed.request))
    os.makedirs(parsed.out, exist_ok=True)
    write_json(os.path.join(parsed.out, "result.json"), unavailable_result(req))
    return 0


def violation_row():
    return {
        "finding": "violation",
        "evidence_class": "bounded_paths",
        "coverage_complete": False,
        "reproduction_status": "model_only",
        "holds_in_scope": False,
    }


def hot_row():
    return {
        "finding": "inconclusive",
        "evidence_class": "observed_trace",
        "coverage_complete": False,
        "reproduction_status": "not_attempted",
        "holds_in_scope": False,
    }


def no_bug_row():
    return {
        "finding": "no_violation_found",
        "evidence_class": "bounded_paths",
        "coverage_complete": False,
        "reproduction_status": "not_attempted",
        "holds_in_scope": False,
    }


def failed_row():
    return {
        "finding": "inconclusive",
        "evidence_class": "bounded_paths",
        "coverage_complete": False,
        "reproduction_status": "not_attempted",
        "execution_status": "failed",
        "holds_in_scope": False,
    }


def parse_text(text):
    if "Bug found:" in text:
        return violation_row()
    if "remained hot" in text:
        return hot_row()
    if "No bug found" in text:
        return no_bug_row()
    return failed_row()


def do_parse(parsed):
    if not parsed.fixture:
        return 2
    with open(parsed.fixture) as handle:
        body = parse_text(handle.read())
    json.dump(body, sys.stdout)
    sys.stdout.write("\n")
    return 0


COMMANDS = {"describe": do_describe, "run": do_run, "parse": do_parse}


def main(argv):
    parsed = parse_args(argv)
    handler = COMMANDS.get(parsed.command)
    if handler is None:
        return 2
    return handler(parsed)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
