#!/usr/bin/env python3
"""Measure write contention on one Ikaros store (IK-07).

Every request is recorded, including failures and timeouts; latency statistics are computed
over ALL requests so slowdown cannot be hidden by dropping failed ones. Run:

    python3 validation/ik07/qualify_contention.py --ik target/debug/ik --out validation/ik07/report.json

Declared workload and budgets are in docs/execution-envelope.md and are echoed into the report.
"""

from __future__ import annotations

import argparse
import json
import multiprocessing
import os
import platform
import resource
import sqlite3
import sys
import tempfile
import time
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(REPO / "python" / "src"))
sys.path.insert(0, str(HERE))

from invariant_kernel import Kernel  # noqa: E402
from invariant_kernel.errors import KernelError, StaleCapabilityError, TransportError  # noqa: E402

COUNTER = HERE / "counter.ik.toml"
ENTRYPOINT = "ik07_reducers:ControlledReducer"
WORKER_TIMEOUT_S = 1.0
KERNEL_TIMEOUT_S = 10.0
VERIFY_TIMEOUT_S = 120.0

# Declared alpha workload (see docs/execution-envelope.md). Budgets are the pass criteria.
# Derived from the two named consumers, not from benchmark ambition: a human-paced
# interactive-fiction session (external Python reducer) and a native delegation lifecycle.
WORKLOAD = {
    "topology": "one persistent local SQLite store, one host, several writer processes",
    "writers": 4,
    "aggregates_per_store": 100,
    "offered_write_load": "<= 2 commits/s sustained across <= 4 writers, bursts <= 5/s",
    "external_reducer_latency_ms": 50,
    "worker_timeout_ms": int(WORKER_TIMEOUT_S * 1000),
    "busy_timeout_ms": 5000,
}
BUDGETS = {
    "caller_wait_p99_ms_normal": 2000,
    "hung_worker_fails_within_ms": 2500,
    "other_writer_blocked_by_hang_at_most_ms": 2500,
    "recovery_within_ms": 5000,
}


def classify(err: BaseException) -> str:
    if isinstance(err, StaleCapabilityError):
        return "stale_refusal"
    text = str(err).lower()
    if "locked" in text or "busy" in text:
        return "store_busy"
    if isinstance(err, TransportError):
        return "timeout_or_transport"
    if isinstance(err, KernelError):
        return f"refused:{err.code}"
    return f"other:{type(err).__name__}"


def percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, int(round(pct / 100.0 * (len(ordered) - 1)))))
    return round(ordered[index], 2)


def latency_block(latencies: list[float]) -> dict[str, float]:
    return {
        "p50": percentile(latencies, 50),
        "p95": percentile(latencies, 95),
        "p99": percentile(latencies, 99),
        "max": round(max(latencies), 2) if latencies else 0.0,
    }


def summarize(rows: list[dict[str, Any]], wall_s: float) -> dict[str, Any]:
    outcomes: dict[str, int] = {}
    for row in rows:
        outcomes[row["outcome"]] = outcomes.get(row["outcome"], 0) + 1
    return {
        "requests": len(rows),
        "completed": outcomes.get("ok", 0),
        "outcomes": outcomes,
        # Every request, failures included. The split below only separates each writer's first
        # request (which pays worker/session startup while holding the store write lock).
        "latency_ms_all_requests": latency_block([row["ms"] for row in rows]),
        "latency_ms_first_request_per_writer": latency_block([r["ms"] for r in rows if r["first"]]),
        "latency_ms_steady_state": latency_block([r["ms"] for r in rows if not r["first"]]),
        "wall_s": round(wall_s, 3),
        "completed_per_s": round(outcomes.get("ok", 0) / wall_s, 2) if wall_s else 0.0,
    }


def open_target(spec: dict[str, Any]):
    kernel = Kernel(
        spec["store"],
        binary=spec["binary"],
        timeout=KERNEL_TIMEOUT_S,
        session=spec["warm"],
    )
    if spec["mode"] == "declarative":
        return kernel, kernel.machine(COUNTER)
    machine = kernel.reducer(ENTRYPOINT, python=sys.executable, worker_timeout=WORKER_TIMEOUT_S)
    return kernel, machine


