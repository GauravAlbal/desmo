"""Recoverable invocations: persist the description before sending, reconcile after a crash."""

from __future__ import annotations

from pathlib import Path

import pytest

from conftest import VA
from invariant_kernel import (
    Aggregate,
    HistoricalResult,
    Invocation,
    Kernel,
)

START = {"name": "start"}


def machine_for(ik_bin: Path, store: Path):
    return Kernel(store, binary=str(ik_bin)).machine(VA)


def test_a_crash_after_commit_is_reconciled_from_the_persisted_description(
    ik_bin: Path, store: Path
) -> None:
    first = machine_for(ik_bin, store)
    created = first.create("job-1")
    invocation = created.prepare(START)
    persisted = invocation.to_json()  # the host keeps this in its durable job record
    started = first.dispatch(invocation)  # commits; the process "dies" before using the reply
    assert isinstance(started, Aggregate)
    moved = started.command("cancel")  # another writer moves the aggregate on
    assert moved.generation == 2

    fresh = machine_for(ik_bin, store)  # a new process holds only the persisted description
    recovered = Invocation.from_json(persisted)
    lookup = fresh.reconcile(recovered)
    assert lookup.outcome == "committed_exact"
    assert lookup.history is not None
    assert lookup.history.generation == 1  # the original coordinate, not the current one
    assert lookup.history.current_generation == 2


def test_a_prepared_but_unsent_invocation_is_not_recorded_and_can_still_be_dispatched(
    ik_bin: Path, store: Path
) -> None:
    machine = machine_for(ik_bin, store)
    created = machine.create("job-2")
    invocation = created.prepare(START)
    assert machine.reconcile(invocation).outcome == "not_recorded"
    sent = machine.dispatch(Invocation.from_json(invocation.to_json()))
    assert isinstance(sent, Aggregate)
    assert machine.reconcile(invocation).outcome == "committed_exact"


def test_dispatching_the_same_invocation_again_is_history_not_a_second_commit(
    ik_bin: Path, store: Path
) -> None:
    machine = machine_for(ik_bin, store)
    created = machine.create("job-3")
    invocation = created.prepare(START)
    assert isinstance(machine.dispatch(invocation), Aggregate)
    repeat = machine.dispatch(Invocation.from_json(invocation.to_json()))
    assert isinstance(repeat, HistoricalResult)
    assert repeat.operation_id == invocation.operation_id
    assert machine.show("job-3").generation == 1


def test_the_operation_id_does_not_change_when_a_newer_handle_is_read(
    ik_bin: Path, store: Path
) -> None:
    machine = machine_for(ik_bin, store)
    created = machine.create("job-4")
    invocation = created.prepare(START)
    machine.dispatch(invocation)
    refreshed = machine.show("job-4")
    again = refreshed.prepare(START)
    assert again.operation_id != invocation.operation_id  # a new invocation is a new id
    assert invocation.operation_id == Invocation.from_json(invocation.to_json()).operation_id
    # Reconciling the original invocation with the refreshed capability is a different request.
    lookup = machine.lookup_operation(
        "job-4", refreshed.capability, invocation.operation_id, command=START
    )
    assert lookup.outcome == "conflict"


def test_a_transaction_invocation_round_trips_and_reconciles(ik_bin: Path, store: Path) -> None:
    machine = machine_for(ik_bin, store)
    created = machine.create("job-5")
    invocation = created.prepare_transaction([START])
    wire = invocation.to_wire()
    assert wire["kind"] == "transaction"
    assert machine.reconcile(Invocation.from_wire(wire)).outcome == "not_recorded"
    assert isinstance(machine.dispatch(invocation), Aggregate)
    assert machine.reconcile(Invocation.from_wire(wire)).outcome == "committed_exact"


def test_an_invocation_for_another_machine_identity_is_refused(ik_bin: Path, store: Path) -> None:
    machine = machine_for(ik_bin, store)
    created = machine.create("job-6")
    wire = created.prepare(START).to_wire()
    wire["machine"]["spec_hash"] = "0" * 64
    other = Invocation.from_wire(wire)
    with pytest.raises(ValueError):
        machine.dispatch(other)
    with pytest.raises(ValueError):
        machine.reconcile(other)


