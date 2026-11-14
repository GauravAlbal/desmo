from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path
from typing import Any, Mapping

import pytest

from invariant_kernel import Kernel
from invariant_kernel import transport
from invariant_kernel.errors import KernelError
from invariant_kernel.reducer import canonical_json
from reducer_fixtures import ParityReducer


REPO = Path(__file__).resolve().parents[2]
CORPUS_PATH = REPO / "conformance" / "reducers" / "native_parity_v1.json"
ENTRYPOINT = "reducer_fixtures:ParityReducer"


def corpus() -> dict[str, Any]:
    return json.loads(CORPUS_PATH.read_text(encoding="utf-8"))


def reducer_kernel(ik_bin: Path, store: Path, **kwargs: Any) -> Kernel:
    return Kernel(store, binary=str(ik_bin), **kwargs)


def worker_result(machine: Any, op: str, **fields: Any) -> Mapping[str, Any]:
    del machine
    identity = ParityReducer.identity.to_wire() if op != "describe" else None
    response = transport.invoke_worker(
        ENTRYPOINT,
        identity,
        op=op,
        python=sys.executable,
        worker_timeout=3.0,
        **fields,
    )
    return response["result"]


def assert_process_gone(pid: int) -> None:
    deadline = time.monotonic() + 2
    while time.monotonic() < deadline:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return
        time.sleep(0.01)
    pytest.fail(f"reducer descendant {pid} survived cleanup")



def test_python_float_canonicalization_matches_rust_contract() -> None:
    assert canonical_json(
        {"float_boundaries": [1e-5, 1e-6, 1e-7, 1e16, 1e20, -0.0]}
    ) == '{"float_boundaries":[0.00001,1e-6,1e-7,1e+16,1e+20,-0.0]}'


def test_shared_json_integer_range_is_exact() -> None:
    assert canonical_json({"lower": -(1 << 63), "upper": (1 << 64) - 1})
    for value in (-(1 << 63) - 1, 1 << 64):
        with pytest.raises(ValueError, match="shared JSON range"):
            canonical_json({"value": value})

def test_external_worker_reads_every_shared_vector(ik_bin: Path, store: Path) -> None:
    vectors = corpus()
    machine = reducer_kernel(ik_bin, store).reducer(ENTRYPOINT, python=sys.executable)
    initial = vectors["initial_state"]
    assert worker_result(machine, "describe")["initial_state"] == initial
    for case in vectors["cases"]:
        fields = {
            "state": initial,
            "effect": {"status": "none"},
            "command": case["command"],
            "observation": case["observation"],
        }
        if "diagnostic" in case:
            with pytest.raises(KernelError) as raised:
                worker_result(machine, "decide", **fields)
            assert raised.value.code == case["diagnostic"]["code"]
            assert raised.value.rule == case["diagnostic"]["rule"]
            continue
        raw = worker_result(machine, "decide", **fields)
        decision = {"kind": raw["decision"]}
        for key in ("event", "note"):
            if key in raw:
                decision[key] = raw[key]
        assert decision == case["decision"]
        if "apply" in case:
            folded = worker_result(machine, "apply", state=initial, event=raw["event"])
            assert folded == case["apply"]

def test_external_replay_is_deterministic_and_observation_is_frozen(ik_bin: Path, store: Path) -> None:
    vectors = corpus()
    machine = reducer_kernel(ik_bin, store).reducer(ENTRYPOINT, python=sys.executable)
    state = vectors["initial_state"]
    for event in vectors["replay"]:
        state = worker_result(machine, "apply", state=state, event=event)["state"]
    assert state == vectors["replay_state"]
    accepted = vectors["cases"][0]
    first = worker_result(
        machine,
        "decide",
        state=vectors["initial_state"],
        effect={"status": "none"},
        command=accepted["command"],
        observation=accepted["observation"],
    )
    second = worker_result(
        machine,
        "decide",
        state=vectors["initial_state"],
        effect={"status": "none"},
        command=accepted["command"],
        observation={"weather": "changed", "witness": "other"},
    )
    repeated = worker_result(
        machine,
        "decide",
        state=vectors["initial_state"],
        effect={"status": "none"},
        command=accepted["command"],
        observation=accepted["observation"],
    )
    assert repeated == first
    assert first["event"] != second["event"]
    assert first["event"]["payload"]["observed"] == accepted["observation"]


