"""Protocol walls, resource bounds, reliability and performance of the persistent worker/session.

Hermetic: fresh ``tmp_path`` stores, in-tree fixtures, fake ``ik`` shell scripts, pid files.
Waits are bounded liveness polls on a process's own exit, never fixed sleeps.

    RA-W1  hostile/malformed JSON-RPC from a worker fails closed, never mutates, never orphans
    RA-W2  a worker is recycled at EXACTLY the documented call bound (resource bound)
    RA-W3  killing ``ik`` (SIGKILL) cannot leave an orphaned reducer worker
    RA-W4  the Python ``Session`` never retries, never trusts a mismatched/garbled reply, and
           stays dead after a transport failure
    RA-W5  concurrent callers on one session are serialized without crossed responses
    RA-W6  detached children are reaped while the persistent worker/session stay idle and live
    RA-W7  worker child reaping preserves callback subprocess exit statuses
    RA-P1  persistent session is materially faster than one-shot over the SAME workload
"""
from __future__ import annotations

import os
import signal
import stat
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Any

import pytest

from invariant_kernel import Kernel, transport
from invariant_kernel.errors import KernelError, TransportError

HERE = Path(__file__).resolve().parent
PARITY = "reducer_fixtures:ParityReducer"
PID_REDUCER = "reducer_fixtures:PidRecordingReducer"

# Production default is 5000 (`DEFAULT_RECYCLE_AFTER` in src/external.rs).


def pids_in(marker: Path) -> list[int]:
    return [int(line) for line in marker.read_text(encoding="utf-8").split()] if marker.exists() else []


def wait_gone(pid: int, seconds: float = 60.0) -> bool:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return True
        time.sleep(0.01)
    return False


def accept(aggregate: Any, amount: int = 1) -> Any:
    return aggregate.execute({"name": "accept", "args": {"amount": amount}})


def snapshot(aggregate: Any) -> tuple[Any, int, Any]:
    shown = aggregate.show()
    return shown.state, shown.generation, shown.replay()


# ---------------------------------------------------------------- RA-W1 hostile worker replies

HOSTILE_MODES = [
    "wrong_id",
    "bad_version",
    "error_object",
    "neither",
    "both",
    "non_object_result",
    "garbage",
    "oversize",
    "eof",
    "hang",
]


def set_mode(tmp_path: Path, mode: str) -> None:
    (tmp_path / "mode.txt").write_text(mode, encoding="utf-8")