def one_request(handle: Any, spec: dict[str, Any], delay_ms: float, mode: str) -> Any:
    if spec["mode"] == "declarative":
        return handle.command("tick")
    return handle.execute({"name": "accept", "args": {"amount": 1, "delay_ms": delay_ms, "mode": mode}})


def request_mode(spec: dict[str, Any], index: int) -> str:
    if spec.get("every_request_mode"):
        return spec["request_mode"]
    if index == 0 and spec.get("first_request_mode"):
        return spec["request_mode"]
    return "normal"


def after_failure(outcome: str, spec: dict[str, Any], kernel: Kernel, machine: Any, handle: Any):
    """Reread the aggregate after a stale refusal; reopen the session after a transport/busy failure."""
    if outcome == "stale_refusal":
        return kernel, machine, machine.show(spec["aggregate"])
    if outcome in ("timeout_or_transport", "store_busy"):
        kernel.close()
        kernel, machine = open_target(spec)
        return kernel, machine, machine.show(spec["aggregate"])
    return kernel, machine, handle


def attempt(spec: dict[str, Any], kernel: Kernel, machine: Any, handle: Any, index: int):
    """One request. Every failure is a data point and is never dropped."""
    began = time.time()
    try:
        handle = one_request(handle, spec, spec["delay_ms"], request_mode(spec, index))
        outcome, detail = "ok", ""
    except Exception as err:
        outcome, detail = classify(err), str(err)[:160]
        kernel, machine, handle = after_failure(outcome, spec, kernel, machine, handle)
    row = {
        "writer": spec["name"],
        "first": index == 0,
        "at_s": round(began - spec["start_at"], 3),
        "ms": round((time.time() - began) * 1000, 2),
        "outcome": outcome,
        "detail": detail,
    }
    return kernel, machine, handle, row


def writer(spec: dict[str, Any]) -> dict[str, Any]:
    """One writer process; returns every request it made."""
    rows: list[dict[str, Any]] = []
    kernel, machine = open_target(spec)
    handle = machine.show(spec["aggregate"])
    while time.time() < spec["start_at"]:
        time.sleep(0.001)
    for index in range(spec["count"]):
        kernel, machine, handle, row = attempt(spec, kernel, machine, handle, index)
        rows.append(row)
        if spec.get("pace_ms"):
            time.sleep(spec["pace_ms"] / 1000.0)
    kernel.close()
    peak = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    return {"rows": rows, "child_peak_rss_raw": peak}


def store_facts(store: Path) -> dict[str, Any]:
    conn = sqlite3.connect(str(store))
    journal = conn.execute("SELECT COUNT(*) FROM journal").fetchone()[0]
    ops = conn.execute("SELECT COUNT(*) FROM operations").fetchone()[0]
    conn.close()
    size = sum(p.stat().st_size for p in store.parent.glob(store.name + "*") if p.is_file())
    return {"journal_rows": journal, "operation_rows": ops, "store_bytes_incl_wal": size}


def prepare(binary: str, store: Path, mode: str, aggregates: list[str]) -> None:
    kernel = Kernel(store, binary=binary, timeout=KERNEL_TIMEOUT_S)
    machine = (
        kernel.machine(COUNTER)
        if mode == "declarative"
        else kernel.reducer(ENTRYPOINT, python=sys.executable, worker_timeout=WORKER_TIMEOUT_S)
    )
    for aggregate in aggregates:
        machine.create(aggregate)


def run_writers(specs: list[dict[str, Any]]) -> tuple[list[dict[str, Any]], float, int]:
    ctx = multiprocessing.get_context("spawn")
    start_at = time.time() + 2.0
    for spec in specs:
        spec["start_at"] = start_at
    with ctx.Pool(len(specs)) as pool:
        results = pool.map(writer, specs)
    rows = [row for result in results for row in result["rows"]]
    wall = max((row["at_s"] + row["ms"] / 1000.0 for row in rows), default=0.0)
    return rows, wall, max(result["child_peak_rss_raw"] for result in results)


def base_spec(binary: str, store: Path, name: str, mode: str, warm: bool, aggregate: str, count: int, delay_ms: float) -> dict[str, Any]:
    return {
        "binary": binary,
        "store": str(store),
        "name": name,
        "mode": mode,
        "warm": warm,
        "aggregate": aggregate,
        "count": count,
        "delay_ms": delay_ms,
        "request_mode": "normal",
    }


