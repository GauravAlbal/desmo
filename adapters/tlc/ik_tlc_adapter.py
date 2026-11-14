#!/usr/bin/python3
"""TLC adapter over an exported ik.graph.v1 finite graph.

This process never downloads a tool. Java and the TLC jar come from the
toolchain file next to this script, or from --toolchain.

Predicate language (controls.predicate), a conjunction of:
  true
  label == "NAME"
  label != "NAME"
  fact.NAME == JSON
  fact.NAME != JSON
  node != ID
The predicate must hold in every reachable node. It is translated to the set
of node ids that violate it. Reachability uses the invariant-violation trick:
the target set is asserted unreachable, and a violation is the witness.

Results are model-level evidence for the exported graph, its restrictions, and
the probed TLC version. A complete graph plus an exhaustive TLC run can support
holds_in_scope. An incomplete graph never does.
"""

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile

CLASS_QUESTION = {
    "safety": "safety_invariant",
    "reachability": "reach_terminal",
    "finite_scope_stuck": "stuck",
    "temporal": "temporal",
}
QUESTION_CLASS = {
    "safety_invariant": "safety",
    "reach_terminal": "reachability",
    "reach_state": "reachability",
    "stuck": "finite_scope_stuck",
    "temporal": "temporal",
    "liveness": "temporal",
}
PREFIXES = (
    ("label == ", "label_eq"),
    ("label != ", "label_ne"),
    ("node != ", "node_ne"),
    ("fact.", "fact"),
)
EXPECTED_VERSION = "TLC2 Version 2.19 of 08 August 2024 (rev: 5a47802)"
PINNED_JAR_SHA = "936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88"


def parse_args(argv):
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--toolchain")
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


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        chunk = handle.read(1024 * 1024)
        while chunk:
            digest.update(chunk)
            chunk = handle.read(1024 * 1024)
    return digest.hexdigest()


def load_toolchain(path, script):
    chosen = path or os.path.join(os.path.dirname(os.path.abspath(script)), "toolchain.json")
    if not os.path.isfile(chosen):
        return None
    data = read_json(chosen)
    return {key: os.path.expanduser(val) if key in ("java", "jar") else val for key, val in data.items()}


def probe_output(java, jar, tmp):
    try:
        done = subprocess.run(
            [java, "-Djava.io.tmpdir=" + tmp, "-cp", jar, "tlc2.TLC"],
            capture_output=True, text=True, timeout=30, env=tlc_env(tmp),
        )
    except (OSError, subprocess.TimeoutExpired):
        return ""
    return done.stdout + "\n" + done.stderr


def version_text(java, jar):
    tmp = tempfile.mkdtemp(prefix="ik-tlc-probe-")
    try:
        return probe_output(java, jar, tmp)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def probe(toolchain):
    if not toolchain_present(toolchain):
        return None
    text = version_text(toolchain["java"], toolchain["jar"])
    if toolchain.get("version", EXPECTED_VERSION) not in text:
        return None
    return toolchain.get("version", EXPECTED_VERSION)


def toolchain_present(toolchain):
    if not toolchain:
        return False
    java = toolchain.get("java") or ""
    jar = toolchain.get("jar") or ""
    if not os.path.isfile(java) or not os.access(java, os.X_OK):
        return False
    if not os.path.isfile(jar):
        return False
    if sha256_file(jar) != toolchain.get("jar_sha256"):
        return False
    if toolchain.get("jar_sha256") != PINNED_JAR_SHA:
        return False
    return True


def cmd_describe(toolchain):
    probed = probe(toolchain)
    body = {
        "engine": {"id": "TLC", "version": probed},
        "java": None if not toolchain else toolchain.get("java"),
        "jar_sha256": None if not toolchain else toolchain.get("jar_sha256"),
        "inputs": ["finite_runtime_graph"],
        "properties": ["safety", "reachability", "finite_scope_stuck", "temporal"],
        "evidence": ["exhaustive_finite_model", "bounded_paths"],
        "downloads": False,
    }
    json.dump(body, sys.stdout)
    sys.stdout.write("\n")
    if probed is None:
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


