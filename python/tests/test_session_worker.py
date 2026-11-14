"""Persistent reducer worker, ``ik --session`` and ``transaction`` — normative, hermetic tests.

Every test uses a fresh ``tmp_path`` store and the in-tree fixture reducers; worker lifetime is
observed through pid files (no sleeps beyond a bounded liveness poll, no ambient state).

    IK-PROP-017  a long-lived worker serves many commands (one pid) and yields results identical
                 to fresh-worker-per-call dispatch
    IK-PROP-018  worker death cannot partially persist; the worker is replaced; none is orphaned
    IK-PROP-019  paranoid mode fails closed on an impure reducer (default mode does not look)
    IK-PROP-020  ``transaction`` through a Python reducer is all-or-nothing and effect-bounded;
                 the session never retries and survives refusals
"""
from __future__ import annotations

import os
import time
from pathlib import Path
from typing import Any

import pytest

from invariant_kernel import Kernel, transport
from invariant_kernel.errors import KernelError, TransportError

PID_ENTRYPOINT = "reducer_fixtures:PidRecordingReducer"
PARITY_ENTRYPOINT = "reducer_fixtures:ParityReducer"
LEAKY_ENTRYPOINT = "reducer_fixtures:LeakyReducer"


def pids(marker: Path) -> list[int]:
    return [int(line) for line in marker.read_text(encoding="utf-8").split()]


def gone(pid: int) -> bool:
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return True
        time.sleep(0.01)
    return False


def accept_sequence(aggregate: Any, amounts: list[int]) -> Any:
    for amount in amounts:
        aggregate = aggregate.execute({"name": "accept", "args": {"amount": amount}})
    return aggregate