def experiment(
    name: str,
    binary: str,
    mode: str,
    warm: bool,
    writers: int,
    count: int,
    delay_ms: float,
    shared: bool,
    pace_ms: float = 0.0,
) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix="ik07-") as tmp:
        store = Path(tmp) / "store.db"
        aggregates = ["shared"] if shared else [f"agg-{i}" for i in range(writers)]
        prepare(binary, store, mode, aggregates)
        before = store_facts(store)
        specs = [
            base_spec(binary, store, f"w{i}", mode, warm, "shared" if shared else aggregates[i], count, delay_ms)
            for i in range(writers)
        ]
        for spec in specs:
            spec["pace_ms"] = pace_ms
        rows, wall, rss = run_writers(specs)
        after = store_facts(store)
        verify_began = time.time()
        verify_ok = verify_store(binary, store, mode)
        verify_ms = round((time.time() - verify_began) * 1000, 1)
    return {
        "experiment": name,
        "mode": mode,
        "client": "warm_session" if warm else "cold_one_shot",
        "writers": writers,
        "same_aggregate": shared,
        "reducer_delay_ms": delay_ms,
        "think_time_ms": pace_ms,
        **summarize(rows, wall),
        "child_peak_rss_raw": rss,
        "journal_growth": {k: after[k] - before[k] for k in before},
        "store_verifies": verify_ok,
        "verify_ms": verify_ms,
    }


def verify_store(binary: str, store: Path, mode: str) -> bool:
    """Full journal verify through a warm session (a one-shot verify would also pay worker spawn per event)."""
    kernel = Kernel(store, binary=binary, timeout=VERIFY_TIMEOUT_S, session=True)
    machine = (
        kernel.machine(COUNTER)
        if mode == "declarative"
        else kernel.reducer(ENTRYPOINT, python=sys.executable, worker_timeout=WORKER_TIMEOUT_S)
    )
    try:
        machine.verify()
        return True
    except KernelError:
        return False
    finally:
        kernel.close()


def verify_scaling_experiment(binary: str, mode: str, counts: list[int]) -> dict[str, Any]:
    """Cost of verify/replay as the journal grows: it re-runs the machine for every event."""
    points = []
    for events in counts:
        with tempfile.TemporaryDirectory(prefix="ik07-verify-") as tmp:
            store = Path(tmp) / "store.db"
            kernel = Kernel(store, binary=binary, timeout=VERIFY_TIMEOUT_S, session=True)
            machine = (
                kernel.machine(COUNTER)
                if mode == "declarative"
                else kernel.reducer(ENTRYPOINT, python=sys.executable, worker_timeout=WORKER_TIMEOUT_S)
            )
            machine.create("a")
            handle = machine.show("a")
            for _ in range(events):
                handle = one_request(handle, {"mode": mode}, 0, "normal")
            began = time.time()
            try:
                machine.verify()
                outcome = "ok"
            except KernelError as err:
                outcome = classify(err)
            verify_ms = round((time.time() - began) * 1000, 1)
            kernel.close()
        points.append(
            {"events": events, "verify_ms": verify_ms, "ms_per_event": round(verify_ms / events, 2), "outcome": outcome}
        )
    return {"experiment": "verify_cost_scaling", "mode": mode, "points": points}


def hung_reducer_experiment(binary: str) -> dict[str, Any]:
    """One writer hits a hung reducer; an unrelated writer works at a steady cadence meanwhile."""
    with tempfile.TemporaryDirectory(prefix="ik07-hang-") as tmp:
        store = Path(tmp) / "store.db"
        prepare(binary, store, "external", ["victim", "bystander"])
        hung = base_spec(binary, store, "hung", "external", True, "victim", 1, 0)
        hung.update(request_mode="hang", first_request_mode=True)
        steady = base_spec(binary, store, "bystander", "external", True, "bystander", 40, 5)
        steady["pace_ms"] = 100
        rows, wall, rss = run_writers([hung, steady])
        hung_rows = [r for r in rows if r["writer"] == "hung"]
        steady_rows = [r for r in rows if r["writer"] == "bystander"]
        recovered = recover_probe(binary, store)
        facts = store_facts(store)
        verify_ok = verify_store(binary, store, "external")
    return {
        "experiment": "hung_reducer_with_unrelated_writer",
        "worker_timeout_ms": int(WORKER_TIMEOUT_S * 1000),
        "hung_request": hung_rows,
        "bystander": summarize(steady_rows, wall),
        "bystander_max_block_ms": max((r["ms"] for r in steady_rows), default=0.0),
        "recovery_probe": recovered,
        "store_verifies": verify_ok,
        "store": facts,
        "child_peak_rss_raw": rss,
    }