def property_of(req, finding, evidence, complete, reason, reproduction):
    return {
        "goal_id": required_id(req),
        "property_class": req["property_class"],
        "finding": finding,
        "evidence_class": evidence,
        "coverage": {"complete": complete, "reason": reason},
        "reproduction": reproduction,
    }


def question_of(req):
    controls = req.get("controls") or {}
    asked = controls.get("question") or CLASS_QUESTION.get(req.get("property_class"))
    if asked in ("liveness", "temporal"):
        return "temporal"
    if QUESTION_CLASS.get(asked) != req.get("property_class"):
        return None
    return asked


def question_ready(question, controls):
    if question == "safety_invariant":
        return bool((controls.get("predicate") or "").strip())
    if question == "reach_state":
        return bool(controls.get("target_label"))
    return question in ("reach_terminal", "stuck")


def graph_rel(req):
    controls = req.get("controls") or {}
    if controls.get("graph"):
        return controls["graph"]
    return first_input(req.get("inputs") or [])


def first_input(inputs):
    for item in inputs:
        path = item.get("path") or ""
        if path.endswith("graph.json"):
            return path
    if not inputs:
        return None
    return inputs[0].get("path")


def load_graph(req):
    rel = graph_rel(req)
    if not rel or rel.startswith("/") or ".." in rel.split("/"):
        return None
    path = os.path.join(req["bundle_dir"], rel)
    if not os.path.isfile(path):
        return None
    graph = read_json(path)
    if graph.get("schema") != "ik.graph.v1":
        return None
    return graph


def subject_ok(req, graph):
    ident = req["subject"]["identity"]
    if ident.get("machine_id") != graph.get("machine_id"):
        return False
    return ident.get("spec_hash") == graph.get("spec_hash")


def cmd_run(request_path, out, toolchain):
    os.makedirs(out, exist_ok=True)
    req = read_json(request_path)
    if probe(toolchain) is None:
        write_status(req, out, "tool_unavailable")
        return
    question = question_of(req)
    if question in (None, "temporal"):
        write_status(req, out, "unsupported")
        return
    graph = load_graph(req)
    if graph is None or not subject_ok(req, graph):
        write_status(req, out, "failed")
        return
    if not question_ready(question, req.get("controls") or {}):
        write_status(req, out, "failed")
        return
    finish_model(req, out, toolchain, question, graph)


def finish_model(req, out, toolchain, question, graph):
    native = os.path.join(out, "native")
    os.makedirs(native, exist_ok=True)
    forbidden, error = safe_forbidden(graph, question, req.get("controls") or {})
    if error:
        write_status(req, out, "failed")
        return
    edges = edges_for_model(graph, (req.get("controls") or {}).get("mutate"))
    write_model(native, graph, edges, forbidden)
    code, stdout, stderr = run_tlc(toolchain["java"], toolchain["jar"], native)
    write_text(os.path.join(native, "stdout.log"), stdout)
    write_text(os.path.join(native, "stderr.log"), stderr)
    kind = classify_tlc(code, stdout)
    report = conformance(graph, stdout, kind == "ok")
    write_json(os.path.join(native, "conformance.json"), report)
    write_provenance(native, toolchain, graph)
    if kind == "error" or fidelity_failed(report):
        write_fidelity_failure(req, out, report)
        return
    write_checked(req, out, question, graph, kind, stdout)


def safe_forbidden(graph, question, controls):
    try:
        return forbidden_ids(graph, question, controls), None
    except ValueError as err:
        return None, str(err)


def fidelity_failed(report):
    return bool(report.get("checked")) and not report.get("match")