def test_narrative_wedding_weekend_transition_is_atomic_and_replays(ik_bin: Path, store: Path) -> None:
    vectors = corpus()
    case = next(c for c in vectors["cases"] if c["name"] == "narrative_wedding_weekend_kiss_noah")
    kernel = reducer_kernel(ik_bin, store)
    machine = kernel.reducer(ENTRYPOINT, python=sys.executable)
    aggregate = machine.create("wedding-weekend-01")
    updated = aggregate.execute(case["command"], case["observation"])
    expected = case["apply"]["state"]
    assert updated.state == expected
    assert updated.state["facts"]["kiss_noah"] is True
    assert updated.state["relationship"] == {"appraisal": 4, "stage": "chosen", "trust": 3}
    assert updated.state["route"] == "noah"
    assert updated.state["clock"] == 2
    assert updated.state["obligation"] == "tell_noah_the_truth"
    replay = updated.replay()
    verify = updated.kernel_verify()
    assert replay["aggregates"][0]["state"] == expected
    assert replay["aggregates"][0]["generation"] == updated.generation
    assert verify["verified"] is True
    assert verify["journal_ok"] is True
    assert verify["projection_ok"] is True


def test_effect_intent_and_outcome_commit_across_replay(ik_bin: Path, store: Path) -> None:
    kernel = reducer_kernel(ik_bin, store)
    machine = kernel.reducer(ENTRYPOINT, python=sys.executable)
    aggregate = machine.create("parity-aggregate")
    planned = aggregate.execute({"name": "plan_effect", "args": {"enabled": True}})
    assert planned.state["effect_state"] == "planned"
    assert planned.effect.status == "pending"
    assert planned.effect.intent is not None
    assert planned.effect.intent.effect_id == "parity-effect-1"
    assert planned.effect.intent.aggregate_id == "parity-aggregate"
    assert planned.effect.intent.generation == planned.generation == 1

    recorded = planned.execute({"name": "resolve_effect", "args": {}})
    assert recorded.state["effect_state"] == "resolved"
    assert recorded.generation == 2
    assert recorded.effect.status == "recorded"
    assert recorded.effect.outcome_digest == "c" * 64
    replay = recorded.replay()
    assert replay["aggregates"][0]["state"] == recorded.state
    assert recorded.show().effect.status == "recorded"
    assert recorded.kernel_verify()["verified"] is True


def test_terminal_transition_commits_replays_and_blocks_mutation(ik_bin: Path, store: Path) -> None:
    kernel = reducer_kernel(ik_bin, store)
    machine = kernel.reducer(ENTRYPOINT, python=sys.executable)
    finished = machine.create("terminal").execute({"name": "finish", "args": {}})
    assert finished.terminal is True
    assert finished.state["finished"] is True
    before = aggregate_snapshot(finished)
    with pytest.raises(KernelError) as raised:
        finished.execute({"name": "accept", "args": {"amount": 1}})
    assert raised.value.code == "IK_ILLEGAL_STATE"
    assert raised.value.rule == "terminal"
    assert aggregate_snapshot(finished.show()) == before


def test_identity_mismatch_is_rejected_by_worker() -> None:
    wrong = dict(ParityReducer.identity.to_wire())
    wrong["reducer_name"] = "not-native-parity"
    with pytest.raises(KernelError) as raised:
        transport.invoke_worker(
            ENTRYPOINT,
            wrong,
            op="decide",
            state=corpus()["initial_state"],
            effect={"status": "none"},
            command={"name": "noop", "args": {}},
            observation=None,
            python=sys.executable,
            worker_timeout=3.0,
        )
    assert raised.value.code == "IK_MACHINE_MISMATCH"
    assert raised.value.rule == "reducer.identity"


