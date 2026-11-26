from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

from conftest import EFFECT, REPO
from invariant_kernel import (
    Aggregate,
    EffectCompletionError,
    EffectConflictError,
    EffectIntent,
    EffectMismatchError,
    HistoricalResult,
    Kernel,
    ProtocolError,
    TransportError,
)


def _machine(ik_bin: Path, store: Path):
    return Kernel(store, binary=str(ik_bin)).machine(EFFECT)


def _wire(intent: EffectIntent, outcome: str) -> dict:
    return {
        "effect_id": intent.effect_id,
        "aggregate_id": intent.aggregate_id,
        "planned_generation": intent.generation,
        "payload_digest": intent.payload_digest,
        "idempotency_key": intent.idempotency_key,
        "outcome_digest": outcome,
    }


def test_outcome_bound_to_original_intent_not_later_generation(ik_bin: Path, store: Path) -> None:
    probe = _machine(ik_bin, store)
    planned = probe.create("fx").plan(effect_id="e1", payload_digest="p1", idempotency_key="k1")
    assert isinstance(planned, Aggregate)
    intent = planned.pending_intent
    assert intent is not None
    advanced = planned.finish(result_digest="later")
    assert isinstance(advanced, Aggregate)
    assert advanced.generation != intent.generation
    assert advanced.pending_intent is not None
    assert advanced.pending_intent.generation == intent.generation
    recorded = advanced.record_effect_outcome("o1", intent=intent)
    assert isinstance(recorded, Aggregate)
    assert recorded.effect.status == "recorded"
    assert recorded.effect.outcome_digest == "o1"
    assert recorded.effect.intent is not None
    assert recorded.effect.intent.generation == intent.generation
    assert recorded.effect.intent.effect_id == intent.effect_id
    assert recorded.effect.intent.payload_digest == intent.payload_digest


def test_repeat_is_noop_and_conflict_keeps_first(ik_bin: Path, store: Path) -> None:
    probe = _machine(ik_bin, store)
    planned = probe.create("fx").plan(effect_id="e1", payload_digest="p1", idempotency_key="k1")
    assert isinstance(planned, Aggregate)
    intent = planned.pending_intent
    assert intent is not None
    first = planned.record_effect_outcome("o1", intent=intent, operation_id="rec-1")
    assert isinstance(first, Aggregate)
    assert first.mutation is True
    replay = planned.record_effect_outcome("o1", intent=intent, operation_id="rec-1")
    assert isinstance(replay, HistoricalResult)
    again = planned.record_effect_outcome("o1")
    assert isinstance(again, Aggregate)
    assert again.mutation is False
    with pytest.raises(EffectConflictError) as exc:
        first.record("other", operation_id="rec-2")
    assert exc.value.code == "IK_EFFECT_OUTCOME_CONFLICT"
    shown = probe.show("fx")
    assert shown.effect.outcome_digest == "o1"
    assert shown.generation == first.generation


def test_wrong_binding_is_kernel_diagnostic_and_records_nothing(ik_bin: Path, store: Path) -> None:
    kernel = Kernel(store, binary=str(ik_bin))
    probe = kernel.machine(EFFECT)
    planned = probe.create("fx").plan(effect_id="e1", payload_digest="p1", idempotency_key="k1")
    assert isinstance(planned, Aggregate)
    intent = planned.pending_intent
    assert intent is not None
    other = probe.create("other").plan(effect_id="e-other", payload_digest="p-other", idempotency_key="k-other")
    assert isinstance(other, Aggregate)
    other_intent = other.pending_intent
    assert other_intent is not None
    with pytest.raises(EffectConflictError) as generation:
        planned.record("o-bad", planned_generation=intent.generation + 9, operation_id="bad-gen")
    assert generation.value.code == "IK_EFFECT_OUTCOME_CONFLICT"
    with pytest.raises(EffectConflictError) as digest:
        planned.record("o-bad", payload_digest="not-the-planned-digest", operation_id="bad-digest")
    assert digest.value.code == "IK_EFFECT_OUTCOME_CONFLICT"
    with pytest.raises(EffectMismatchError) as foreign:
        kernel.raw().call(
            "record",
            bound_machine=probe,
            aggregate_id=planned.id,
            capability=planned.capability,
            operation_id="bad-foreign",
            effect={
                "effect_id": other_intent.effect_id,
                "aggregate_id": other.id,
                "planned_generation": other_intent.generation,
                "payload_digest": other_intent.payload_digest,
                "idempotency_key": other_intent.idempotency_key,
                "outcome_digest": "o-foreign",
            },
        )
    assert foreign.value.code == "IK_EFFECT_MISMATCH"
    shown = probe.show(planned.id)
    assert shown.generation == planned.generation
    assert shown.effect.status == "pending"
    assert shown.effect.outcome_digest is None


