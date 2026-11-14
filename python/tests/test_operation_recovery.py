"""Operation attribution and recovery through the Python client (IK-04)."""

from __future__ import annotations

import stat
import subprocess
from pathlib import Path

import pytest

from conftest import VA
from invariant_kernel import (
    Aggregate,
    HistoricalResult,
    IllegalStateError,
    Kernel,
    OperationConflictError,
    PreDispatchError,
    StaleCapabilityError,
    TransportError,
)

START = {"name": "start"}
CANCEL = {"name": "cancel"}


def make_kernel(ik_bin: Path, store: Path, **kwargs) -> Kernel:
    return Kernel(store, binary=str(ik_bin), **kwargs)


def test_dropped_reply_then_other_writer_is_recovered_by_a_fresh_client(
    ik_bin: Path, store: Path
) -> None:
    first = make_kernel(ik_bin, store).machine(VA)
    created = first.create("attempt-1")
    persisted_capability = created.capability.to_wire()
    operation_id = first.kernel.new_operation_id()
    # The submitting process commits, then "dies" before it can read the reply.
    started = created.execute(START, operation_id=operation_id)
    assert isinstance(started, Aggregate)
    # A different writer advances the aggregate past the original capability.
    cancelled = started.command("cancel")
    assert isinstance(cancelled, Aggregate)
    assert cancelled.generation == 2

    # A fresh client in a "new process" holds only what the caller persisted.
    fresh = make_kernel(ik_bin, store).machine(VA)
    capability = type(created.capability).from_wire(persisted_capability)
    lookup = fresh.lookup_operation("attempt-1", capability, operation_id, command=START)
    assert lookup.outcome == "committed_exact"
    assert lookup.history is not None
    assert lookup.history.first_sequence == 1
    assert lookup.history.current_generation == 2
    # The recorded commit is history; the aggregate has moved on.
    assert lookup.history.generation == 1


def test_exact_repeat_is_history_without_authority_and_does_not_mutate(
    ik_bin: Path, store: Path
) -> None:
    machine = make_kernel(ik_bin, store).machine(VA)
    created = machine.create("attempt-1")
    operation_id = "recovery-repeat-1"
    started = created.execute(START, operation_id=operation_id)
    assert isinstance(started, Aggregate)
    moved = started.command("cancel")
    assert isinstance(moved, Aggregate)

    repeat = created.execute(START, operation_id=operation_id)
    assert isinstance(repeat, HistoricalResult)
    assert not isinstance(repeat, Aggregate)
    assert not hasattr(repeat, "capability")
    assert repeat.operation_id == operation_id
    assert repeat.current_generation == 2
    after = machine.show("attempt-1")
    assert after.generation == 2
    assert after.state == "cancelled"


def test_a_stale_handle_without_an_operation_match_is_still_refused(
    ik_bin: Path, store: Path
) -> None:
    machine = make_kernel(ik_bin, store).machine(VA)
    created = machine.create("attempt-1")
    created.execute(START, operation_id="first-op")
    with pytest.raises(StaleCapabilityError) as refused:
        created.execute(CANCEL, operation_id="different-op")
    assert refused.value.submission == "refused"
    assert refused.value.operation_id == "different-op"


def test_same_operation_id_with_changed_semantics_is_a_conflict(
    ik_bin: Path, store: Path
) -> None:
    machine = make_kernel(ik_bin, store).machine(VA)
    created = machine.create("attempt-1")
    created.execute(START, operation_id="reused")
    with pytest.raises(OperationConflictError) as conflict:
        created.execute(CANCEL, operation_id="reused")
    assert conflict.value.code == "IK_OPERATION_CONFLICT"
    assert conflict.value.operation_id == "reused"
    lookup = machine.lookup_operation(
        "attempt-1", created.capability, "reused", command=CANCEL
    )
    assert lookup.outcome == "conflict"


def test_lookup_of_an_unsent_operation_is_not_recorded(ik_bin: Path, store: Path) -> None:
    machine = make_kernel(ik_bin, store).machine(VA)
    created = machine.create("attempt-1")
    lookup = machine.lookup_operation(
        "attempt-1", created.capability, "never-sent", command=START
    )
    assert lookup.outcome == "not_recorded"
    assert lookup.history is None
    assert machine.show("attempt-1").generation == 0


