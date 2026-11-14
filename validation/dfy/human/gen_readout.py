#!/usr/bin/env python3
"""Regenerate HUMAN-READOUT.md from journey fixtures and the packed ik binary.

Python 3 stdlib only. Pastes the product's rendered handoff text. Does not rewrite it.
"""
from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

HERE = Path(__file__).resolve().parent
DEFAULT_IK = HERE.parents[1] / "batch-018" / "ik-packed"
DEFAULT_FIXTURES = HERE.parent / "journeys" / "fixtures"
OUT_PATH = HERE / "HUMAN-READOUT.md"
AGENT_PATH = HERE / "AGENT-INTERFACE.md"
BODY_KEYS = (
    "aggregate_id",
    "capability",
    "command",
    "operation_id",
    "observation",
    "effect",
    "commands",
)
HEX64 = re.compile(r"\b[0-9a-f]{64}\b")


def die(msg: str, code: int = 2) -> None:
    sys.stderr.write(msg + "\n")
    raise SystemExit(code)


def resolve_ik() -> Path:
    env = os.environ.get("IK_BINARY")
    path = Path(env) if env else DEFAULT_IK
    if path.is_file():
        return path
    die("ik binary not found at %s (set IK_BINARY)" % path)
    return path


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


def require_ok(proc: subprocess.CompletedProcess, label: str, expect: Optional[int] = 0) -> None:
    if expect is not None and proc.returncode != expect:
        die("%s: exit %s != %s\n%s\n%s" % (label, proc.returncode, expect, proc.stdout, proc.stderr))


def parse_json(text: str) -> Any:
    text = text.strip()
    if not text:
        return None
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return None


def envelope(op: str, ident: Dict[str, Any], body: Dict[str, Any], runtime: str) -> str:
    req = {
        "protocol": {"major": 1, "minor": 0},
        "runtime_version": runtime,
        "op": op,
        "machine": ident,
        "request_id": body.get("request_id", "r"),
    }
    for key in BODY_KEYS:
        if key in body:
            req[key] = body[key]
    return json.dumps(req, separators=(",", ":"))


def store_op(
    ik: Path,
    cwd: Path,
    store: Path,
    machine: str,
    ident: Dict[str, Any],
    op: str,
    body: Dict[str, Any],
    runtime: str,
) -> subprocess.CompletedProcess:
    stdin = envelope(op, ident, body, runtime)
    return run_ik(ik, ["--store", str(store), "--machine", machine, op], cwd, stdin=stdin)


def identity(ik: Path, cwd: Path, machine: str) -> Dict[str, Any]:
    proc = run_ik(ik, ["identity", machine], cwd)
    require_ok(proc, "identity %s" % machine)
    ident = parse_json(proc.stdout)
    if not isinstance(ident, dict):
        die("identity %s: not JSON" % machine)
    return ident