def test_empty_digest_and_foreign_intent_are_pre_dispatch(ik_bin: Path, store: Path, tmp_path: Path) -> None:
    kernel = Kernel(store, binary=str(ik_bin))
    probe = kernel.machine(EFFECT)
    planned = probe.create("fx").plan(effect_id="e1", payload_digest="p1", idempotency_key="k1")
    assert isinstance(planned, Aggregate)
    intent = planned.pending_intent
    assert intent is not None
    log = tmp_path / "sent.log"
    stub = tmp_path / "log-ik"
    stub.write_text("#!/bin/sh\necho sent >> %s\n" % log)
    stub.chmod(0o755)
    kernel.binary = str(stub)
    with pytest.raises(EffectCompletionError) as empty:
        planned.record_effect_outcome("", operation_id="empty-1")
    assert empty.value.submission == "not-sent"
    assert empty.value.operation_id == "empty-1"
    wrong = EffectIntent(
        {
            "effect_id": intent.effect_id,
            "aggregate_id": intent.aggregate_id,
            "generation": intent.generation + 4,
            "effect_kind": intent.effect_kind,
            "payload_digest": intent.payload_digest,
            "idempotency_key": intent.idempotency_key,
        }
    )
    with pytest.raises(EffectCompletionError) as foreign:
        planned.record_effect_outcome("o1", intent=wrong, operation_id="wrong-intent")
    assert foreign.value.submission == "not-sent"
    assert not log.exists()
    kernel.binary = str(ik_bin)
    shown = probe.show(planned.id)
    assert shown.generation == planned.generation
    lookup = probe.lookup_operation(
        planned.id,
        planned.capability,
        "empty-1",
        effect=_wire(intent, "unused"),
    )
    assert lookup.outcome == "not_recorded"


def test_partial_effect_is_protocol_error_not_transport(ik_bin: Path, store: Path) -> None:
    kernel = Kernel(store, binary=str(ik_bin))
    probe = kernel.machine(EFFECT)
    planned = probe.create("fx").plan(effect_id="e1", payload_digest="p1", idempotency_key="k1")
    assert isinstance(planned, Aggregate)
    with pytest.raises(ProtocolError) as exc:
        kernel.raw().call(
            "record",
            bound_machine=probe,
            aggregate_id=planned.id,
            capability=planned.capability,
            operation_id="partial-1",
            effect={"effect_id": "e1"},
        )
    assert exc.value.code == "IK_PROTOCOL_INVALID"
    assert not isinstance(exc.value, TransportError)
    shown = probe.show(planned.id)
    assert shown.generation == planned.generation
    assert shown.effect.status == "pending"


def test_dropped_reply_lookup_does_not_record_again(ik_bin: Path, store: Path) -> None:
    probe = _machine(ik_bin, store)
    planned = probe.create("fx").plan(effect_id="e1", payload_digest="p1", idempotency_key="k1")
    assert isinstance(planned, Aggregate)
    intent = planned.pending_intent
    assert intent is not None
    planned.record_effect_outcome("o-dropped", intent=intent, operation_id="dropped-1")
    fresh = Kernel(store, binary=str(ik_bin)).machine(EFFECT)
    found = fresh.lookup_operation(
        planned.id,
        planned.capability,
        "dropped-1",
        effect=_wire(intent, "o-dropped"),
    )
    assert found.outcome == "committed_exact"
    shown = fresh.show(planned.id)
    assert shown.effect.outcome_digest == "o-dropped"
    assert shown.generation == planned.generation + 1