def test_the_wire_form_rejects_unknown_versions_fields_and_empty_ids() -> None:
    with pytest.raises(ValueError):
        Invocation.from_wire({"invocation_version": 2})
    good = {
        "invocation_version": 1,
        "kind": "command",
        "machine": {"machine_id": "m", "version": 1, "spec_hash": "h"},
        "aggregate_id": "a",
        "operation_id": "op",
        "capability": {"generation": 0, "nonce": "n"},
        "command": {"name": "start"},
    }
    assert Invocation.from_wire(good).operation_id == "op"
    with pytest.raises(ValueError):
        Invocation.from_wire({**good, "surprise": 1})
    with pytest.raises(ValueError):
        Invocation.from_wire({**good, "operation_id": ""})
    with pytest.raises(ValueError):
        Invocation.from_wire({**good, "kind": "transaction"})


def test_a_pinned_fixture_serializes_to_the_same_bytes_in_every_client() -> None:
    from invariant_kernel import Capability, MachineIdentity

    invocation = Invocation(
        kind="command",
        machine=MachineIdentity("m", 1, "h"),
        aggregate_id="a",
        operation_id="op",
        capability=Capability(0, "n"),
        command={"name": "start"},
    )
    assert invocation.to_json() == (
        '{"aggregate_id":"a","capability":{"generation":0,"nonce":"n"},"command":{"name":"start"},'
        '"invocation_version":1,"kind":"command","machine":{"machine_id":"m","spec_hash":"h","version":1},'
        '"operation_id":"op"}'
    )


def test_the_prepared_request_is_a_snapshot_of_what_was_persisted(ik_bin: Path, store: Path) -> None:
    machine = machine_for(ik_bin, store)
    started = machine.dispatch(machine.create("job-7").prepare(START))
    assert isinstance(started, Aggregate)
    args = {"artifact_digest": "AMBER", "baseline_digest": "b"}
    observation = {"note": {"tags": ["x"]}}
    invocation = started.prepare({"name": "submit", "args": args}, observation=observation)
    persisted = invocation.to_json()

    args["artifact_digest"] = "BLUE"  # ordinary reuse of a caller-owned dict
    observation["note"]["tags"].append("y")
    exported = invocation.to_wire()
    exported["command"]["args"]["artifact_digest"] = "RED"
    invocation.command["args"]["artifact_digest"] = "GREEN"  # a returned copy
    assert invocation.to_json() == persisted
    with pytest.raises(AttributeError):
        invocation.operation_id = "other"  # type: ignore[misc]

    sent = machine.dispatch(invocation)
    assert isinstance(sent, Aggregate)
    moved = sent.command("cancel")  # another writer advances the aggregate
    assert moved.generation > sent.generation
    lookup = machine_for(ik_bin, store).reconcile(Invocation.from_json(persisted))
    assert lookup.outcome == "committed_exact"


def test_transaction_steps_are_snapshotted_and_unserializable_input_is_refused(
    ik_bin: Path, store: Path
) -> None:
    machine = machine_for(ik_bin, store)
    created = machine.create("job-8")
    steps = [{"name": "start"}, {"name": "cancel"}]
    invocation = created.prepare_transaction(steps)
    persisted = invocation.to_json()
    steps[1]["name"] = "start"
    steps.append({"name": "start"})
    assert invocation.to_json() == persisted
    assert isinstance(machine.dispatch(invocation), Aggregate)
    assert machine_for(ik_bin, store).reconcile(Invocation.from_json(persisted)).outcome == "committed_exact"
    with pytest.raises(TypeError):
        created.prepare({"name": "start", "args": {"x": object()}})


CORPUS = (
    r'{"aggregate_id":"a","capability":{"generation":0,"nonce":"n"},"command":{"args":{"label":"\u00e9<&\u2603",'
    r'"n":9007199254740993,"nested":{"a":[1,{"b":"x"}]}},"name":"start"},"invocation_version":1,"kind":"command",'
    r'"machine":{"machine_id":"m","spec_hash":"h","version":1},"operation_id":"op"}'
)


def test_the_shared_corpus_restores_with_exact_values_and_survives_a_second_round_trip() -> None:
    first = Invocation.from_json(CORPUS)
    args = first.command["args"]
    assert args["label"] == "\u00e9<&\u2603"
    assert args["n"] == 9007199254740993
    again = Invocation.from_json(first.to_json())
    assert again.to_wire() == first.to_wire()
    assert "9007199254740993" in again.to_json()