def test_binding_hash_covers_entrypoint_and_initial_state(ik_bin: Path, store: Path) -> None:
    kernel = reducer_kernel(ik_bin, store)
    base = kernel.reducer(ENTRYPOINT, python=sys.executable)
    alias = kernel.reducer("reducer_fixtures:ParityReducerAlias", python=sys.executable)
    changed_initial = kernel.reducer(
        "reducer_fixtures:AlternateInitialStateReducer", python=sys.executable
    )
    hashes = {
        base.identity.to_wire()["spec_hash"],
        alias.identity.to_wire()["spec_hash"],
        changed_initial.identity.to_wire()["spec_hash"],
    }
    assert len(hashes) == 3
    assert base.create("binding-base").generation == 0
    assert alias.create("binding-alias").generation == 0
    assert changed_initial.create("binding-initial").state["score"] == 1



def test_large_initial_state_uses_request_body_not_process_argv(
    ik_bin: Path, store: Path
) -> None:
    machine = reducer_kernel(ik_bin, store).reducer(
        "reducer_fixtures:LargeInitialStateReducer", python=sys.executable
    )
    created = machine.create("large-initial-state")
    assert len(created.state["blob"]) == 300 * 1024


def test_out_of_range_initial_state_is_rejected_during_discovery(
    ik_bin: Path, store: Path
) -> None:
    with pytest.raises(KernelError) as raised:
        reducer_kernel(ik_bin, store).reducer(
            "reducer_fixtures:OutOfRangeInitialStateReducer",
            python=sys.executable,
        )
    assert raised.value.code == "IK_INTERNAL"
    assert raised.value.rule == "reducer.exception"