def hostile_machine(kernel: Kernel, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Any:
    set_mode(tmp_path, "none")
    monkeypatch.setenv("IK_HOSTILE_MODE_FILE", str(tmp_path / "mode.txt"))
    # Protocol-hostile cases must survive load; hang never replies, so a generous
    # worker bound still fails closed. 1.5s was incidental and flaked under load.
    machine = kernel.reducer(PARITY, python=sys.executable, worker_timeout=25.0)
    command = list(machine.worker_spec["command"])
    command[2] = str(HERE / "hostile_worker.py")  # same argv, different program; not in the semantic hash
    machine.worker_spec["command"] = command
    return machine


@pytest.mark.parametrize("session", [False, True], ids=["one-shot", "session"])
@pytest.mark.parametrize("mode", HOSTILE_MODES)
def test_hostile_worker_reply_fails_closed_without_mutation_or_orphans(
    ik_bin: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch, mode: str, session: bool
) -> None:
    marker = tmp_path / "pids.txt"
    monkeypatch.setenv("IK_WORKER_PID_FILE", str(marker))
    with Kernel(tmp_path / "k.db", binary=str(ik_bin), session=session, timeout=60.0) as kernel:
        machine = hostile_machine(kernel, tmp_path, monkeypatch)
        aggregate = accept(machine.create("agg"))
        before = snapshot(aggregate)

        set_mode(tmp_path, mode)
        with pytest.raises((KernelError, TransportError)):
            aggregate.execute({"name": "accept", "args": {"amount": 2}})
        assert snapshot(aggregate) == before, f"{mode}: failed command mutated durable state"

    for pid in set(pids_in(marker)):
        assert wait_gone(pid), f"{mode}: worker {pid} survived the failure/close"


def test_hostile_harness_negative_control_behaves_like_the_real_worker(
    ik_bin: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """`none` mode must succeed: otherwise every hostile case above could pass vacuously."""
    with Kernel(tmp_path / "k.db", binary=str(ik_bin), session=True) as kernel:
        machine = hostile_machine(kernel, tmp_path, monkeypatch)
        assert accept(accept(machine.create("agg"))).generation == 2


# ------------------------------------------------------------- RA-W2 recycle bound, exactly

def test_worker_is_recycled_at_exactly_the_documented_call_bound(
    ik_bin: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    bound = 20
    marker = tmp_path / "pids.txt"
    monkeypatch.setenv("IK_WORKER_PID_FILE", str(marker))
    monkeypatch.setenv("IK_RECYCLE_AFTER", str(bound))
    commands = bound // 2 + 2  # decide+apply per command => crosses the bound once
    with Kernel(tmp_path / "k.db", binary=str(ik_bin), session=True, timeout=60.0) as kernel:
        machine = kernel.reducer(PID_REDUCER, python=sys.executable)
        aggregate = machine.create("agg")
        for _ in range(commands):
            aggregate = accept(aggregate)
        assert aggregate.generation == commands, "recycling lost or duplicated a committed step"
        assert aggregate.show().generation == commands
    served = pids_in(marker)
    distinct = list(dict.fromkeys(served))
    assert len(distinct) == 2, f"expected exactly one recycle, saw workers {distinct}"
    assert served.count(distinct[0]) == bound, "first worker must serve exactly the bound"
    assert served.count(distinct[1]) == 2 * commands - bound
    assert wait_gone(distinct[0]), "recycled worker was not reaped"


# --------------------------------------------------------------- RA-W3 SIGKILL of ik, no orphan

@pytest.mark.skipif(not sys.platform.startswith("linux"), reason="parent-death signal is Linux-only")
def test_killing_ik_cannot_orphan_the_reducer_worker(
    ik_bin: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    marker = tmp_path / "pids.txt"
    monkeypatch.setenv("IK_WORKER_PID_FILE", str(marker))
    kernel = Kernel(tmp_path / "k.db", binary=str(ik_bin), session=True)
    try:
        machine = kernel.reducer(PID_REDUCER, python=sys.executable)
        accept(machine.create("agg"))
        (worker,) = set(pids_in(marker))
        (session,) = kernel._sessions.values()  # noqa: SLF001 - need the ik child pid to kill it
        os.kill(session._proc.pid, signal.SIGKILL)  # noqa: SLF001
        assert wait_gone(worker), "reducer worker outlived a SIGKILLed ik"
    finally:
        try:
            kernel.close()
        except Exception:  # noqa: BLE001 - ik is already dead by construction
            pass


# ----------------------------------------------------- RA-W4 Python Session transport discipline

def fake_ik(tmp_path: Path, body: str) -> Path:
    path = tmp_path / "fake-ik"
    path.write_text("#!/bin/sh\n" + body, encoding="utf-8")
    path.chmod(path.stat().st_mode | stat.S_IEXEC)
    return path


def show_request(request_id: str = "r1") -> dict[str, Any]:
    return {
        "protocol": {"major": transport.PROTOCOL_MAJOR, "minor": transport.PROTOCOL_MINOR},
        "runtime_version": transport.RUNTIME_VERSION,
        "request_id": request_id,
        "op": "show",
        "aggregate_id": "x",
    }


def test_session_timeout_kills_the_child_and_never_retries(tmp_path: Path) -> None:
    seen = tmp_path / "seen.txt"
    binary = fake_ik(tmp_path, f'while read line; do echo x >> "{seen}"; sleep 30; done\n')
    session = transport.Session(str(binary), tmp_path / "s.db", None, timeout=0.5)
    with pytest.raises(TransportError):
        session.invoke(show_request("r1"))
    try:  # SIGKILL is asynchronous: bounded wait for the child's own exit, not an instant poll
        session._proc.wait(timeout=5)  # noqa: SLF001
    except subprocess.TimeoutExpired:
        pytest.fail("timed-out ik child was not killed")
    with pytest.raises(TransportError):
        session.invoke(show_request("r2"))
    assert seen.read_text().split() == ["x"], "the request was re-sent (retry) or a second one reached ik"


@pytest.mark.parametrize(
    "body",
    [
        'read line; echo \'{"protocol":{"major":1,"minor":0},"request_id":"OTHER","ok":true,"result":{}}\'\n',
        "read line; echo 'not json at all'\n",
        "read line; exit 3\n",
    ],
    ids=["mismatched-request-id", "garbage-line", "exits-mid-request"],
)
def test_session_rejects_untrustworthy_replies_and_stays_dead(tmp_path: Path, body: str) -> None:
    session = transport.Session(str(fake_ik(tmp_path, body)), tmp_path / "s.db", None, timeout=5.0)
    with pytest.raises(TransportError):
        session.invoke(show_request("r1"))
    with pytest.raises(TransportError):
        session.invoke(show_request("r2"))


# ----------------------------------------------------------------- RA-W5 concurrent callers

def test_concurrent_callers_on_one_session_get_their_own_responses(ik_bin: Path, tmp_path: Path) -> None:
    threads, steps = 8, 10
    with Kernel(tmp_path / "k.db", binary=str(ik_bin), session=True) as kernel:
        machine = kernel.reducer(PARITY, python=sys.executable)
        machine.create("warmup")  # open the session before threads race to create it
        results: dict[int, tuple[str, int]] = {}
        errors: list[BaseException] = []

        def work(index: int) -> None:
            try:
                aggregate = machine.create(f"agg-{index}")
                for _ in range(steps):
                    aggregate = accept(aggregate)
                results[index] = (aggregate.id, aggregate.generation)
            except BaseException as exc:  # noqa: BLE001 - surfaced below
                errors.append(exc)

        pool = [threading.Thread(target=work, args=(i,)) for i in range(threads)]
        [t.start() for t in pool]
        [t.join(timeout=120) for t in pool]
        assert not errors, errors
        assert results == {i: (f"agg-{i}", steps) for i in range(threads)}, "responses crossed between callers"


# ------------------------------------------------------------ RA-W6/7 child ownership boundaries

@pytest.mark.skipif(os.name != "posix", reason="detached process cleanup is POSIX-specific")
def test_detached_descendant_is_reaped_while_worker_and_session_remain_idle(
    ik_bin: Path, tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    workers = tmp_path / "workers.txt"
    descendant = tmp_path / "descendant.pid"
    monkeypatch.setenv("IK_WORKER_PID_FILE", str(workers))
    monkeypatch.setenv("IK_DESCENDANT_PID_FILE", str(descendant))
    with Kernel(tmp_path / "k.db", binary=str(ik_bin), session=True) as kernel:
        machine = kernel.reducer(
            "reducer_fixtures:DetachedPidRecordingReducer", python=sys.executable
        )
        aggregate = machine.create("idle")
        result = aggregate.execute({"name": "noop", "args": {}})
        assert result.generation == 0
        assert result.state["score"] == 0
        (worker,) = set(pids_in(workers))
        (session,) = kernel._sessions.values()  # noqa: SLF001 - assert no owner exits
        assert session._proc.poll() is None  # noqa: SLF001
        os.kill(worker, 0)
        # No further request or close may be needed to reap the terminated child.
        assert wait_gone(int(descendant.read_text(encoding="utf-8")), seconds=2.0)
        assert session._proc.poll() is None  # noqa: SLF001
        os.kill(worker, 0)


def test_callback_subprocess_exit_status_drives_committed_semantic_result(
    ik_bin: Path, tmp_path: Path
) -> None:
    with Kernel(tmp_path / "k.db", binary=str(ik_bin), session=True) as kernel:
        machine = kernel.reducer(
            "reducer_fixtures:SubprocessExitStatusReducer", python=sys.executable
        )
        aggregate = machine.create("exit-status")
        # Discovery's callback exits 3; worker loading exits 5 and decide exits 7.
        assert aggregate.state["score"] == 3
        result = accept(aggregate)
        assert result.generation == 1
        assert result.state["score"] == 15
        assert result.show().state["score"] == 15


# ------------------------------------------------------------------------- RA-P1 performance

def run_workload(ik_bin: Path, store: Path, session: bool, commands: int) -> float:
    started = time.perf_counter()
    with Kernel(store, binary=str(ik_bin), session=session) as kernel:
        machine = kernel.reducer(PARITY, python=sys.executable)
        aggregate = machine.create("agg")
        for _ in range(commands):
            aggregate = accept(aggregate)
        assert aggregate.generation == commands
    return time.perf_counter() - started


def test_persistent_session_is_materially_faster_than_one_shot_for_the_same_workload(
    ik_bin: Path, tmp_path: Path
) -> None:
    """Self-normalizing ratio (same machine, same load, best-of-3) rather than an absolute time.

    Envelope: Python reducer via the real worker, 8 sequential commands, Linux, local disk. The
    measured ratio on the reference host is ~4x; the gate is 2x so scheduler noise cannot flake it
    while a regression to per-command process spawn (ratio ~1x) still fails."""
    commands = 8
    one_shot = min(run_workload(ik_bin, tmp_path / f"o{i}.db", False, commands) for i in range(3))
    persistent = min(run_workload(ik_bin, tmp_path / f"p{i}.db", True, commands) for i in range(3))
    ratio = one_shot / persistent
    print(f"one-shot {one_shot:.3f}s  session {persistent:.3f}s  ratio {ratio:.2f}x")
    assert ratio >= 2.0, f"session only {ratio:.2f}x faster than one-shot"