def test_ik_prop_017_worker_persists_and_matches_fresh_worker_results(
    ik_bin: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    amounts = [1, 2, 3, 4]
    outcomes = {}
    pid_sets = {}
    for mode in (False, True):
        marker = tmp_path / f"pids-{mode}.txt"
        monkeypatch.setenv("IK_WORKER_PID_FILE", str(marker))
        with Kernel(tmp_path / f"kernel-{mode}.db", binary=str(ik_bin), session=mode) as kernel:
            machine = kernel.reducer(PID_ENTRYPOINT, python=os.sys.executable)
            final = accept_sequence(machine.create("agg"), amounts)
            outcomes[mode] = (final.state, final.generation, final.replay()["aggregates"])
        pid_sets[mode] = set(pids(marker))
    assert len(pid_sets[True]) == 1, f"session mode must reuse ONE worker, saw {pid_sets[True]}"
    assert len(pid_sets[False]) > 1, "one-shot mode spawns a worker per `ik` invocation"
    assert outcomes[True] == outcomes[False], "persistent worker changed the committed outcome"
    assert outcomes[True][1] == len(amounts)


def test_ik_prop_018_worker_death_cannot_partially_persist_and_is_replaced(
    ik_bin: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    marker = tmp_path / "pids.txt"
    monkeypatch.setenv("IK_WORKER_PID_FILE", str(marker))
    with Kernel(tmp_path / "kernel.db", binary=str(ik_bin), session=True) as kernel:
        machine = kernel.reducer(PID_ENTRYPOINT, python=os.sys.executable)
        aggregate = accept_sequence(machine.create("agg"), [1])
        before = (aggregate.state, aggregate.generation, aggregate.replay())
        first_worker = pids(marker)[-1]

        with pytest.raises(KernelError):
            aggregate.execute({"name": "crash"})
        after = aggregate.show()
        assert (after.state, after.generation, after.replay()) == before, "death mutated state"
        assert gone(first_worker), "the dead worker was never reaped"

        recovered = after.execute({"name": "accept", "args": {"amount": 2}})
        assert recovered.generation == before[1] + 1, "exactly one committed step after recovery"
        assert pids(marker)[-1] != first_worker, "the replacement must be a NEW worker process"


def test_ik_prop_019_paranoid_mode_fails_closed_on_an_impure_reducer(
    ik_bin: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # Default (off): the contract is cooperative; the impure reducer is NOT looked at.
    with Kernel(tmp_path / "default.db", binary=str(ik_bin), session=True) as kernel:
        machine = kernel.reducer(LEAKY_ENTRYPOINT, python=os.sys.executable)
        assert accept_sequence(machine.create("agg"), [1, 1]).generation == 2

    monkeypatch.setenv("IK_REDUCER_PARANOID", "1")
    with Kernel(tmp_path / "paranoid.db", binary=str(ik_bin), session=True) as kernel:
        machine = kernel.reducer(LEAKY_ENTRYPOINT, python=os.sys.executable)
        aggregate = accept_sequence(machine.create("agg"), [1])
        before = (aggregate.state, aggregate.generation, aggregate.replay())
        with pytest.raises(KernelError) as raised:
            aggregate.execute({"name": "accept", "args": {"amount": 1}})
        assert "differs between a long-lived and a fresh worker" in str(raised.value)
        shown = aggregate.show()
        assert (shown.state, shown.generation, shown.replay()) == before, "refusal mutated state"


def test_ik_prop_020_transaction_is_atomic_effect_bounded_and_session_survives(
    ik_bin: Path, tmp_path: Path
) -> None:
    with Kernel(tmp_path / "kernel.db", binary=str(ik_bin), session=True) as kernel:
        machine = kernel.reducer(PARITY_ENTRYPOINT, python=os.sys.executable)
        aggregate = machine.create("agg")

        ok = aggregate.transaction(
            [
                {"name": "accept", "args": {"amount": 2}},
                ({"name": "accept", "args": {"amount": 3}}, {"source": "test"}),
            ]
        )
        assert ok.generation == 2 and len(ok.raw["steps"]) == 2
        sequential = accept_sequence(machine.create("seq"), [2])
        sequential = sequential.execute({"name": "accept", "args": {"amount": 3}}, {"source": "test"})
        assert ok.state == sequential.state, "transaction differs from the same commands one by one"

        snapshot = (ok.state, ok.generation, ok.replay())
        with pytest.raises(KernelError) as rejected:
            ok.transaction([{"name": "accept", "args": {"amount": 5}}, {"name": "reject"}])
        assert "transaction step 1:" in str(rejected.value)
        refreshed = ok.show()
        assert (refreshed.state, refreshed.generation, refreshed.replay()) == snapshot

        with pytest.raises(KernelError) as boundary:
            refreshed.transaction(
                [{"name": "plan_effect", "args": {"enabled": True}}, {"name": "accept", "args": {"amount": 1}}]
            )
        assert boundary.value.code == "IK_TRANSACTION_BOUNDARY"
        assert refreshed.show().generation == snapshot[1]

        # The session survived both refusals and the original authority is still valid.
        again = refreshed.transaction([{"name": "accept", "args": {"amount": 7}}])
        assert again.generation == snapshot[1] + 1


def test_session_never_retries_and_stays_dead_after_close(ik_bin: Path, tmp_path: Path) -> None:
    session = transport.Session(str(ik_bin), tmp_path / "s.db", None, timeout=10.0)
    show = {
        "protocol": {"major": transport.PROTOCOL_MAJOR, "minor": transport.PROTOCOL_MINOR},
        "runtime_version": transport.RUNTIME_VERSION,
        "request_id": "r1",
        "op": "show",
        "aggregate_id": "missing",
    }
    # A refusal is an ordinary response: it must not end the session.
    with pytest.raises(KernelError):
        session.invoke(show)
    with pytest.raises(KernelError):
        session.invoke({**show, "request_id": "r2"})
    session.close()
    session.close()  # idempotent
    with pytest.raises(TransportError):
        session.invoke({**show, "request_id": "r3"})
    with pytest.raises(TransportError):  # and still no implicit respawn/retry
        session.invoke({**show, "request_id": "r4"})