def test_generated_operation_id_is_exposed_on_the_error_and_matches_the_wire(
    ik_bin: Path, store: Path
) -> None:
    machine = make_kernel(ik_bin, store).machine(VA)
    created = machine.create("attempt-1")
    with pytest.raises(IllegalStateError) as refused:
        created.execute({"name": "submit", "args": {"artifact_digest": "a", "baseline_digest": "b"}})
    generated = refused.value.operation_id
    assert isinstance(generated, str) and generated.startswith("py-")
    lookup = machine.lookup_operation(
        "attempt-1",
        created.capability,
        generated,
        command={"name": "submit", "args": {"artifact_digest": "a", "baseline_digest": "b"}},
    )
    # A refusal commits nothing and records nothing under the id.
    assert lookup.outcome == "not_recorded"


def test_transaction_exact_repeat_is_history(ik_bin: Path, store: Path) -> None:
    machine = make_kernel(ik_bin, store).machine(VA)
    created = machine.create("attempt-1")
    first = created.transaction([START, CANCEL], operation_id="batch-1")
    assert isinstance(first, Aggregate)
    repeat = created.transaction([START, CANCEL], operation_id="batch-1")
    assert isinstance(repeat, HistoricalResult)
    assert (repeat.first_sequence, repeat.last_sequence) == (1, 2)
    assert repeat.kind == "transaction"


def test_missing_binary_is_pre_dispatch_and_keeps_the_operation_id(
    ik_bin: Path, store: Path, tmp_path: Path
) -> None:
    kernel = make_kernel(ik_bin, store)
    machine = kernel.machine(VA)
    created = machine.create("attempt-1")
    kernel.binary = str(tmp_path / "no-such-ik")
    with pytest.raises(PreDispatchError) as missing:
        created.execute(START, operation_id="persisted-before-dispatch")
    assert missing.value.submission == "not-sent"
    assert missing.value.operation_id == "persisted-before-dispatch"
    # Provably unsent: the store is untouched, so the id is not recorded.
    reopened = make_kernel(ik_bin, store).machine(VA)
    lookup = reopened.lookup_operation(
        "attempt-1", created.capability, "persisted-before-dispatch", command=START
    )
    assert lookup.outcome == "not_recorded"


def test_timeout_after_dispatch_is_submission_unknown_and_never_retried(
    ik_bin: Path, store: Path, tmp_path: Path
) -> None:
    dispatches = tmp_path / "dispatches.log"
    stub = tmp_path / "slow-ik"
    real_version = subprocess.run(
        [str(ik_bin), "--version"], capture_output=True, text=True, check=True
    ).stdout.strip()
    stub.write_text(
        "#!/bin/sh\n"
        f'if [ "$1" = "--version" ]; then echo "{real_version}"; exit 0; fi\n'
        f'echo dispatched >> "{dispatches}"\n'
        "sleep 5\n"
    )
    stub.chmod(stub.stat().st_mode | stat.S_IEXEC)
    machine = make_kernel(ik_bin, store).machine(VA)
    created = machine.create("attempt-1")
    machine.kernel.binary = str(stub)
    machine.kernel.timeout = 0.5
    with pytest.raises(TransportError) as unknown:
        created.execute(START, operation_id="maybe-committed")
    assert not isinstance(unknown.value, PreDispatchError)
    assert unknown.value.submission == "submission-unknown"
    assert unknown.value.operation_id == "maybe-committed"
    assert dispatches.read_text().splitlines() == ["dispatched"]


def test_a_dead_session_keeps_an_in_flight_operation_id_unknown_not_unsent(tmp_path: Path) -> None:
    from invariant_kernel import transport

    stub = tmp_path / "silent-session"
    stub.write_text("#!/bin/sh\ncat > /dev/null\n")
    stub.chmod(stub.stat().st_mode | stat.S_IEXEC)
    session = transport.Session(str(stub), tmp_path / "s.db", None, timeout=0.3)
    request = {"op": "command", "request_id": "r1", "operation_id": "op-in-flight"}
    with pytest.raises(TransportError) as first:
        session.invoke(request)
    assert first.value.submission == "submission-unknown"

    # The same logical operation stays unknown: its first attempt may have committed.
    with pytest.raises(TransportError) as same_id:
        session.invoke({**request, "request_id": "r2"})
    assert not isinstance(same_id.value, PreDispatchError)
    assert same_id.value.submission == "submission-unknown"
    assert "lookup_operation" in same_id.value.message

    # A different operation never reached the dead session: provably not sent.
    with pytest.raises(PreDispatchError) as other:
        session.invoke({"op": "command", "request_id": "r3", "operation_id": "another-op"})
    assert other.value.submission == "not-sent"