def write_fidelity_failure(req, out, report):
    reason = report.get("reason") or "tlc discrepancy"
    prop = property_of(req, "inconclusive", "bounded_paths", False, reason, {"status": "not_attempted"})
    document = base(req, "failed")
    document["properties"] = [prop]
    document["artifacts"] = listed(out, None)
    write_json(os.path.join(out, "result.json"), document)


def write_checked(req, out, question, graph, kind, stdout):
    finding, evidence, complete = mapped_finding(question, graph, kind)
    reproduction, scenario = reproduction_for(kind, graph, stdout, req, out)
    prop = property_of(req, finding, evidence, complete, coverage_reason(graph, finding), reproduction)
    document = base(req, "completed")
    document["properties"] = [prop]
    document["artifacts"] = listed(out, scenario)
    write_json(os.path.join(out, "result.json"), document)


def mapped_finding(question, graph, kind):
    if question in ("reach_terminal", "reach_state"):
        return map_reach(kind, graph)
    return map_safety(kind, graph)


def map_safety(kind, graph):
    if not graph.get("complete"):
        return incomplete_safety(kind)
    if kind == "violation":
        return "violation", "exhaustive_finite_model", True
    if kind == "ok":
        return "holds_in_scope", "exhaustive_finite_model", True
    return "inconclusive", "bounded_paths", False


def incomplete_safety(kind):
    if kind == "violation":
        return "violation", "bounded_paths", False
    return "inconclusive", "bounded_paths", False


def map_reach(kind, graph):
    if kind == "violation":
        return reach_witness(graph)
    if not graph.get("complete"):
        return "inconclusive", "bounded_paths", False
    if kind == "ok":
        return "no_witness_found", "exhaustive_finite_model", True
    return "inconclusive", "bounded_paths", False


def reach_witness(graph):
    if graph.get("complete"):
        return "witness_found", "exhaustive_finite_model", True
    return "witness_found", "bounded_paths", False


def coverage_reason(graph, finding):
    if graph.get("complete") and finding != "inconclusive":
        return "exhaustive TLC over the exported finite graph; model-level evidence only"
    reason = graph.get("incomplete_reason") or "graph incomplete"
    return "bounded paths; graph incomplete (%s); not holds_in_scope" % reason


def listed(out, scenario):
    names = [
        "native/IkGraph.tla",
        "native/IkGraph.cfg",
        "native/stdout.log",
        "native/stderr.log",
        "native/conformance.json",
        "native/provenance.json",
    ]
    if scenario:
        names.append(scenario)
    return [name for name in names if nonempty(os.path.join(out, name))]


def nonempty(path):
    return os.path.isfile(path) and os.path.getsize(path) > 0


def write_provenance(native, toolchain, graph):
    body = {
        "evidence_scope": "model",
        "engine": "TLC",
        "engine_version": toolchain.get("version"),
        "java": toolchain.get("java"),
        "jar_sha256": toolchain.get("jar_sha256"),
        "graph_complete": bool(graph.get("complete")),
        "incomplete_reason": graph.get("incomplete_reason"),
        "restrictions": graph.get("restrictions") or [],
        "frontier": graph.get("frontier") or [],
        "note": "MODEL-level evidence for the exported finite graph, its restrictions, and this TLC version",
    }
    write_json(os.path.join(native, "provenance.json"), body)


def edges_for_model(graph, mutate):
    edges = [edge for edge in graph["edges"] if edge.get("outcome") == "committed"]
    if mutate == "drop_committed_edge" and edges:
        return edges[1:]
    return edges


def write_model(native, graph, edges, forbidden):
    write_text(os.path.join(native, "IkGraph.tla"), module_text(graph, edges, forbidden))
    write_text(os.path.join(native, "IkGraph.cfg"), "SPECIFICATION Spec\nINVARIANT Inv\n")


