#!/usr/bin/python3
"""Starter analysis adapter over an exported ik.graph.v1 graph.

Property family: dead_end_count. A dead end is a non-terminal node that is not
on the export frontier and has no committed outgoing edge. Refusal and no-op
self-loops are not continuations.

Evidence is exhaustive_finite_model only when the export is closed. Otherwise
it is bounded_paths. Any other question is unsupported. This process does not
download a tool and does not read a production store.

controls.conformance_fault is a starter SDK hook used only by
`ik analysis conformance` fixtures. Keep it when you replace the property
function. timeout and cancel wait until the host kills the process so a
killed run still had a result file on disk. Other rejection fixtures are
injected by the host after this process exits; this script does not emit
them on a normal run.
"""

import argparse
import json
import os
import sys

REQUEST_ID_MODE = "echo"
CONTAINED_WRITES = "contained"
WRITE_RESULT = "write"
INCOMPLETE_FINDING = "honest"
KEEP_NATIVE_LOG = "keep"
HONOR_TIMEOUT = "sleep"
HONOR_CANCEL = "sleep"
QUESTION = "dead_end_count"


def parse_args(argv):
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("command")
    parser.add_argument("--request")
    parser.add_argument("--out")
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
    write_text(path, json.dumps(value) + "\n")


def control(req, key):
    controls = req.get("controls") or {}
    return controls.get(key) or ""


def request_id_of(req):
    if REQUEST_ID_MODE == "echo":
        return req["request_id"]
    return "wrong-request-id"


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
        "request_id": request_id_of(req),
        "run_id": req["run_id"],
        "subject": json.loads(json.dumps(req["subject"])),
        "goals": goal_rows(req),
        "profile_digest": req["profile_digest"],
        "inputs": input_rows(req),
        "execution_status": execution,
        "properties": [],
        "artifacts": [],
    }


def property_of(req, finding, evidence, complete, reason):
    return {
        "goal_id": required_id(req),
        "property_class": req["property_class"],
        "finding": finding,
        "evidence_class": evidence,
        "coverage": {"complete": complete, "reason": reason},
        "reproduction": {"status": "model_only"},
    }


def emit_result(out, document):
    if WRITE_RESULT != "write":
        return 0
    write_json(os.path.join(out, "result.json"), document)
    return 0


def write_status(req, out, execution):
    return emit_result(out, base(req, execution))


def describe_body():
    return {
        "engine": {"id": "starter-graph", "version": "1"},
        "inputs": ["finite_runtime_graph"],
        "properties": ["finite_scope_stuck"],
        "questions": [QUESTION],
        "evidence": ["bounded_paths", "exhaustive_finite_model"],
        "downloads": False,
    }


def do_describe(_parsed):
    json.dump(describe_body(), sys.stdout)
    sys.stdout.write("\n")
    return 0


def supported(req):
    return req.get("property_class") == "finite_scope_stuck" and control(req, "question") == QUESTION


def graph_path(req):
    inputs = req.get("inputs") or []
    if not inputs:
        return None
    bundle = req.get("bundle_dir") or ""
    relative = inputs[0].get("path") or ""
    if not bundle or not relative:
        return None
    return os.path.join(bundle, relative)


def load_graph(req):
    path = graph_path(req)
    if path is None:
        return None
    try:
        graph = read_json(path)
    except (OSError, ValueError):
        return None
    if graph.get("schema") != "ik.graph.v1":
        return None
    return graph


def note_committed(sources, edge):
    if edge.get("outcome") == "committed":
        sources.add(edge.get("from"))


def committed_sources(graph):
    sources = set()
    for edge in graph.get("edges") or []:
        note_committed(sources, edge)
    return sources


def is_stuck(node, sources, frontier):
    if node.get("terminal"):
        return False
    if node.get("id") in frontier:
        return False
    return node.get("id") not in sources


def dead_ends(graph):
    frontier = set(graph.get("frontier") or [])
    sources = committed_sources(graph)
    return [node for node in graph.get("nodes") or [] if is_stuck(node, sources, frontier)]


def evidence_of(graph):
    if graph.get("complete") is True and not graph.get("frontier"):
        return "exhaustive_finite_model", True, "exported graph is closed"
    reason = graph.get("incomplete_reason") or "graph incomplete"
    return "bounded_paths", False, reason


def incomplete_answer(nodes, evidence, reason):
    if INCOMPLETE_FINDING != "honest":
        return "holds_in_scope", "exhaustive_finite_model", True, "overclaim"
    if nodes:
        return "violation", evidence, False, reason
    return "inconclusive", evidence, False, reason


def classify(graph, nodes):
    evidence, complete, reason = evidence_of(graph)
    if not complete:
        return incomplete_answer(nodes, evidence, reason)
    if nodes:
        return "violation", evidence, True, reason
    return "holds_in_scope", evidence, True, "no non-terminal stuck node in the closed graph"


def write_native(out, text):
    if KEEP_NATIVE_LOG != "keep":
        return None
    write_text(os.path.join(out, "native", "checker.log"), text)
    return "native/checker.log"


def announce(count):
    if KEEP_NATIVE_LOG != "keep":
        return
    sys.stdout.write("dead_end_count=%s\n" % count)
    sys.stdout.flush()


def native_text(nodes, evidence, complete):
    ids = [node.get("id") for node in nodes]
    return "dead_end_count=%s evidence=%s complete=%s ids=%s\n" % (len(nodes), evidence, complete, ids)


def write_escape(out):
    parent = os.path.dirname(os.path.abspath(out))
    write_text(os.path.join(parent, "escaped.txt"), "escaped\n")


def kept_log(log_name):
    if log_name:
        return [log_name]
    return []


def escape_rows(out):
    if CONTAINED_WRITES != "escape":
        return []
    write_escape(out)
    return ["../escaped.txt"]


def artifact_list(out, log_name):
    return kept_log(log_name) + escape_rows(out)


def write_count(req, out, graph):
    nodes = dead_ends(graph)
    finding, evidence, complete, reason = classify(graph, nodes)
    log_name = write_native(out, native_text(nodes, evidence, complete))
    announce(len(nodes))
    document = base(req, "completed")
    document["properties"] = [property_of(req, finding, evidence, complete, reason)]
    document["artifacts"] = artifact_list(out, log_name)
    return emit_result(out, document)


def run_check(req, out):
    if not supported(req):
        return write_status(req, out, "unsupported")
    graph = load_graph(req)
    if graph is None:
        return write_status(req, out, "failed")
    return write_count(req, out, graph)


def wait_until_killed():
    """Block until SIGKILL. A finite sleep can finish before a loaded timeout."""
    read_fd, write_fd = os.pipe()
    try:
        os.read(read_fd, 1)
    finally:
        os.close(read_fd)
        os.close(write_fd)


def maybe_sleep(mode):
    if mode == "sleep":
        wait_until_killed()


def finish_run(req, out):
    fault = control(req, "conformance_fault")
    run_check(req, out)
    if fault == "timeout":
        maybe_sleep(HONOR_TIMEOUT)
    elif fault == "cancel":
        maybe_sleep(HONOR_CANCEL)
    return 0


def do_run(parsed):
    if not parsed.request or not parsed.out:
        return 2
    os.makedirs(parsed.out, exist_ok=True)
    return finish_run(read_json(parsed.request), parsed.out)


COMMANDS = {"describe": do_describe, "run": do_run}


def main(argv):
    parsed = parse_args(argv)
    handler = COMMANDS.get(parsed.command)
    if handler is None:
        return 2
    return handler(parsed)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