def test_helper_does_not_call_service_or_retry_transport(ik_bin: Path, store: Path, tmp_path: Path) -> None:
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
    import threading

    hits = []

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            hits.append(1)
            self.send_response(200)
            self.end_headers()

        def log_message(self, fmt, *args):
            return

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        kernel = Kernel(store, binary=str(ik_bin))
        probe = kernel.machine(EFFECT)
        planned = probe.create("fx").plan(effect_id="e1", payload_digest="p1", idempotency_key="k1")
        assert isinstance(planned, Aggregate)
        planned.record_effect_outcome("o-local", intent=planned.pending_intent)
        assert hits == []
    finally:
        server.shutdown()
    log = tmp_path / "dispatches.log"
    stub = tmp_path / "slow-ik"
    stub.write_text("#!/bin/sh\necho dispatched >> %s\nsleep 5\n" % log)
    stub.chmod(0o755)
    slow = Kernel(store, binary=str(ik_bin), timeout=0.3)
    probe = slow.machine(EFFECT)
    planned = probe.create("slow").plan(effect_id="e1", payload_digest="p1", idempotency_key="k1")
    assert isinstance(planned, Aggregate)
    slow.binary = str(stub)
    with pytest.raises(TransportError) as exc:
        planned.record_effect_outcome("o-lost", operation_id="maybe-recorded")
    assert exc.value.submission == "submission-unknown"
    assert log.read_text().strip().splitlines() == ["dispatched"]


def test_python_recipe_runs_against_ik_and_a_fake_service(ik_bin: Path, tmp_path: Path) -> None:
    docs = (REPO / "docs" / "python.md").read_text()
    marker = "```python\n# effect-completion-recipe\n"
    start = docs.index(marker) + len(marker)
    source = docs[start:docs.index("```", start)]
    script = tmp_path / "recipe.py"
    script.write_text(source)
    store = tmp_path / "store.db"
    env = os.environ.copy()
    env["STORE"] = str(store)
    env["MACHINE_PATH"] = str(EFFECT)
    env["IK_BINARY"] = str(ik_bin)
    env["PYTHONPATH"] = str(REPO / "python" / "src")
    result = subprocess.run(
        [sys.executable, str(script)],
        env=env,
        text=True,
        capture_output=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    summary = json.loads(result.stdout.strip().splitlines()[-1])
    expected = hashlib.sha256(summary["responseBody"].encode("utf-8")).hexdigest()
    assert summary["serviceCalls"] == 1
    assert summary["lookup"] == "committed_exact"
    assert summary["plannedGeneration"] == 1
    assert summary["recordedGeneration"] == 1
    assert summary["recordedOutcome"] == expected
    assert summary["state"] == "finished"
    shown = Kernel(store, binary=str(ik_bin)).machine(EFFECT).show(summary["aggregateId"])
    assert shown.state == "finished"
    assert shown.effect.status == "recorded"
    assert shown.effect.outcome_digest == expected
    assert shown.effect.intent is not None
    assert shown.effect.intent.generation == 1
    assert shown.effect.intent.effect_id == "publish-1"


def test_run_effect_helper_plans_executes_and_refuses_duplicate_plan(ik_bin: Path, store: Path) -> None:
    kernel = Kernel(store, binary=str(ik_bin))
    probe = kernel.machine(EFFECT)
    created = probe.create("fx-run-1")
    seen_keys: list[str] = []

    def execute(intent):
        seen_keys.append(intent.idempotency_key)
        return ("succeeded", {"charge": "ok"})

    res = created.run_effect(
        "plan",
        {"effect_id": "e-run-1", "payload_digest": "p-run-1", "idempotency_key": "k-run-1"},
        executor=execute,
    )
    assert seen_keys == ["k-run-1"]
    assert res.outcome_digest == "succeeded"
    assert res.result == {"charge": "ok"}
    assert isinstance(res.recorded, Aggregate)
    assert res.recorded.effect.status == "recorded"
    assert res.recorded.effect.outcome_digest == "succeeded"

    # Replaying the same plan_operation_id must refuse instead of re-running executor.
    c2 = probe.create("fx-run-2")
    c2.execute(
        {"name": "plan", "args": {"effect_id": "e-run-2", "payload_digest": "p-run-2", "idempotency_key": "k-run-2"}},
        operation_id="dup-plan-op",
    )
    ran = False

    def should_not_run(_intent):
        nonlocal ran
        ran = True
        return "succeeded"

    with pytest.raises(EffectCompletionError):
        c2.run_effect(
            "plan",
            {"effect_id": "e-run-2", "payload_digest": "p-run-2", "idempotency_key": "k-run-2"},
            executor=should_not_run,
            plan_operation_id="dup-plan-op",
        )
    assert ran is False