def module_text(graph, edges, forbidden):
    terminals = [node["id"] for node in graph["nodes"] if node.get("terminal")]
    lines = [
        "---- MODULE IkGraph ----",
        "EXTENDS TLC",
        "VARIABLE node",
        "Init == node = %s" % tla_string(graph["initial"]),
        "TERMINAL == %s" % tla_set(terminals),
        "Next ==",
        next_clauses(edges),
        "Spec == Init /\\ [][Next]_node",
        "Inv == %s" % inv_expr(forbidden),
        "====",
        "",
    ]
    return "\n".join(lines)


def next_clauses(edges):
    lines = [committed_clause(edge) for edge in edges]
    lines.append("  \\/ (node \\in TERMINAL /\\ UNCHANGED node)")
    lines.append('  \\/ (node = "n__none__" /\\ node\' = "n__none__")')
    return "\n".join(lines)


def committed_clause(edge):
    return "  \\/ (node = %s /\\ node' = %s)" % (tla_string(edge["from"]), tla_string(edge["to"]))


def inv_expr(forbidden):
    if not forbidden:
        return "TRUE"
    return "node \\notin %s" % tla_set(forbidden)


def tla_set(ids):
    if not ids:
        return "{}"
    return "{%s}" % ", ".join(tla_string(item) for item in ids)


def tla_string(value):
    escaped = str(value).replace("\\", "\\\\").replace('"', '\\"')
    return '"%s"' % escaped


def ensure_dir(path):
    os.makedirs(path, exist_ok=True)
    return path


def tlc_command(java, jar, work):
    meta = ensure_dir(os.path.join(work, "states"))
    tmp = ensure_dir(os.path.join(work, "jvm-tmp"))
    home = ensure_dir(os.path.join(work, "jvm-home"))
    cmd = [
        java,
        "-Djava.io.tmpdir=" + tmp,
        "-Duser.home=" + home,
        "-XX:+UseParallelGC",
        "-cp", jar,
        "tlc2.TLC",
        "-nowarning",
        "-workers", "1",
        "-deadlock",
        "-metadir", meta,
        "-config", "IkGraph.cfg",
        "IkGraph.tla",
    ]
    return cmd, tmp


def tlc_env(tmp):
    return {"TMPDIR": tmp, "TEMP": tmp, "TMP": tmp}


def run_tlc(java, jar, work):
    cmd, tmp = tlc_command(java, jar, work)
    try:
        done = subprocess.run(
            cmd, cwd=work, capture_output=True, text=True, timeout=120, env=tlc_env(tmp)
        )
    except (OSError, subprocess.TimeoutExpired) as err:
        return 1, "", str(err)
    return done.returncode, done.stdout, done.stderr


def classify_tlc(code, stdout):
    if "Invariant Inv is violated." in stdout:
        return "violation"
    if code == 0 and "No error has been found." in stdout:
        return "ok"
    return "error"


def conformance(graph, text, completed_ok):
    native = len(native_reachable(graph))
    distinct = distinct_count(text)
    if not completed_ok:
        return unchecked(native, distinct)
    return checked_count(native, distinct)


def unchecked(native, distinct):
    return {
        "match": True,
        "checked": False,
        "native_reachable": native,
        "tlc_distinct": distinct,
        "reason": "counterexample search stopped; distinct count is not a closure check",
    }


def checked_count(native, distinct):
    match = distinct == native
    reason = "committed-edge reachable set matches TLC distinct states"
    if not match:
        reason = "generated Next relation does not match committed edges"
    return {
        "match": match,
        "checked": True,
        "native_reachable": native,
        "tlc_distinct": distinct,
        "reason": reason,
    }


def distinct_count(text):
    import re
    found = re.findall(r"(\d+) distinct states? found", text)
    if not found:
        return None
    return int(found[-1])


def note_committed(outgoing, edge):
    if edge.get("outcome") != "committed":
        return
    outgoing.setdefault(edge["from"], []).append(edge["to"])