def write_json(path: Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def first_finding_suffix(report: Dict[str, Any], suffix: str) -> Optional[str]:
    for finding in report.get("findings") or []:
        fid = finding.get("id") or ""
        if isinstance(fid, str) and fid.endswith(suffix):
            return fid
    return None


def first_finding_contains(report: Dict[str, Any], needle: str) -> Optional[str]:
    for finding in report.get("findings") or []:
        fid = finding.get("id") or ""
        if isinstance(fid, str) and needle in fid:
            return fid
    return None


def fence(text: str) -> str:
    body = text.replace("\r\n", "\n")
    if body and not body.endswith("\n"):
        body += "\n"
    return "~~~~text\n%s~~~~\n" % body


def redact_paths(text: str, mapping: List[Tuple[str, str]]) -> str:
    out = text
    for old, new in sorted(mapping, key=lambda pair: len(pair[0]), reverse=True):
        out = out.replace(old, new)
    return out


def keep_hex(ident: Dict[str, Any]) -> set:
    keep = set()
    spec = ident.get("spec_hash")
    if isinstance(spec, str):
        keep.add(spec)
    return keep


def stable_product_text(text: str, keep: set) -> str:
    def repl(match: re.Match) -> str:
        token = match.group(0)
        if token in keep:
            return token
        return "<hex>"

    return HEX64.sub(repl, text)


def cmd_line(words: List[str]) -> str:
    return "`ik %s`" % " ".join(words)


def section(title: str, command: str, text: str) -> str:
    return "### %s\n\n%s\n\n%s\n" % (title, command, fence(text))


def mapping_for(tmp: Path, ik: Path, work: Path) -> List[Tuple[str, str]]:
    pairs: List[Tuple[str, str]] = []
    for path, token in (
        (tmp, "$RUN"),
        (work, "$WORK"),
        (HERE.parent / "journeys", "$JOURNEYS"),
        (ik, "$IK"),
        (Path.home(), "$HOME"),
    ):
        text = str(path)
        pairs.append((text, token))
        real = os.path.realpath(text)
        if real != text:
            pairs.append((real, token))
    return pairs


def guard_hash(ik: Path, work: Path, args: List[str], rule: str) -> str:
    """The guard hash the handoff prints for `rule`, read from its JSON."""
    proc = run_ik(ik, ["handoff"] + args + ["--json"], work)
    require_ok(proc, "guard hash handoff")
    for item in json_dict(proc, "guard hash handoff")["origins"]["rules"]:  # type: ignore[index]
        if item["rule"] == rule:
            return str(item["guard_hash"])
    die("no origins entry for rule " + rule)
    return ""


def capture_build(ik: Path, work: Path) -> Dict[str, str]:
    ident_txt = run_ik(ik, ["identity", "fixtures/tagged_item.ik.toml"], work)
    require_ok(ident_txt, "build identity")
    laws = run_ik(ik, ["laws", "fixtures/tagged_item.ik.toml"], work)
    require_ok(laws, "build laws")
    check_argv = [
        "check",
        "fixtures/tagged_item.ik.toml",
        "--corpus",
        "fixtures/tagged_item.ik.cases.json",
        "--profile",
        "fixtures/tagged_item.ik.checks.toml",
    ]
    check = run_ik(ik, check_argv, work)
    require_ok(check, "build check")
    if "ACCEPT" not in check.stdout:
        die("build check: expected ACCEPT in rendered text")
    check_json = run_ik(ik, check_argv + ["--json"], work)
    require_ok(check_json, "build check json")
    check_path = work / "out" / "check-build.json"
    parsed = parse_json(check_json.stdout)
    if not isinstance(parsed, dict):
        die("build check json: not JSON")
    write_json(check_path, parsed)
    out_dir = work / "out" / "sc-build"
    out_dir.mkdir(parents=True, exist_ok=True)
    scenario = run_ik(
        ik,
        [
            "scenario",
            "run",
            "fixtures/tagged_item.ik.toml",
            "fixtures/scenarios/honest-tag.json",
            "--out",
            "out/sc-build",
        ],
        work,
    )
    require_ok(scenario, "build scenario")
    report = run_ik(ik, ["report", "out/sc-build", "out/check-build.json"], work)
    require_ok(report, "build report")
    report_json = run_ik(ik, ["report", "out/sc-build", "out/check-build.json", "--json"], work)
    require_ok(report_json, "build report json")
    report_doc = parse_json(report_json.stdout)
    if not isinstance(report_doc, dict):
        die("build report json: not JSON")
    tag_id = first_finding_suffix(report_doc, "/tag")
    goal_id = first_finding_contains(report_doc, "#tagged")
    if not tag_id or not goal_id:
        die("build report: missing tag or tagged finding")
    behavior = {
        "behavior_version": 1,
        "title": "tagged item delivery",
        "outcomes": [
            {
                "id": "B1",
                "text": "a valid tag on a new item commits",
                "claim": "new_mutation",
                "evidence": [tag_id],
            },
            {
                "id": "B2",
                "text": "the required terminal tagged was witnessed in the supplied cases",
                "evidence": [goal_id],
            },
        ],
        "unchanged": [],
        "assumptions": [],
        "rule_witnesses": [
            {
                "rule": "tag",
                "examples": [
                    {"args": {"id": "a"}, "outcome": "accepted"},
                    {"args": {"id": "x" * 64}, "outcome": "accepted"},
                    {"args": {"id": "a.b-9_Z"}, "outcome": "accepted"},
                    {"args": {"id": ""}, "outcome": "rejected"},
                    {"args": {"id": "x" * 65}, "outcome": "rejected"},
                    {"args": {"id": "item/1"}, "outcome": "rejected"},
                ],
            }
        ],
    }
    behavior_path = work / "behavior-build.json"
    write_json(behavior_path, behavior)
    build_args = [
        "behavior-build.json", "--evidence", "out/sc-build", "--evidence", "out/check-build.json", "--kind", "delivery",
    ]
    tag_hash = guard_hash(ik, work, build_args, "tag")
    behavior["requirement"] = {
        "text": "A valid tag on a new item commits.",
        "source": "exercise brief for the tagged item (a fixture requirement, not a real PRD)",
        "provenance": "agent-restated",
        "atoms": [{"id": "R1", "span": "A valid tag on a new item commits."}],
        "traces": [{"atom": "R1", "element": "command:tag"}],
    }
    behavior["origins"] = [
        {"rule": "tag", "point": point, "guard_hash": tag_hash, "origin": "chosen"}
        for point in ("charset:A-Za-z0-9._-", "min_length:1", "max_length:64")
    ]
    write_json(behavior_path, behavior)
    handoff = run_ik(
        ik,
        [
            "handoff",
            "behavior-build.json",
            "--evidence",
            "out/sc-build",
            "--evidence",
            "out/check-build.json",
            "--kind",
            "delivery",
        ],
        work,
    )
    require_ok(handoff, "build handoff")
    return {
        "identity": ident_txt.stdout,
        "laws": laws.stdout,
        "check": check.stdout,
        "scenario": scenario.stdout,
        "report": report.stdout,
        "handoff": handoff.stdout,
    }


def json_dict(proc, label: str) -> Dict[str, object]:
    doc = parse_json(proc.stdout)
    if not isinstance(doc, dict):
        die(label + ": not JSON")
    return doc


def capture_change(ik: Path, work: Path, runtime: str, keep: set) -> Dict[str, str]:
    ident_v1 = identity(ik, work, "fixtures/loan_checkout.ik.toml")
    ident_v2 = identity(ik, work, "fixtures/loan_v2.ik.toml")
    keep.update(keep_hex(ident_v1))
    keep.update(keep_hex(ident_v2))
    ident_v1_txt = run_ik(ik, ["identity", "fixtures/loan_checkout.ik.toml"], work)
    ident_v2_txt = run_ik(ik, ["identity", "fixtures/loan_v2.ik.toml"], work)
    check_argv = [
        "check",
        "fixtures/loan_v2.ik.toml",
        "--prior",
        "fixtures/loan_checkout.ik.toml",
        "--corpus",
        "fixtures/loan_v2.ik.cases.json",
        "--profile",
        "fixtures/loan_checkout.ik.checks.toml",
    ]
    check = run_ik(ik, check_argv, work)
    require_ok(check, "change check")
    if "ACCEPT" not in check.stdout:
        die("change check: expected ACCEPT in rendered text")
    check_json = run_ik(ik, check_argv + ["--json"], work)
    require_ok(check_json, "change check json")
    parsed = json_dict(check_json, "change check json")
    write_json(work / "out" / "check-change.json", parsed)
    report = run_ik(ik, ["report", "out/check-change.json"], work)
    require_ok(report, "change report")
    report_json = run_ik(ik, ["report", "out/check-change.json", "--json"], work)
    require_ok(report_json, "change report json")
    report_doc = json_dict(report_json, "change report json")
    returned_id = first_finding_contains(report_doc, "#returned")
    if not returned_id:
        die("change report: missing returned finding")
    sc_out = work / "out" / "sc-change"
    sc_out.mkdir(parents=True, exist_ok=True)
    scenario = run_ik(
        ik,
        [
            "scenario",
            "run",
            "fixtures/loan_v2.ik.toml",
            "fixtures/scenarios/existing-loan.json",
            "--out",
            "out/sc-change",
        ],
        work,
    )
    require_ok(scenario, "change scenario")
    sc_report = run_ik(ik, ["report", "out/sc-change"], work)
    require_ok(sc_report, "change scenario report")
    sc_report_json = run_ik(ik, ["report", "out/sc-change", "--json"], work)
    require_ok(sc_report_json, "change scenario report json")
    sc_report_doc = parse_json(sc_report_json.stdout)
    if not isinstance(sc_report_doc, dict):
        die("change scenario report json: not JSON")
    under_id = first_finding_suffix(sc_report_doc, "/under_prior")
    recall_id = first_finding_suffix(sc_report_doc, "/recall")
    extend_id = first_finding_suffix(sc_report_doc, "/extend")
    if not under_id or not recall_id or not extend_id:
        die("change scenario report: missing continuity findings")
    store = work / "loan.sqlite"
    created = store_op(
        ik,
        work,
        store,
        "fixtures/loan_checkout.ik.toml",
        ident_v1,
        "create",
        {"aggregate_id": "loan-1", "request_id": "lc1"},
        runtime,
    )
    require_ok(created, "change create")
    created_doc = parse_json(created.stdout)
    cap0 = created_doc["result"]["capability"]
    checkout = store_op(
        ik,
        work,
        store,
        "fixtures/loan_checkout.ik.toml",
        ident_v1,
        "command",
        {
            "aggregate_id": "loan-1",
            "request_id": "co1",
            "capability": cap0,
            "operation_id": "op-checkout-1",
            "command": {
                "name": "checkout",
                "args": {
                    "checkout_ms": 1000,
                    "loan_until_ms": 1209601000,
                    "deposit_id": "dep-1",
                },
            },
        },
        runtime,
    )
    require_ok(checkout, "change checkout")
    checkout_doc = parse_json(checkout.stdout)
    cap1 = checkout_doc["result"]["capability"]
    shown = store_op(
        ik,
        work,
        store,
        "fixtures/loan_checkout.ik.toml",
        ident_v1,
        "show",
        {"aggregate_id": "loan-1", "request_id": "sh1"},
        runtime,
    )
    require_ok(shown, "change show")
    extend = store_op(
        ik,
        work,
        store,
        "fixtures/loan_v2.ik.toml",
        ident_v2,
        "command",
        {
            "aggregate_id": "loan-1",
            "request_id": "ext1",
            "capability": cap1,
            "command": {"name": "extend", "args": {"new_until_ms": 1209601001}},
        },
        runtime,
    )
    require_ok(extend, "change extend", expect=1)
    extend_doc = parse_json(extend.stdout)
    if not isinstance(extend_doc, dict) or extend_doc.get("diagnostic", {}).get("code") != "IK_MACHINE_MISMATCH":
        die("change extend: expected IK_MACHINE_MISMATCH")
    recall = store_op(
        ik,
        work,
        store,
        "fixtures/loan_checkout.ik.toml",
        ident_v1,
        "command",
        {
            "aggregate_id": "loan-1",
            "request_id": "rec1",
            "capability": cap1,
            "command": {"name": "recall"},
        },
        runtime,
    )
    require_ok(recall, "change recall")
    recall_doc = parse_json(recall.stdout)
    if not isinstance(recall_doc, dict) or recall_doc.get("result", {}).get("state", {}).get("recall_placed") != 1:
        die("change recall: expected recall_placed=1")
    definition_id = first_finding_contains(report_doc, "#definition")
    declared_outcomes = []
    if definition_id:
        declared_outcomes.append({
            "id": "C0",
            "text": "Version 2 adds Extend for loans created under the new rules. While a loan is on loan, Extend can set a later deadline; an equal or earlier deadline is refused.",
            "claim": "declared",
            "evidence": [definition_id],
        })
    behavior = {
        "behavior_version": 1,
        "title": "loan checkout version 2",
        "outcomes": declared_outcomes + [
            {
                "id": "C1",
                "text": "the new definition still reaches returned",
                "evidence": [returned_id],
            },
            {
                "id": "C2",
                "text": "instances created under version 1 keep version 1; recall still works on them; extend is refused on them with IK_MACHINE_MISMATCH",
                "claim": "continuity",
                "evidence": [under_id, recall_id, extend_id],
            },
        ],
        "why": {
            "text": "Borrowers need to extend a loan that is already out, without returning it first.",
            "source": "exercise brief for the loan change (a fixture requirement, not a real PRD)",
        },
        "unchanged": [],
        "assumptions": [],
        "rule_witnesses": [
            {
                "rule": "extend",
                "examples": [
                    {"args": {"new_until_ms": 200}, "facts": {"loan_until_ms": 100}, "outcome": "accepted"},
                    {"args": {"new_until_ms": 100}, "facts": {"loan_until_ms": 100}, "outcome": "rejected"},
                    {"args": {"new_until_ms": 50}, "facts": {"loan_until_ms": 100}, "outcome": "rejected"},
                ],
            }
        ],
    }
    write_json(work / "behavior-change.json", behavior)
    change_args = [
        "behavior-change.json", "--evidence", "out/check-change.json", "--evidence", "out/sc-change",
        "--kind", "change", "--store", "out/sc-change/scratch/store.sqlite",
    ]
    extend_hash = guard_hash(ik, work, change_args, "extend")
    behavior["requirement"] = {
        "text": "While a loan is on loan, Extend can set a later deadline; an equal or earlier deadline is refused.",
        "source": "exercise brief for the loan change (a fixture requirement, not a real PRD)",
        "provenance": "agent-restated",
        "atoms": [
            {"id": "R1", "span": "While a loan is on loan"},
            {"id": "R2", "span": "Extend can set a later deadline"},
            {"id": "R3", "span": "an equal or earlier deadline is refused."},
        ],
        "traces": [
            {"atom": "R1", "element": "state:on_loan"},
            {"atom": "R2", "element": "command:extend"},
            {"atom": "R3", "element": "command:extend"},
        ],
    }
    behavior["origins"] = [
        {"rule": "extend", "point": "operator:>", "guard_hash": extend_hash, "origin": "stated", "span": "a later deadline"}
    ]
    write_json(work / "behavior-change.json", behavior)
    handoff = run_ik(
        ik,
        [
            "handoff",
            "behavior-change.json",
            "--evidence",
            "out/check-change.json",
            "--evidence",
            "out/sc-change",
            "--kind",
            "change",
            "--store",
            "out/sc-change/scratch/store.sqlite",
        ],
        work,
    )
    require_ok(handoff, "change handoff")
    store_block = "\n".join(
        [
            "create:",
            created.stdout.strip(),
            "checkout:",
            checkout.stdout.strip(),
            "show:",
            shown.stdout.strip(),
            "extend under the version-2 definition:",
            extend.stdout.strip(),
            "recall under the version-1 definition:",
            recall.stdout.strip(),
        ]
    )
    return {
        "identity_v1": ident_v1_txt.stdout,
        "identity_v2": ident_v2_txt.stdout,
        "check": check.stdout,
        "report": report.stdout,
        "scenario": scenario.stdout,
        "scenario_report": sc_report.stdout,
        "store": stable_product_text(store_block, keep),
        "handoff": handoff.stdout,
    }


def capture_diagnose(ik: Path, work: Path) -> Dict[str, str]:
    out_dir = work / "out" / "sc-lost"
    out_dir.mkdir(parents=True, exist_ok=True)
    scenario = run_ik(
        ik,
        [
            "scenario",
            "run",
            "fixtures/tagged_item.ik.toml",
            "fixtures/scenarios/lost-reply.json",
            "--out",
            "out/sc-lost",
        ],
        work,
    )
    require_ok(scenario, "diagnose scenario")
    report = run_ik(ik, ["report", "out/sc-lost"], work)
    require_ok(report, "diagnose report")
    report_json = run_ik(ik, ["report", "out/sc-lost", "--json"], work)
    require_ok(report_json, "diagnose report json")
    report_doc = parse_json(report_json.stdout)
    if not isinstance(report_doc, dict):
        die("diagnose report json: not JSON")
    found_id = first_finding_suffix(report_doc, "/found")
    if not found_id:
        die("diagnose report: missing /found finding")
    found_row = None
    for finding in report_doc.get("findings") or []:
        if finding.get("id") == found_id:
            found_row = finding
            break
    if not found_row or found_row.get("status") != "committed_exact":
        die("diagnose report: /found status is not committed_exact")
    behavior = {
        "behavior_version": 1,
        "title": "lost reply is history",
        "outcomes": [
            {
                "id": "R1",
                "text": "lookup of the dropped commit is committed_exact",
                "evidence": [found_id],
            }
        ],
        "unchanged": [],
        "assumptions": [],
    }
    write_json(work / "behavior-recover.json", behavior)
    handoff = run_ik(
        ik,
        [
            "handoff",
            "behavior-recover.json",
            "--evidence",
            "out/sc-lost",
            "--kind",
            "recovery",
        ],
        work,
    )
    require_ok(handoff, "diagnose handoff")
    return {
        "scenario": scenario.stdout,
        "report": report.stdout,
        "handoff": handoff.stdout,
    }


def render_readout(
    version: str,
    build: Dict[str, str],
    change: Dict[str, str],
    diagnose: Dict[str, str],
) -> str:
    parts = [
        "# Human readout",
        "",
        "One unfamiliar reader, at most 20 minutes. The blocks below are the product's own rendered handoff text for three journeys. This file does not contain the answers.",
        "",
        "Packed binary version line:",
        "",
        fence(version),
        "## Journey 1 — BUILD",
        "",
        "Application under delivery: tagged item.",
        "",
        section(
            "Delivery handoff",
            cmd_line(
                [
                    "handoff",
                    "behavior-build.json",
                    "--evidence",
                    "out/sc-build",
                    "--evidence",
                    "out/check-build.json",
                    "--kind",
                    "delivery",
                ]
            ),
            build["handoff"],
        ),
        "### Questions",
        "",
        "1. What will the application do?",
        "2. What is not yet established?",
        "3. Does anything require your decision?",
        "",
        "## Journey 2 — CHANGE",
        "",
        "Change under review: loan checkout version 2.",
        "",
        section(
            "Change handoff",
            cmd_line(
                [
                    "handoff",
                    "behavior-change.json",
                    "--evidence",
                    "out/check-change.json",
                    "--evidence",
                    "out/sc-change",
                    "--kind",
                    "change",
                    "--store",
                    "out/sc-change/scratch/store.sqlite",
                ]
            ),
            change["handoff"],
        ),
        "### Questions",
        "",
        "1. What changes for new instances?",
        "2. What happens to existing ones?",
        "3. Is a migration or permission needed?",
        "",
        "## Journey 3 — DIAGNOSE / RECOVER",
        "",
        "Incident under review: a lost reply after tagging an item. Scenario: `fixtures/scenarios/lost-reply.json`.",
        "",
        section(
            "Recovery handoff",
            cmd_line(
                [
                    "handoff",
                    "behavior-recover.json",
                    "--evidence",
                    "out/sc-lost",
                    "--kind",
                    "recovery",
                ]
            ),
            diagnose["handoff"],
        ),
        "### Questions",
        "",
        "1. Did the operation commit?",
        "2. What is still uncertain?",
        "3. What action is safe next?",
        "",
    ]
    return "\n".join(parts)


def render_agent(
    version: str,
    build: Dict[str, str],
    change: Dict[str, str],
    diagnose: Dict[str, str],
) -> str:
    parts = [
        "# Agent interface fixture",
        "",
        "Technical command outputs for the three journeys. Not the human readout.",
        "",
        "Packed binary version line:",
        "",
        fence(version),
        "## BUILD",
        "",
        section(
            "Identity",
            cmd_line(["identity", "fixtures/tagged_item.ik.toml"]),
            build["identity"],
        ),
        section(
            "Laws inventory",
            cmd_line(["laws", "fixtures/tagged_item.ik.toml"]),
            build["laws"],
        ),
        section(
            "Developer check",
            cmd_line(
                [
                    "check",
                    "fixtures/tagged_item.ik.toml",
                    "--corpus",
                    "fixtures/tagged_item.ik.cases.json",
                    "--profile",
                    "fixtures/tagged_item.ik.checks.toml",
                ]
            ),
            build["check"],
        ),
        section(
            "Scenario run",
            cmd_line(
                [
                    "scenario",
                    "run",
                    "fixtures/tagged_item.ik.toml",
                    "fixtures/scenarios/honest-tag.json",
                    "--out",
                    "out/sc-build",
                ]
            ),
            build["scenario"],
        ),
        section(
            "Report",
            cmd_line(["report", "out/sc-build", "out/check-build.json"]),
            build["report"],
        ),
        "## CHANGE",
        "",
        section(
            "Identity of the previous definition",
            cmd_line(["identity", "fixtures/loan_checkout.ik.toml"]),
            change["identity_v1"],
        ),
        section(
            "Identity of the new definition",
            cmd_line(["identity", "fixtures/loan_v2.ik.toml"]),
            change["identity_v2"],
        ),
        section(
            "Developer check with prior",
            cmd_line(
                [
                    "check",
                    "fixtures/loan_v2.ik.toml",
                    "--prior",
                    "fixtures/loan_checkout.ik.toml",
                    "--corpus",
                    "fixtures/loan_v2.ik.cases.json",
                    "--profile",
                    "fixtures/loan_checkout.ik.checks.toml",
                ]
            ),
            change["check"],
        ),
        section(
            "Report of that check",
            cmd_line(["report", "out/check-change.json"]),
            change["report"],
        ),
        section(
            "Existing-instance continuity scenario",
            cmd_line(
                [
                    "scenario",
                    "run",
                    "fixtures/loan_v2.ik.toml",
                    "fixtures/scenarios/existing-loan.json",
                    "--out",
                    "out/sc-change",
                ]
            ),
            change["scenario"],
        ),
        section(
            "Report of that scenario",
            cmd_line(["report", "out/sc-change"]),
            change["scenario_report"],
        ),
        section(
            "Live store operations on an existing loan (product JSON as printed)",
            "`ik --store loan.sqlite --machine <definition> create|command|show`",
            change["store"],
        ),
        "## DIAGNOSE / RECOVER",
        "",
        section(
            "Scenario run",
            cmd_line(
                [
                    "scenario",
                    "run",
                    "fixtures/tagged_item.ik.toml",
                    "fixtures/scenarios/lost-reply.json",
                    "--out",
                    "out/sc-lost",
                ]
            ),
            diagnose["scenario"],
        ),
        section(
            "Report",
            cmd_line(["report", "out/sc-lost"]),
            diagnose["report"],
        ),
    ]
    return "\n".join(parts)


def generate(ik: Path, fixtures: Path) -> Tuple[str, str]:
    version_proc = run_ik(ik, ["--version"], fixtures)
    require_ok(version_proc, "version")
    version = version_proc.stdout.strip()
    runtime = version.split()[1] if len(version.split()) > 1 else "0.5.0"
    tmp = Path(tempfile.mkdtemp(prefix="ik-human-"))
    try:
        work = tmp / "work"
        shutil.copytree(fixtures, work / "fixtures")
        (work / "out").mkdir()
        keep = set()
        tagged = identity(ik, work, "fixtures/tagged_item.ik.toml")
        keep.update(keep_hex(tagged))
        build = capture_build(ik, work)
        change = capture_change(ik, work, runtime, keep)
        diagnose = capture_diagnose(ik, work)
        mapping = mapping_for(tmp, ik, work)
        human = redact_paths(render_readout(version_proc.stdout, build, change, diagnose), mapping)
        agent = redact_paths(render_agent(version_proc.stdout, build, change, diagnose), mapping)
        if "$HOME" in human or "$HOME" in agent:
            die("readout leaked a home path")
        return human, agent
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main() -> int:
    ik = resolve_ik()
    fixtures = DEFAULT_FIXTURES
    if not fixtures.is_dir():
        die("fixtures not found at %s" % fixtures)
    human, agent = generate(ik, fixtures)
    OUT_PATH.write_text(human, encoding="utf-8")
    AGENT_PATH.write_text(agent, encoding="utf-8")
    sys.stdout.write(
        "wrote %s (%s bytes) and %s (%s bytes)\n"
        % (
            OUT_PATH.name,
            len(human.encode("utf-8")),
            AGENT_PATH.name,
            len(agent.encode("utf-8")),
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
