#!/usr/bin/env python3
"""Reproducible local performance benchmark for Desmo (`ik --session` & `ik list`).

Usage:
  cargo build --release --bin ik && python3 scripts/bench.py [--json]
"""

from __future__ import annotations

import argparse
import json
import math
import os
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ORDER_TOML = ROOT / "examples" / "orders" / "order.ik.toml"


def resolve_ik(explicit: str | None) -> Path:
    if explicit:
        return Path(explicit)
    if env := os.environ.get("IK_BINARY"):
        return Path(env)
    rel = ROOT / "target" / "release" / "ik"
    if rel.is_file():
        return rel
    dbg = ROOT / "target" / "debug" / "ik"
    if dbg.is_file():
        return dbg
    raise SystemExit("ik binary not found; run `cargo build --release --bin ik` first")


def percentile(sorted_ms: list[float], p: float) -> float:
    if not sorted_ms:
        return 0.0
    idx = min(len(sorted_ms) - 1, max(0, math.ceil(p * len(sorted_ms)) - 1))
    return sorted_ms[idx]


class Session:
    def __init__(self, binary: Path, store: Path, machine_path: Path, env: dict[str, str]) -> None:
        self.proc = subprocess.Popen(
            [str(binary), "--store", str(store), "--machine", str(machine_path), "--session"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=env,
        )
        self.seq = 0

    def call(self, op: str, aggregate_id: str, identity: dict, **extra: object) -> dict:
        self.seq += 1
        req = {
            "protocol": {"major": 1, "minor": 0},
            "runtime_version": self.runtime_version,
            "request_id": f"bench-{self.seq}",
            "op": op,
            "aggregate_id": aggregate_id,
            "machine": identity,
            **extra,
        }
        assert self.proc.stdin is not None and self.proc.stdout is not None
        self.proc.stdin.write(json.dumps(req, separators=(",", ":")) + "\n")
        self.proc.stdin.flush()
        line = self.proc.stdout.readline()
        if not line:
            err = self.proc.stderr.read() if self.proc.stderr else ""
            raise RuntimeError(f"ik --session closed unexpectedly: {err}")
        resp = json.loads(line)
        if not resp.get("ok"):
            raise RuntimeError(f"ik --session refused {op} on {aggregate_id}: {resp}")
        return resp["result"]

    def close(self) -> None:
        if self.proc.stdin:
            self.proc.stdin.close()
        self.proc.wait(timeout=10)


def runtime_version(binary: Path) -> str:
    out = subprocess.check_output([str(binary), "--version"], text=True).strip()
    return out.split()[1]


def machine_identity(binary: Path, machine_path: Path) -> dict:
    return json.loads(subprocess.check_output([str(binary), "identity", str(machine_path)], text=True))


def bench_commits(binary: Path, store: Path, sync_mode: str, orders_count: int = 100) -> dict:
    env = os.environ.copy()
    if sync_mode == "full":
        env.pop("IK_SQLITE_SYNCHRONOUS", None)
    else:
        env["IK_SQLITE_SYNCHRONOUS"] = sync_mode

    ident = machine_identity(binary, ORDER_TOML)
    ver = runtime_version(binary)
    sess = Session(binary, store, ORDER_TOML, env)
    sess.runtime_version = ver

    # Warmup (compile/schema/meta setup excluded from latency distribution)
    w = sess.call("create", "warmup-0", ident)
    sess.call(
        "command",
        "warmup-0",
        ident,
        capability=w["capability"],
        command={"name": "cancel", "args": {}},
    )

    latencies_ms: list[float] = []
    t_start = time.perf_counter()
    for i in range(orders_count):
        agg = f"ord-{i:05d}"
        t0 = time.perf_counter()
        cur = sess.call("create", agg, ident)
        t1 = time.perf_counter()
        latencies_ms.append((t1 - t0) * 1000.0)

        for cmd in (
            {"name": "pay", "args": {"payment_ref": f"ch_{i}"}},
            {"name": "deliver", "args": {}},
            {"name": "send_back", "args": {}},
            {"name": "refund", "args": {}},
        ):
            ts = time.perf_counter()
            cur = sess.call("command", agg, ident, capability=cur["capability"], command=cmd)
            te = time.perf_counter()
            latencies_ms.append((te - ts) * 1000.0)
    total_s = time.perf_counter() - t_start
    sess.close()

    latencies_ms.sort()
    commits = len(latencies_ms)
    return {
        "synchronous": sync_mode.upper(),
        "commits": commits,
        "total_s": round(total_s, 3),
        "commits_per_sec": round(commits / total_s, 1),
        "p50_ms": round(percentile(latencies_ms, 0.50), 3),
        "p99_ms": round(percentile(latencies_ms, 0.99), 3),
    }


def bench_list_5k(binary: Path, store: Path) -> dict:
    env = os.environ.copy()
    env["IK_SQLITE_SYNCHRONOUS"] = "normal"  # fast seeding for the 5k fixture
    ident = machine_identity(binary, ORDER_TOML)
    ver = runtime_version(binary)
    sess = Session(binary, store, ORDER_TOML, env)
    sess.runtime_version = ver

    total_rows = 5000
    for i in range(total_rows):
        agg = f"agg-{i:05d}"
        cur = sess.call("create", agg, ident)
        if i % 5 == 0:
            sess.call(
                "command",
                agg,
                ident,
                capability=cur["capability"],
                command={"name": "cancel", "args": {}},
            )
    sess.close()

    runs_ms: list[float] = []
    for _ in range(20):
        t0 = time.perf_counter()
        out = subprocess.check_output(
            [
                str(binary),
                "list",
                str(store),
                "--machine",
                "order",
                "--terminal",
                "false",
                "--limit",
                "50",
                "--json",
            ],
            text=True,
        )
        runs_ms.append((time.perf_counter() - t0) * 1000.0)
        payload = json.loads(out)
        assert payload["schema"] == "ik.list.v1"
        assert len(payload["aggregates"]) == 50
        assert payload["has_more"] is True

    runs_ms.sort()
    return {
        "aggregates_total": total_rows,
        "limit": 50,
        "runs": len(runs_ms),
        "p50_ms": round(percentile(runs_ms, 0.50), 2),
        "p99_ms": round(percentile(runs_ms, 0.99), 2),
        "mean_ms": round(statistics.mean(runs_ms), 2),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="Benchmark Desmo persistent session & indexed list")
    parser.add_argument("--binary", help="Path to ik binary (default: IK_BINARY or target/release/ik)")
    parser.add_argument("--json", action="store_true", help="Emit JSON output")
    args = parser.parse_args()

    binary = resolve_ik(args.binary)
    with tempfile.TemporaryDirectory(prefix="desmo-bench-") as tmp:
        tmp_path = Path(tmp)
        full_res = bench_commits(binary, tmp_path / "full.db", "full")
        normal_res = bench_commits(binary, tmp_path / "normal.db", "normal")
        list_res = bench_list_5k(binary, tmp_path / "list5k.db")

    report = {
        "binary": str(binary),
        "commits_full": full_res,
        "commits_normal": normal_res,
        "list_5k": list_res,
    }
    if args.json:
        print(json.dumps(report, indent=2))
    else:
        print(f"Binary: {binary}")
        print(
            f"Session commits (WAL + FULL):   {full_res['commits_per_sec']:7.1f} commits/s  "
            f"p50={full_res['p50_ms']:.3f}ms  p99={full_res['p99_ms']:.3f}ms  ({full_res['commits']} commits)"
        )
        print(
            f"Session commits (WAL + NORMAL): {normal_res['commits_per_sec']:7.1f} commits/s  "
            f"p50={normal_res['p50_ms']:.3f}ms  p99={normal_res['p99_ms']:.3f}ms  ({normal_res['commits']} commits)"
        )
        print(
            f"Indexed ik list (5k rows, CLI): p50={list_res['p50_ms']:.2f}ms  "
            f"p99={list_res['p99_ms']:.2f}ms  (includes subprocess spawn)"
        )


if __name__ == "__main__":
    main()
