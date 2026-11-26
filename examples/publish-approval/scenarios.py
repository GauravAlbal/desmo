"""Executable scenarios for the exact-artifact approval app (IK-09).

Each scenario drives the real `ik` binary and returns the facts it observed. Nothing here
scripts a kernel answer: refusals, histories and unknowns in the emitted traces are whatever
the run produced. Run `python3 scenarios.py --out DIR` to write the two trace bundles.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Optional

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(REPO / "python" / "src"))

from approval_app import (  # noqa: E402
    ApprovalApp,
    ArtifactStore,
    DraftStore,
    ExecutorSubstitutionError,
    LatestContentExecutor,
    NaiveRollbackRouter,
    Projection,
    PublishExecutor,
    Trace,
    VersionRouter,
    sha256_hex,
)
from invariant_kernel import (  # noqa: E402
    EffectConflictError,
    HistoricalResult,
    Kernel,
    KernelError,
    TransportError,
)

DEFINITIONS = {
    1: REPO / "machines" / "publish_approval.ik.toml",
    2: REPO / "machines" / "publish_approval_v2.ik.toml",
    3: HERE / "machines" / "publish_approval_v3.ik.toml",
}
DEFECTIVE = HERE / "machines" / "publish_approval_defective.ik.toml"
DEFECTIVE_CORPUS = HERE / "machines" / "publish_approval_defective.cases.json"
ALL_REFUSE = HERE / "machines" / "publish_approval_all_refuse.ik.toml"
V3_CORPUS = HERE / "machines" / "publish_approval_v3.cases.json"
DOC_A = b"quarterly report, approved wording"
DOC_B = b"quarterly report, silently changed wording"


def default_binary() -> str:
    return os.environ.get("IK_BINARY") or str(REPO / "target" / "debug" / "ik")


@dataclass
class Env:
    root: Path
    kernel: Kernel
    app: ApprovalApp
    trace: Trace
    artifacts: ArtifactStore
    drafts: DraftStore
    sink: Path
    projection: Projection


def make_env(root: Path, binary: str, create_version: int = 3, router_cls: type = VersionRouter) -> Env:
    root.mkdir(parents=True, exist_ok=True)
    kernel = Kernel(root / "store.db", binary=binary)
    router = router_cls(kernel, DEFINITIONS, create_version)
    trace, artifacts, drafts = Trace(), ArtifactStore(root), DraftStore(root)
    sink = root / "sink"
    projection = Projection(root / "projection.json")
    app = ApprovalApp(router, artifacts, drafts, sink, trace, projection)
    return Env(root, kernel, app, trace, artifacts, drafts, sink, projection)


def refusal_code(call: Any) -> Optional[str]:
    try:
        call()
    except KernelError as err:
        return err.code
    return None


def approved_doc(env: Env, doc_id: str, data: bytes = DOC_A) -> Any:
    doc = env.app.create(doc_id)
    doc = env.app.edit(doc, data)
    return env.app.approve(doc)


def publish_exact(env: Env, doc_id: str) -> dict[str, Any]:
    doc = env.app.fresh(doc_id)
    doc = env.app.authorize_publish(doc)
    published = env.app.run_executor(doc_id)
    doc = env.app.record_outcome(doc, published)
    doc = env.app.complete(doc, published.digest)
    return {"state": doc.state, "published": published, "doc": doc}


def scenario_approve_then_mutate(env: Env) -> dict[str, Any]:
    """Approve A, edit to B: publication of either is refused until B is reapproved."""
    doc = approved_doc(env, "doc-mut")
    digest_a, digest_b = sha256_hex(DOC_A), sha256_hex(DOC_B)
    approved_but_b = refusal_code(lambda: env.app.authorize_publish(doc, payload_digest=digest_b))
    revised = env.app.revise(doc, DOC_B)
    authorize_in_draft = refusal_code(lambda: env.app.authorize_publish(revised))
    reapproved = env.app.approve(revised)
    result = publish_exact(env, "doc-mut")
    sink_bytes = Path(result["published"].location).read_bytes()
    return {
        "digest_a": digest_a,
        "digest_b": digest_b,
        "approved_but_b": approved_but_b,
        "authorize_in_draft": authorize_in_draft,
        "reapproved_state": reapproved.state,
        "final_state": result["state"],
        "published_digest": result["published"].digest,
        "sink_bytes": sink_bytes,
    }


def scenario_latest_content(env: Env) -> dict[str, Any]:
    """A worker that reads the mutable working copy is refused before anything is recorded."""
    approved_doc(env, "doc-latest")
    doc = env.app.authorize_publish(env.app.fresh("doc-latest"))
    env.drafts.write("doc-latest", DOC_B)  # edited after authorization, outside the kernel
    bad = LatestContentExecutor(env.artifacts, env.sink, env.drafts, "doc-latest")
    substituted: Optional[str] = None
    try:
        env.app.run_executor("doc-latest", bad)
    except ExecutorSubstitutionError as err:
        substituted = str(err)
    wrong_complete = refusal_code(lambda: env.app.complete(doc, sha256_hex(DOC_B)))
    good = env.app.run_executor("doc-latest")
    return {
        "substituted": substituted,
        "effect_after_refusal": env.app.fresh("doc-latest").effect.status,
        "wrong_complete": wrong_complete,
        "good_digest": good.digest,
        "authorized_digest": sha256_hex(DOC_A),
    }


def scenario_effect_outcomes(env: Env) -> dict[str, Any]:
    """record is idempotent for the same outcome and refuses a conflicting one."""
    approved_doc(env, "doc-effect")
    doc = env.app.authorize_publish(env.app.fresh("doc-effect"))
    published = env.app.run_executor("doc-effect")
    recorded = env.app.record_outcome(doc, published)
    duplicate = env.app.record_outcome(env.app.fresh("doc-effect"), published)
    conflicting = refusal_code(
        lambda: env.app.fresh("doc-effect").record(sha256_hex(b"a different outcome"))
    )
    conflict_type = None
    try:
        env.app.fresh("doc-effect").record(sha256_hex(b"a different outcome"))
    except EffectConflictError:
        conflict_type = "EffectConflictError"
    return {
        "recorded_mutation": recorded.mutation,
        "duplicate_mutation": duplicate.mutation,
        "conflict_code": conflicting,
        "conflict_type": conflict_type,
        "outcome": env.app.fresh("doc-effect").effect.outcome_digest,
        "first_outcome": published.outcome_digest,
    }


class _DiesAfterSinkWrite(PublishExecutor):
    def execute(self, effect_id: str, payload_digest: str) -> Any:
        super().execute(effect_id, payload_digest)
        raise RuntimeError("executor process died after the external write")


def scenario_executor_uncertainty(env: Env) -> dict[str, Any]:
    """The external write happened but was never recorded: recover without claiming exactly-once."""
    approved_doc(env, "doc-unc")
    doc = env.app.authorize_publish(env.app.fresh("doc-unc"))
    died = False
    try:
        env.app.run_executor("doc-unc", _DiesAfterSinkWrite(env.artifacts, env.sink))
    except RuntimeError:
        died = True
    pending = env.app.fresh("doc-unc")
    retried = env.app.run_executor("doc-unc")  # at-least-once: runs again against an idempotent sink
    env.app.record_outcome(pending, retried)
    done = env.app.complete(env.app.fresh("doc-unc"), retried.digest)
    return {
        "died": died,
        "status_while_unknown": pending.effect.status,
        "sink_duplicate_on_retry": retried.duplicate,
        "final_state": done.state,
        "authorized": doc.effect.intent.payload_digest if doc.effect.intent else None,
        "published_digest": retried.digest,
    }


def scenario_lost_reply(env: Env) -> dict[str, Any]:
    """The reply to a committed command is lost; reconcile with the operation id, not a retry."""
    doc = approved_doc(env, "doc-lost")
    digest = str(doc.facts["approved_digest"])
    effect_id = f"publish-doc-lost-{digest[:12]}"
    command = {
        "name": "authorize_publish",
        "args": {"effect_id": effect_id, "payload_digest": digest, "idempotency_key": effect_id},
    }
    op_id = env.kernel.new_operation_id()
    capability = doc.capability
    doc.execute(command, operation_id=op_id)  # committed; the caller "never saw" the answer
    env.kernel.machine(DEFINITIONS[3]).show("doc-lost").record(sha256_hex(b"other writer"))
    machine = env.app.router.machine_for("doc-lost")
    lookup = machine.lookup_operation("doc-lost", capability, op_id, command=command)
    if lookup.history is not None:
        env.trace.historical("authorize_publish (reply lost)", lookup.history)
    return {
        "outcome": lookup.outcome,
        "history_is_historical": isinstance(lookup.history, HistoricalResult),
        "current_generation": lookup.current_generation,
        "stale_retry_code": refusal_code(lambda: doc.execute(command)),
    }


def scenario_timeout_unknown(env: Env) -> dict[str, Any]:
    """A real client timeout: the answer is unknown, and only lookup_operation may resolve it."""
    doc = approved_doc(env, "doc-timeout")
    digest = str(doc.facts["approved_digest"])
    effect_id = f"publish-doc-timeout-{digest[:12]}"
    command = {
        "name": "authorize_publish",
        "args": {"effect_id": effect_id, "payload_digest": digest, "idempotency_key": effect_id},
    }
    op_id = env.kernel.new_operation_id()
    capability = doc.capability
    machine = env.app.router.machine_for("doc-timeout")
    kind = None
    original = env.kernel.timeout
    env.kernel.timeout = 0.000001
    try:
        env.app.authorize_publish(doc, operation_id=op_id)
    except TransportError as err:
        kind = err.submission
    finally:
        env.kernel.timeout = original
    lookup = machine.lookup_operation("doc-timeout", capability, op_id, command=command)
    return {"submission": kind, "lookup": lookup.outcome}


def death_script(store: Path, binary: str) -> str:
    return (
        "import os, sys\n"
        f"sys.path.insert(0, {str(REPO / 'python' / 'src')!r})\n"
        f"sys.path.insert(0, {str(HERE)!r})\n"
        "import scenarios as s\n"
        f"env = s.make_env(__import__('pathlib').Path({str(store)!r}), {binary!r})\n"
        "s.approved_doc(env, 'doc-death')\n"
        "env.app.authorize_publish(env.app.fresh('doc-death'))\n"
        "os._exit(17)\n"
    )


def scenario_process_death(root: Path, binary: str) -> dict[str, Any]:
    """The worker process dies after an authorization committed; a new process recovers."""
    proc = subprocess.run([sys.executable, "-c", death_script(root, binary)], capture_output=True, text=True)
    env = make_env(root, binary)
    doc = env.app.fresh("doc-death")
    published = env.app.run_executor("doc-death")
    env.app.record_outcome(doc, published)
    done = env.app.complete(env.app.fresh("doc-death"), published.digest)
    machine = env.app.router.machine_for("doc-death")
    return {
        "exit_code": proc.returncode,
        "recovered_state": done.state,
        "verify": machine.verify("doc-death"),
        "replay": machine.replay("doc-death"),
    }


def scenario_projection(env: Env) -> dict[str, Any]:
    """A stale or wrong read model is repaired from canonical state; it never answers 'did it commit'."""
    approved_doc(env, "doc-proj")
    env.projection.rows["doc-proj"] = {"state": "published", "generation": 99}
    claimed = env.projection.rows["doc-proj"]["state"]
    canonical = env.app.reconcile_projection("doc-proj")
    return {
        "projection_claimed": claimed,
        "canonical_state": canonical.state,
        "projection_after": env.projection.rows["doc-proj"]["state"],
    }


def scenario_versions(env: Env) -> dict[str, Any]:
    """v1, v2 and v3 aggregates share one store; rollback changes only where new ones start."""
    router = env.app.router
    ids = {}
    for version in (1, 2, 3):
        router.create_version = version
        ids[version] = f"doc-v{version}"
        env.app.create(ids[version])
    router.create_version = 2  # roll the application back
    before_rollback = router.machine_for(ids[3]).identity.version
    router.draining.add(3)
    naive = NaiveRollbackRouter(env.kernel, DEFINITIONS, 2)
    naive_code = refusal_code(
        lambda: naive.machine_for(ids[3]).show(ids[3]).edit(artifact_digest=sha256_hex(DOC_A))
    )
    audits = {}
    for version in (1, 2, 3):
        machine = router.machine_for(ids[version])
        audits[version] = {"verify": machine.verify(ids[version]), "replay": machine.replay(ids[version])}
    return {
        "routed_versions": {v: router.machine_for(ids[v]).identity.version for v in ids},
        "v3_after_rollback": before_rollback,
        "naive_rollback_code": naive_code,
        "audits": audits,
    }


def check_json(machine: Path, corpus: Path, binary: str) -> dict[str, Any]:
    out = subprocess.run(
        [binary, "check", str(machine), "--corpus", str(corpus), "--json"],
        capture_output=True,
        text=True,
    )
    return json.loads(out.stdout)


def scenario_defective(root: Path, binary: str) -> dict[str, Any]:
    """The same invariant on a defective machine: runtime refusal and checker witness agree."""
    kernel = Kernel(root / "defective.db", binary=binary)
    doc = kernel.machine(DEFECTIVE).create("doc-def")
    doc = doc.edit(artifact_digest=sha256_hex(DOC_A))
    doc = doc.approve(artifact_digest=sha256_hex(DOC_A))
    doc = doc.revise(artifact_digest=sha256_hex(DOC_B))
    states = [doc.state]
    code = refusal_code(lambda: doc.publish())
    report = check_json(DEFECTIVE, DEFECTIVE_CORPUS, binary)
    return {
        "state_before_publish": states[0],
        "runtime_code": code,
        "check_verdict": report["judgment"]["verdict"],
        "witnesses": report["invariant_witnesses"],
    }


def scenario_all_refuse(binary: str) -> dict[str, Any]:
    report = check_json(ALL_REFUSE, V3_CORPUS, binary)
    return {"verdict": report["judgment"]["verdict"], "release": report["release"]["release"]["verdict"]}


def scenario_correct_check(binary: str) -> dict[str, Any]:
    report = check_json(DEFINITIONS[3], V3_CORPUS, binary)
    return {"verdict": report["judgment"]["verdict"], "release": report["release"]["release"]["verdict"]}


def trace_bundle(origin: str, events: list[dict[str, str]], reproduction: str) -> dict[str, Any]:
    return {
        "schema": "ikaros.trace.v2",
        "product": "Ikaros",
        "runtime_version": "0.6.0",
        "machine_id": "publish_approval",
        "advisory": True,
        "origin": origin,
        "reproducible": False,
        "redacted": False,
        "events": events,
        "reproduction": reproduction,
    }


def witness_events(binary: str, root: Path) -> list[dict[str, str]]:
    """Turn the checker's bounded witness into witness events, states observed by replaying it."""
    report = check_json(DEFECTIVE, DEFECTIVE_CORPUS, binary)
    kernel = Kernel(root / "witness.db", binary=binary)
    doc = kernel.machine(DEFECTIVE).create("doc-witness")
    events: list[dict[str, str]] = []
    steps = report["invariant_witnesses"][0]["steps"]
    for step in steps:
        corpus = json.loads(DEFECTIVE_CORPUS.read_text())["commands"].get(step["command"], [])
        args = next((c["args"] for c in corpus if c["name"] == step.get("case")), {})
        before = doc.state
        label = step["command"] + (f"[{step['case']}]" if step.get("case") else "")
        try:
            doc = doc.execute({"name": step["command"], "args": args})
            after = doc.state
        except KernelError as err:
            after = f"blocked by {err.code}"
        events.append({"kind": "witness", "attempted": label, "fromState": str(before), "toState": str(after)})
    return events