def recover_probe(binary: str, store: Path, aggregate: str = "victim") -> dict[str, Any]:
    """After the failure: reopen from scratch and commit one normal command."""
    began = time.time()
    try:
        kernel = Kernel(store, binary=binary, timeout=KERNEL_TIMEOUT_S)
        machine = kernel.reducer(ENTRYPOINT, python=sys.executable, worker_timeout=WORKER_TIMEOUT_S)
        machine.show(aggregate).execute({"name": "accept", "args": {"amount": 1}})
        ok, detail = True, ""
    except Exception as err:
        ok, detail = False, f"{classify(err)}: {str(err)[:120]}"
    return {"ok": ok, "ms": round((time.time() - began) * 1000, 2), "detail": detail}


def killed_worker_experiment(binary: str) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix="ik07-kill-") as tmp:
        store = Path(tmp) / "store.db"
        prepare(binary, store, "external", ["victim"])
        killer = base_spec(binary, store, "killer", "external", True, "victim", 1, 0)
        killer.update(request_mode="kill", first_request_mode=True)
        rows, wall, rss = run_writers([killer])
        recovered = recover_probe(binary, store)
        verify_ok = verify_store(binary, store, "external")
    return {
        "experiment": "worker_killed_mid_decision",
        "killed_request": rows,
        "recovery_probe": recovered,
        "store_verifies": verify_ok,
        "child_peak_rss_raw": rss,
    }


def overload_experiment(binary: str, writers: int) -> dict[str, Any]:
    """More simultaneous hung reducers than busy_timeout can queue: beyond the qualified envelope."""
    with tempfile.TemporaryDirectory(prefix="ik07-overload-") as tmp:
        store = Path(tmp) / "store.db"
        aggregates = [f"agg-{i}" for i in range(writers)]
        prepare(binary, store, "external", aggregates)
        specs = []
        for index in range(writers):
            spec = base_spec(binary, store, f"w{index}", "external", True, aggregates[index], 1, 0)
            spec.update(request_mode="hang", first_request_mode=True)
            specs.append(spec)
        rows, wall, rss = run_writers(specs)
        recovered = recover_probe(binary, store, "agg-0")
        verify_ok = verify_store(binary, store, "external")
    return {
        "experiment": "overload_simultaneous_hung_reducers",
        "writers": writers,
        "worker_timeout_ms": int(WORKER_TIMEOUT_S * 1000),
        **summarize(rows, wall),
        "recovery_probe": recovered,
        "store_verifies": verify_ok,
        "child_peak_rss_raw": rss,
    }


def non_ok(result: dict[str, Any]) -> dict[str, int]:
    return {k: v for k, v in result["outcomes"].items() if k != "ok"}


def capacity_row(result: dict[str, Any]) -> dict[str, Any]:
    return {
        "budget": "capacity_observation_not_judged",
        "case": f"{result['experiment']}/{result['mode']}",
        "completed_per_s": result["completed_per_s"],
        "steady_p99_ms": result["latency_ms_steady_state"]["p99"],
        "non_ok_outcomes": non_ok(result),
        "pass": None,
    }


def latency_row(result: dict[str, Any]) -> dict[str, Any]:
    observed = result["latency_ms_all_requests"]["p99"]
    limit = BUDGETS["caller_wait_p99_ms_normal"]
    return {
        "budget": "caller_wait_p99_ms_normal",
        "case": f"{result['experiment']}/{result['mode']}/{result['client']}/delay={result.get('reducer_delay_ms')}",
        "observed": observed,
        "limit": limit,
        "pass": observed <= limit,
        "steady_state_p99_ms": result["latency_ms_steady_state"]["p99"],
        "first_request_p99_ms": result["latency_ms_first_request_per_writer"]["p99"],
        "non_ok_outcomes": non_ok(result),
    }