def committed_adj(graph):
    outgoing = {}
    for edge in graph["edges"]:
        note_committed(outgoing, edge)
    return outgoing


def enqueue(node, known, queue):
    if node not in known:
        queue.append(node)


def visit(node, known, seen, queue, outgoing):
    if node in known:
        return
    known.add(node)
    seen.append(node)
    for nxt in outgoing.get(node, []):
        enqueue(nxt, known, queue)


def drain(queue, known, seen, outgoing):
    while queue:
        visit(queue.pop(0), known, seen, queue, outgoing)


def native_reachable(graph):
    seen = []
    known = set()
    drain([graph["initial"]], known, seen, committed_adj(graph))
    return seen


def forbidden_ids(graph, question, controls):
    if question == "safety_invariant":
        return violating_nodes(graph, controls.get("predicate") or "")
    if question in ("reach_terminal", "reach_state"):
        return [node["id"] for node in target_nodes(graph, question, controls)]
    if question == "stuck":
        return [node["id"] for node in stuck_nodes(graph)]
    return []


def violating_nodes(graph, predicate):
    return [node["id"] for node in graph["nodes"] if not pred_holds(node, predicate)]


def pred_holds(node, predicate):
    text = predicate.strip()
    if text == "true":
        return True
    return all(clause_holds(node, part.strip()) for part in text.split("&&"))


def clause_kind(clause):
    for prefix, kind in PREFIXES:
        if clause.startswith(prefix):
            return kind
    return ""


def clause_holds(node, clause):
    checker = {
        "label_eq": check_label_eq,
        "label_ne": check_label_ne,
        "node_ne": check_node_ne,
        "fact": check_fact,
    }.get(clause_kind(clause))
    if checker is None:
        raise ValueError(clause)
    return checker(node, clause)


def unquote(text):
    value = text.strip()
    if len(value) >= 2 and value[0] == '"' and value[-1] == '"':
        return value[1:-1]
    return value


def check_label_eq(node, clause):
    return node.get("label") == unquote(clause[len("label == "):])


def check_label_ne(node, clause):
    return node.get("label") != unquote(clause[len("label != "):])


def check_node_ne(node, clause):
    return node.get("id") != unquote(clause[len("node != "):])


def check_fact(node, clause):
    if " != " in clause:
        return fact_cmp(node, clause, False)
    if " == " in clause:
        return fact_cmp(node, clause, True)
    raise ValueError(clause)


def fact_cmp(node, clause, expect_eq):
    sep = " == " if expect_eq else " != "
    left, raw = clause.split(sep, 1)
    name = left[len("fact."):].strip()
    expected = json.loads(raw.strip())
    same = node.get("facts", {}).get(name) == expected
    if expect_eq:
        return same
    return not same


def target_nodes(graph, question, controls):
    label = controls.get("target_label")
    if question == "reach_state":
        return [node for node in graph["nodes"] if node.get("label") == label]
    nodes = [node for node in graph["nodes"] if node.get("terminal")]
    if not label:
        return nodes
    return [node for node in nodes if node.get("label") == label]


def stuck_node(node, outgoing, frontier):
    if node.get("terminal") or node["id"] in frontier:
        return False
    return node["id"] not in outgoing


def stuck_nodes(graph):
    frontier = set(graph.get("frontier") or [])
    outgoing = set(committed_adj(graph))
    return [node for node in graph["nodes"] if stuck_node(node, outgoing, frontier)]


def reproduction_for(kind, graph, stdout, req, out):
    if kind != "violation":
        return {"status": "not_attempted"}, None
    return mapped_reproduction(graph, stdout, req, out)


def mapped_reproduction(graph, stdout, req, out):
    import re
    node_ids = re.findall(r'node = "([^"]+)"', stdout)
    mapped = path_edges(graph, node_ids)
    if not mapped or not mapped[0]:
        return {"status": "mapping_incomplete"}, None
    steps, labels = mapped
    return finish_replay(req, out, steps, labels)