def test_out_of_range_command_is_rejected_before_worker_or_mutation(
    ik_bin: Path, store: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    marker = tmp_path / "integer-reducer-invoked"
    monkeypatch.setenv("IK_REDUCER_INVOCATION_FILE", str(marker))
    machine = reducer_kernel(ik_bin, store).reducer(
        "reducer_fixtures:InvocationMarkerReducer", python=sys.executable
    )
    aggregate = machine.create("integer-command")
    before = aggregate_snapshot(aggregate)
    with pytest.raises(ValueError, match="shared JSON range"):
        aggregate.execute({"name": "accept", "args": {"amount": 1 << 64}})
    assert not marker.exists()
    assert aggregate_snapshot(aggregate.show()) == before


def test_raw_external_request_rejects_out_of_range_integer_before_worker(
    ik_bin: Path, store: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    marker = tmp_path / "raw-integer-reducer-invoked"
    monkeypatch.setenv("IK_REDUCER_INVOCATION_FILE", str(marker))
    kernel = reducer_kernel(ik_bin, store)
    machine = kernel.reducer(
        "reducer_fixtures:InvocationMarkerReducer", python=sys.executable
    )
    aggregate = machine.create("raw-integer-command")
    assert machine.worker_spec is not None
    request = kernel._envelope(
        "command",
        aggregate_id=aggregate.id,
        machine=machine.identity,
        capability=aggregate._capability,
        command={"name": "accept", "args": {"amount": 1 << 64}},
    )
    request["reducer_worker"] = dict(machine.worker_spec)
    with pytest.raises(KernelError) as raised:
        transport.invoke(
            kernel.binary,
            kernel.store,
            None,
            request,
            kernel.timeout,
        )
    assert raised.value.code == "IK_PROTOCOL_INVALID"
    assert raised.value.rule == "request.number"
    assert not marker.exists()


def test_out_of_range_apply_state_does_not_mutate(
    ik_bin: Path, store: Path
) -> None:
    machine = reducer_kernel(ik_bin, store).reducer(
        "reducer_fixtures:OutOfRangeApplyReducer", python=sys.executable
    )
    aggregate = machine.create("integer-apply")
    before = aggregate_snapshot(aggregate)
    with pytest.raises(KernelError) as raised:
        aggregate.execute({"name": "accept", "args": {"amount": 1}})
    assert raised.value.code == "IK_INTERNAL"
    assert raised.value.rule == "reducer.exception"
    assert aggregate_snapshot(aggregate.show()) == before

def test_tampered_initial_state_binding_refuses_before_create(ik_bin: Path, store: Path) -> None:
    kernel = reducer_kernel(ik_bin, store)
    machine = kernel.reducer(ENTRYPOINT, python=sys.executable)
    assert machine.worker_spec is not None
    expected_digest = machine.worker_spec["initial_state_digest"]
    machine.worker_spec["initial_state_digest"] = "0" * 64
    with pytest.raises(KernelError) as raised:
        machine.create("tampered-binding")
    assert raised.value.code == "IK_MACHINE_HASH_MISMATCH"
    assert raised.value.rule == "reducer.initial_state_digest"
    machine.worker_spec["initial_state_digest"] = expected_digest
    assert machine.create("tampered-binding").generation == 0

def aggregate_snapshot(aggregate: Any) -> tuple[Any, int, Any, Any]:
    receipt = aggregate.receipt.raw if aggregate.receipt is not None else None
    return aggregate.state, aggregate.generation, receipt, aggregate.replay()



@pytest.mark.parametrize(
    "entrypoint",
    [
        "reducer_fixtures:RaisingReducer",
        "reducer_fixtures:RaisingApplyReducer",
    ],
)
def test_exception_maps_to_internal_diagnostic_without_mutation(
    ik_bin: Path, store: Path, entrypoint: str
) -> None:
    kernel = reducer_kernel(ik_bin, store)
    machine = kernel.reducer(entrypoint, python=sys.executable)
    aggregate = machine.create("raising")
    before = aggregate_snapshot(aggregate)
    with pytest.raises(KernelError) as raised:
        aggregate.execute({"name": "accept", "args": {"amount": 1}}, None)
    assert raised.value.code == "IK_INTERNAL"
    assert raised.value.rule == "reducer.exception"
    assert aggregate_snapshot(aggregate.show()) == before

def test_malformed_return_is_internal_and_does_not_mutate(ik_bin: Path, store: Path) -> None:
    kernel = reducer_kernel(ik_bin, store)
    machine = kernel.reducer("reducer_fixtures:MalformedReducer", python=sys.executable)
    aggregate = machine.create("malformed")
    before = aggregate_snapshot(aggregate)
    with pytest.raises(KernelError) as raised:
        aggregate.execute({"name": "anything", "args": {}}, None)
    assert raised.value.code == "IK_INTERNAL"
    assert raised.value.rule == "reducer.exception"
    after = aggregate.show()
    assert aggregate_snapshot(after) == before


def test_reducer_inputs_are_recursively_frozen_without_mutation(ik_bin: Path, store: Path) -> None:
    kernel = reducer_kernel(ik_bin, store)
    machine = kernel.reducer("reducer_fixtures:MutationReducer", python=sys.executable)
    aggregate = machine.create("mutation")
    before = aggregate_snapshot(aggregate)
    with pytest.raises(KernelError) as raised:
        aggregate.execute({"name": "mutate", "args": {}}, None)
    assert raised.value.code == "IK_INTERNAL"
    assert raised.value.rule == "reducer.exception"
    assert aggregate_snapshot(aggregate.show()) == before


def test_reducer_output_is_bounded_without_mutation(ik_bin: Path, store: Path) -> None:
    kernel = reducer_kernel(ik_bin, store)
    machine = kernel.reducer("reducer_fixtures:NoisyReducer", python=sys.executable)
    aggregate = machine.create("noisy")
    before = aggregate_snapshot(aggregate)
    with pytest.raises(KernelError) as raised:
        aggregate.execute({"name": "noise", "args": {}}, None)
    assert raised.value.code == "IK_INTERNAL"
    assert raised.value.rule == "reducer.output_limit"
    assert aggregate_snapshot(aggregate.show()) == before


def test_direct_worker_output_is_bounded() -> None:
    with pytest.raises(KernelError) as raised:
        transport.invoke_worker(
            "reducer_fixtures:NoisyReducer",
            ParityReducer.identity.to_wire(),
            op="decide",
            state=corpus()["initial_state"],
            effect={"status": "none"},
            command={"name": "noise", "args": {}},
            observation=None,
            python=sys.executable,
            worker_timeout=3.0,
        )
    assert raised.value.code == "IK_TRANSPORT"
    assert "capture limit" in raised.value.message

def test_timeout_kills_worker(ik_bin: Path, store: Path) -> None:
    kernel = reducer_kernel(ik_bin, store)
    machine = kernel.reducer("reducer_fixtures:SleepingReducer", python=sys.executable, worker_timeout=0.5)
    aggregate = machine.create("timeout")
    with pytest.raises(KernelError) as raised:
        aggregate.execute({"name": "sleep", "args": {}}, None)
    assert raised.value.code == "IK_INTERNAL"
    assert raised.value.rule == "reducer.timeout"
    assert "timeout" in raised.value.message


@pytest.mark.skipif(os.name != "posix", reason="process-group reaping is POSIX-specific")
def test_timeout_reaps_worker_descendant(ik_bin: Path, store: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    marker = tmp_path / "child.pid"
    monkeypatch.setenv("IK_DESCENDANT_PID_FILE", str(marker))
    kernel = reducer_kernel(ik_bin, store)
    machine = kernel.reducer("reducer_fixtures:SpawnSleepingReducer", python=sys.executable, worker_timeout=0.5)
    aggregate = machine.create("descendant")
    with pytest.raises(KernelError) as raised:
        aggregate.execute({"name": "spawn", "args": {}}, None)
    assert raised.value.code == "IK_INTERNAL"
    assert raised.value.rule == "reducer.timeout"
    deadline = time.monotonic() + 2
    while not marker.exists() and time.monotonic() < deadline:
        time.sleep(0.01)
    assert marker.exists()
    pid = int(marker.read_text(encoding="utf-8"))
    with pytest.raises(ProcessLookupError):
        os.kill(pid, 0)


@pytest.mark.skipif(os.name != "posix", reason="process-tree reaping is POSIX-specific")
def test_timeout_reaps_detached_worker_descendant(
    ik_bin: Path, store: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    marker = tmp_path / "detached-timeout-child.pid"
    monkeypatch.setenv("IK_DESCENDANT_PID_FILE", str(marker))
    machine = reducer_kernel(ik_bin, store).reducer(
        "reducer_fixtures:DetachedSpawnSleepingReducer",
        python=sys.executable,
        worker_timeout=0.5,
    )
    aggregate = machine.create("detached-timeout")
    with pytest.raises(KernelError) as raised:
        aggregate.execute({"name": "spawn", "args": {}})
    assert raised.value.code == "IK_INTERNAL"
    assert raised.value.rule == "reducer.timeout"
    assert marker.exists()
    assert_process_gone(int(marker.read_text(encoding="utf-8")))


@pytest.mark.skipif(os.name != "posix", reason="process-tree reaping is POSIX-specific")
def test_outer_timeout_reaps_nested_worker_group_without_mutation(
    ik_bin: Path, store: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    marker = tmp_path / "outer-timeout-child.pid"
    monkeypatch.setenv("IK_DESCENDANT_PID_FILE", str(marker))
    # Outer timeout must fire while apply() is blocked AFTER spawning its
    # descendant. Margins leave headroom for two fresh worker startups on a
    # loaded host; the tested invariant (nested group reaped, zero mutation)
    # does not depend on tight timing.
    kernel = reducer_kernel(ik_bin, store, timeout=3.0)
    machine = kernel.reducer(
        "reducer_fixtures:SlowTwoPhaseSpawnReducer",
        python=sys.executable,
        worker_timeout=2.5,
    )
    aggregate = machine.create("outer-timeout")
    before = aggregate_snapshot(aggregate)
    with pytest.raises(KernelError) as raised:
        aggregate.execute({"name": "accept", "args": {"amount": 1}})
    assert raised.value.code == "IK_TRANSPORT"
    assert "ik command timed out" in raised.value.message
    assert marker.exists()
    pid = int(marker.read_text(encoding="utf-8"))
    deadline = time.monotonic() + 2
    while time.monotonic() < deadline:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            break
        time.sleep(0.01)
    else:
        pytest.fail(f"nested reducer descendant {pid} survived outer timeout")
    assert aggregate_snapshot(aggregate.show()) == before


@pytest.mark.skipif(os.name != "posix", reason="process-group reaping is POSIX-specific")
def test_successful_discovery_reaps_worker_descendant(
    ik_bin: Path, store: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    marker = tmp_path / "describe-child.pid"
    monkeypatch.setenv("IK_DESCENDANT_PID_FILE", str(marker))
    kernel = reducer_kernel(ik_bin, store)
    machine = kernel.reducer(
        "reducer_fixtures:SpawnOnDescribeReducer", python=sys.executable
    )
    assert marker.exists()
    pid = int(marker.read_text(encoding="utf-8"))
    deadline = time.monotonic() + 2
    while time.monotonic() < deadline:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            break
        time.sleep(0.01)
    else:
        pytest.fail(f"worker descendant {pid} survived discovery")
    marker.unlink()
    assert machine.create("no-describe-on-create").generation == 0
    assert not marker.exists()


@pytest.mark.skipif(os.name != "posix", reason="process-tree reaping is POSIX-specific")
def test_successful_discovery_reaps_detached_descendant(
    ik_bin: Path, store: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    marker = tmp_path / "detached-describe-child.pid"
    monkeypatch.setenv("IK_DESCENDANT_PID_FILE", str(marker))
    machine = reducer_kernel(ik_bin, store).reducer(
        "reducer_fixtures:DetachedOnDescribeReducer", python=sys.executable
    )
    assert marker.exists()
    assert_process_gone(int(marker.read_text(encoding="utf-8")))
    assert machine.create("detached-describe").generation == 0


@pytest.mark.skipif(os.name != "posix", reason="process-tree reaping is POSIX-specific")
def test_successful_command_reaps_detached_descendant(
    ik_bin: Path, store: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    marker = tmp_path / "detached-command-child.pid"
    monkeypatch.setenv("IK_DESCENDANT_PID_FILE", str(marker))
    aggregate = reducer_kernel(ik_bin, store).reducer(
        "reducer_fixtures:DetachedOnDecideReducer", python=sys.executable
    ).create("detached-command")
    result = aggregate.execute({"name": "noop", "args": {}})
    assert result.generation == 0
    assert marker.exists()
    assert_process_gone(int(marker.read_text(encoding="utf-8")))


def test_stale_copied_handle_is_rejected_before_reducer(
    ik_bin: Path, store: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    marker = tmp_path / "reducer-invoked"
    monkeypatch.setenv("IK_REDUCER_INVOCATION_FILE", str(marker))
    kernel = reducer_kernel(ik_bin, store)
    machine = kernel.reducer(
        "reducer_fixtures:InvocationMarkerReducer", python=sys.executable
    )
    original = machine.create("stale")
    stale = original
    current = original.execute({"name": "accept", "args": {"amount": 1}}, None)
    assert current.state["score"] == 1
    before = aggregate_snapshot(current)
    marker.unlink()
    with pytest.raises(KernelError) as raised:
        stale.execute({"name": "reject", "args": {}}, None)
    assert raised.value.code == "IK_CAPABILITY_STALE"
    assert raised.value.rule == "capability.generation"
    assert not marker.exists()
    assert aggregate_snapshot(current.show()) == before


def test_stale_exact_outcome_retry_is_rejected_before_reducer(
    ik_bin: Path, store: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    marker = tmp_path / "duplicate-reducer-invoked"
    monkeypatch.setenv("IK_REDUCER_INVOCATION_FILE", str(marker))
    machine = reducer_kernel(ik_bin, store).reducer(
        "reducer_fixtures:InvocationMarkerReducer", python=sys.executable
    )
    planned = machine.create("parity-aggregate").execute(
        {"name": "plan_effect", "args": {"enabled": True}}
    )
    recorded = planned.execute({"name": "resolve_effect", "args": {}})
    assert recorded.effect.status == "recorded"
    before = aggregate_snapshot(recorded)
    marker.unlink()

    with pytest.raises(KernelError) as raised:
        planned.execute({"name": "resolve_effect", "args": {}})

    assert raised.value.code == "IK_CAPABILITY_STALE"
    assert raised.value.rule == "capability.generation"
    assert not marker.exists()
    assert aggregate_snapshot(recorded.show()) == before


def test_kernel_law_refuses_defective_python_post_state(ik_bin: Path, store: Path) -> None:
    kernel = reducer_kernel(ik_bin, store)
    machine = kernel.reducer(
        ENTRYPOINT,
        python=sys.executable,
        kernel_laws=[{"name": "score_frozen", "assert": "score == 0"}],
    )
    aggregate = machine.create("law-agg")
    assert aggregate.state["score"] == 0
    with pytest.raises(KernelError) as raised:
        aggregate.execute({"name": "accept", "args": {"amount": 1}}, None)
    assert raised.value.code == "IK_INVARIANT_FAILED"
    assert aggregate.show().state["score"] == 0


def test_false_initial_kernel_law_refuses_creation_with_no_aggregate(
    ik_bin: Path, store: Path
) -> None:
    kernel = reducer_kernel(ik_bin, store)
    machine = kernel.reducer(
        ENTRYPOINT,
        python=sys.executable,
        kernel_laws=[{"name": "score_starts_at_one", "assert": "score == 1"}],
    )
    with pytest.raises(KernelError) as raised:
        machine.create("bad-genesis")
    assert raised.value.code == "IK_INVARIANT_FAILED"
    with pytest.raises(KernelError):
        machine.show("bad-genesis")


def test_code_source_mismatch_refuses_before_create(
    ik_bin: Path, store: Path, tmp_path: Path
) -> None:
    source = tmp_path / "reducer.py"
    source.write_text("print('not the declared digest')\n", encoding="utf-8")
    kernel = reducer_kernel(ik_bin, store)
    machine = kernel.reducer(
        ENTRYPOINT,
        python=sys.executable,
        code_source=str(source),
    )
    with pytest.raises(KernelError) as raised:
        machine.create("digest-agg")
    assert raised.value.code == "IK_MACHINE_MISMATCH"


def test_kernel_laws_change_machine_identity(ik_bin: Path, store: Path) -> None:
    kernel = reducer_kernel(ik_bin, store)
    laws = [{"name": "score_frozen", "assert": "score == 0"}]
    lawful = kernel.reducer(ENTRYPOINT, python=sys.executable, kernel_laws=laws)
    plain = kernel.reducer(ENTRYPOINT, python=sys.executable)
    assert lawful.identity.to_wire()["spec_hash"] != plain.identity.to_wire()["spec_hash"]
    aggregate = lawful.create("law-identity")
    from invariant_kernel.client import Aggregate

    crossed = Aggregate.from_result(plain, aggregate.raw, created=False)
    with pytest.raises(KernelError) as raised:
        crossed.execute({"name": "accept", "args": {"amount": 1}}, None)
    assert raised.value.code == "IK_MACHINE_MISMATCH"