def hang_rows(result: dict[str, Any]) -> list[dict[str, Any]]:
    hung = result["hung_request"][0]
    fail_limit = BUDGETS["hung_worker_fails_within_ms"]
    block_limit = BUDGETS["other_writer_blocked_by_hang_at_most_ms"]
    blocked = result["bystander_max_block_ms"]
    return [
        {
            "budget": "hung_worker_fails_within_ms",
            "observed": hung["ms"],
            "limit": fail_limit,
            "pass": hung["outcome"] != "ok" and hung["ms"] <= fail_limit,
            "diagnostic": hung["detail"],
        },
        {
            "budget": "other_writer_blocked_by_hang_at_most_ms",
            "observed": blocked,
            "limit": block_limit,
            "pass": blocked <= block_limit,
        },
    ]


def recovery_row(result: dict[str, Any]) -> dict[str, Any]:
    probe = result["recovery_probe"]
    limit = BUDGETS["recovery_within_ms"]
    return {
        "budget": "recovery_within_ms",
        "case": result["experiment"],
        "observed": probe["ms"],
        "limit": limit,
        "pass": probe["ok"] and result["store_verifies"] and probe["ms"] <= limit,
    }


def judge(result: dict[str, Any]) -> list[dict[str, Any]]:
    name = result["experiment"]
    if name.startswith("saturation_"):
        return [capacity_row(result)]
    if name.startswith("offered_load") or name == "uncontended":
        return [latency_row(result)]
    if name == "hung_reducer_with_unrelated_writer":
        return [*hang_rows(result), recovery_row(result)]
    if name == "worker_killed_mid_decision":
        return [recovery_row(result)]
    return []


def evaluate(results: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Pass/fail of every declared budget against what was observed. Nothing is dropped."""
    return [row for result in results for row in judge(result)]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--ik", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--quick", action="store_true", help="fewer requests (smoke only; do not publish)")
    args = parser.parse_args()
    binary = str(Path(args.ik).resolve())
    n = 10 if args.quick else 60
    offered = 4 if args.quick else 20

    results: list[dict[str, Any]] = []
    for warm in (False, True):
        results.append(experiment("uncontended", binary, "declarative", warm, 1, n, 0, False))
        results.append(experiment("uncontended", binary, "external", warm, 1, n, 0, False))
        results.append(experiment("uncontended", binary, "external", warm, 1, n, 50, False))
    # Offered load derived from the declared workload (docs/execution-envelope.md).
    results.append(experiment("offered_load_2_per_s", binary, "external", True, 4, offered, 50, False, pace_ms=1800))
    results.append(experiment("offered_load_5_per_s", binary, "external", True, 4, offered, 50, False, pace_ms=650))
    results.append(experiment("offered_load_2_per_s", binary, "declarative", True, 4, offered, 0, False, pace_ms=1800))
    # Saturation (closed loop, no think time): capacity and starvation behaviour, not a budget.
    results.append(experiment("saturation_same_aggregate", binary, "declarative", True, 4, n, 0, True))
    results.append(experiment("saturation_same_aggregate", binary, "external", True, 4, n, 50, True))
    results.append(experiment("saturation_unrelated_aggregates", binary, "declarative", True, 4, n, 0, False))
    results.append(experiment("saturation_unrelated_aggregates", binary, "external", True, 4, n, 50, False))
    results.append(hung_reducer_experiment(binary))
    results.append(killed_worker_experiment(binary))
    results.append(overload_experiment(binary, 8))
    results.append(verify_scaling_experiment(binary, "declarative", [100, 1000] if not args.quick else [20]))
    results.append(verify_scaling_experiment(binary, "external", [25, 50, 100] if not args.quick else [10]))

    report = {
        "schema": "ikaros.ik07.contention.v1",
        "environment": {
            "platform": platform.platform(),
            "machine": platform.machine(),
            "python": platform.python_version(),
            "cpu_count": os.cpu_count(),
            "rss_units": "ru_maxrss: bytes on macOS, KiB on Linux",
            "ik": binary,
            "quick": args.quick,
        },
        "workload": WORKLOAD,
        "budgets": BUDGETS,
        "results": results,
        "evaluation": evaluate(results),
    }
    Path(args.out).write_text(json.dumps(report, indent=2) + "\n")
    print(f"wrote {args.out} ({len(results)} experiments)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