def note_edge(index, edge):
    if edge.get("outcome") != "committed":
        return
    index.setdefault((edge["from"], edge["to"]), edge)


def edge_index(graph):
    index = {}
    for edge in graph["edges"]:
        note_edge(index, edge)
    return index


def take_pair(src, dst, index, nodes, steps, labels):
    if src == dst:
        return True
    edge = index.get((src, dst))
    if edge is None or dst not in nodes:
        return False
    steps.append(edge)
    labels.append(nodes[dst]["label"])
    return True


def path_edges(graph, node_ids):
    nodes = {node["id"]: node for node in graph["nodes"]}
    index = edge_index(graph)
    steps = []
    labels = []
    for src, dst in zip(node_ids, node_ids[1:]):
        if not take_pair(src, dst, index, nodes, steps, labels):
            return None
    return steps, labels


def command_step(index, edge):
    return {
        "kind": "command",
        "label": "s%d" % (index + 1),
        "name": edge["command"],
        "args": edge.get("args") or {},
        "expect": "committed",
    }


def scenario_doc(req, steps, replayable):
    aggregate = (req.get("controls") or {}).get("aggregate") or "tlc-witness"
    return {
        "scenario_version": 1,
        "machine": "machine.ik.toml",
        "aggregate": aggregate,
        "replayable": replayable,
        "steps": steps,
    }


def safe_goal(req):
    goal = required_id(req)
    if not goal or "/" in goal or ".." in goal:
        return "goal"
    return goal


def finish_replay(req, out, steps, labels):
    goal = safe_goal(req)
    rel = "findings/%s/scenario.json" % goal
    dest = os.path.join(out, "findings", goal)
    os.makedirs(dest, exist_ok=True)
    machine = copy_machine(req, dest)
    scenario_path = os.path.join(dest, "scenario.json")
    matched = replay_matches(req, machine, scenario_path, steps, labels, os.path.join(out, "replay"))
    write_json(scenario_path, scenario_doc(req, [command_step(i, edge) for i, edge in enumerate(steps)], matched))
    if matched:
        return {"status": "concrete_reproduced", "artifact": rel}, rel
    return {"status": "mapping_incomplete"}, rel


def copy_machine(req, dest_dir):
    rel = (req.get("controls") or {}).get("machine")
    if not rel or rel.startswith("/") or ".." in rel.split("/"):
        return None
    src = os.path.join(req["bundle_dir"], rel)
    if not os.path.isfile(src):
        return None
    dest = os.path.join(dest_dir, "machine.ik.toml")
    shutil.copyfile(src, dest)
    return dest


def ik_bin(req):
    path = (req.get("controls") or {}).get("ik_bin") or ""
    if os.path.isfile(path) and os.access(path, os.X_OK):
        return path
    return None


def replay_matches(req, machine, scenario_path, steps, labels, scratch):
    binary = ik_bin(req)
    if not binary or not machine:
        return False
    write_json(scenario_path, scenario_doc(req, [command_step(i, edge) for i, edge in enumerate(steps)], False))
    result = run_scenario(binary, machine, scenario_path, scratch)
    if result is None:
        return False
    return labels_match(result, labels)


def run_scenario(binary, machine, scenario, scratch):
    os.makedirs(scratch, exist_ok=True)
    try:
        done = subprocess.run(
            [binary, "scenario", "run", machine, scenario, "--out", scratch],
            capture_output=True, text=True, timeout=60,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    if done.returncode != 0:
        return None
    path = os.path.join(scratch, "result.json")
    if not os.path.isfile(path):
        return None
    return read_json(path)


def command_labels(result):
    labels = []
    for step in result.get("steps") or []:
        if step.get("kind") == "command":
            labels.append(step.get("state_after", {}).get("state"))
    return labels


def labels_match(result, expected):
    if not result.get("all_expectations_met"):
        return False
    return command_labels(result) == expected


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