def run_all(out: Path, binary: str) -> dict[str, Any]:
    out.mkdir(parents=True, exist_ok=True)
    main = make_env(out / "main", binary)
    results = {
        "approve_then_mutate": scenario_approve_then_mutate(main),
        "latest_content": scenario_latest_content(main),
        "effect_outcomes": scenario_effect_outcomes(main),
        "executor_uncertainty": scenario_executor_uncertainty(main),
        "lost_reply": scenario_lost_reply(main),
        "timeout_unknown": scenario_timeout_unknown(main),
        "projection": scenario_projection(main),
        "versions": scenario_versions(main),
        "process_death": scenario_process_death(out / "death", binary),
        "defective": scenario_defective(out, binary),
        "all_refuse": scenario_all_refuse(binary),
        "correct_check": scenario_correct_check(binary),
    }
    observed = trace_bundle("observed", main.trace.events, "python3 examples/publish-approval/scenarios.py")
    witness = trace_bundle(
        "bounded-witness",
        witness_events(binary, out),
        "ik check examples/publish-approval/machines/publish_approval_defective.ik.toml --corpus "
        "examples/publish-approval/machines/publish_approval_defective.cases.json --json",
    )
    (out / "observed.trace.json").write_text(json.dumps(observed, indent=2, default=str))
    (out / "witness.trace.json").write_text(json.dumps(witness, indent=2))
    results["traces"] = {"observed": observed, "witness": witness}
    return results


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, default=Path(tempfile.mkdtemp(prefix="ik09-")))
    parser.add_argument("--binary", default=default_binary())
    args = parser.parse_args()
    results = run_all(args.out, args.binary)
    summary = {k: v for k, v in results.items() if k != "traces"}
    print(json.dumps(summary, indent=2, default=lambda o: getattr(o, "__dict__", str(o))))
    print(f"traces: {args.out}/observed.trace.json {args.out}/witness.trace.json", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
