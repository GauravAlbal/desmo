# Execution envelope (IK-07)

What a single Desmo store was measured to sustain, and the one decision taken
from it. Raw observations: `validation/ik07/report.json` (declarative and
external reducers, process-level) and `validation/ik07/native.json` (native
machines). Reproduce:

```bash
cargo build --release --bin ik
python3 validation/ik07/qualify_contention.py --ik target/release/ik --out validation/ik07/report.json
cargo run --release --example contention_native -- validation/ik07/native.json
```

Measured on one host only: Apple M1 Pro, macOS 14.7 arm64, 8 CPUs, Python 3.9.6,
release build of `ik` 0.4.1 + schema 4. **Linux x64 was not measured** (no runner
was available); nothing below is claimed for it.

## The mechanism being measured

A command evaluates its reducer **while the store's `BEGIN IMMEDIATE` write
transaction is held** (`src/store.rs`; external evaluation in `src/external.rs`).
One store therefore has one writer at a time, across all aggregates. Contenders
wait on SQLite's busy handler with `busy_timeout` = 5 s; waiters are not served
first-come-first-served.

## Declared workload and budgets

Derived from the two named consumers, not from benchmark ambition. They are
provisional until the consumers confirm them (IK-10, IK-13).

| Item | Declared value | Source |
|---|---|---|
| Topology | one persistent local SQLite store, one host, several writer processes | `docs/alpha-contract.md` |
| Reducer modes | declarative, native, external Python (session worker) | alpha slice |
| Writers | at most 4 per store | a few concurrent interactive sessions |
| Offered load | ≤ 2 commits/s sustained, bursts ≤ 5/s, across all writers | human-paced interactive fiction; a native delegation lifecycle is native and far lighter per store |
| External reducer service time | ~50 ms | typical small Python `decide` |
| Worker timeout | 1 s (`worker_timeout`) | client default is min(5 s, timeout−0.5 s) |
| Caller wait, normal requests | p99 ≤ 2000 ms over **all** requests, failures included | interactive tolerance |
| Hung worker | fails within 2500 ms | worker timeout + overhead |
| Other writers blocked by a hung worker | ≤ 2500 ms | same |
| Recovery after worker failure | store usable and `verify` passes within 5000 ms | operational |

## Results

| Case | Result |
|---|---|
| Native, 1 writer | p99 0.96 ms, 4,600 commits/s |
| Native, 4 writers, same aggregate | p99 2.4 ms; 3 of 600 stale refusals; 3,350 commits/s |
| Native, 4 writers, unrelated aggregates | p99 2.4 ms; 3,780 commits/s |
| Declarative, warm session, 1 writer | p99 0.8 ms |
| Declarative, cold one-shot (`ik` per call), 1 writer | p99 235 ms (process start dominates) |
| Declarative, 4 writers saturated | p99 ≈ 40 ms; 1,360–1,570 commits/s |
| External, warm, 1 writer, 50 ms reducer | steady p99 262 ms (≈190 ms p50); first request 444 ms; ~5 commits/s capacity |
| External, cold one-shot, 1 writer | p99 ≈ 840 ms (every call spawns `ik` and a worker) |
| External, 4 writers, offered 2/s, 50 ms reducer | **budget met**: all-request p99 1896 ms, steady p99 306 ms; **cold-start tail 2478 ms** |
| External, 4 writers, offered 5/s, 50 ms reducer | budget met: all-request p99 1254 ms, steady p99 532 ms; cold-start tail 2078 ms |
| External, 4 writers, **saturated** (no think time) | capacity ≈ 5 commits/s; **9–10 of 240 requests fail `store_busy` after ≈ 5.7–6.2 s** while others progress |
| Hung external reducer | fails in 2076 ms with `IK_INTERNAL rule=reducer.timeout`; an unrelated writer was blocked up to 2013 ms; recovery probe 1462 ms; `verify` passes |
| External worker killed mid-decision (SIGKILL) | fails in 257 ms with `rule=reducer.exit`; recovery 2239 ms; `verify` passes; nothing partial committed |
| 8 simultaneous hung reducers (beyond envelope) | 0 completed; 4 time out at the worker limit, 4 fail `store_busy` after ≈ 5.7–6.6 s; store recovers (probe 983 ms) |
| `verify`/`replay` cost, declarative | 0.02–0.05 ms per event (1,000 events: 48 ms) |
| `verify`/`replay` cost, external | **55–72 ms per event** (25 events 1.8 s; 100 events 5.5 s; ~230 events ≈ 18 s) |

Latency percentiles include failed and timed-out requests; nothing was dropped
from them. The first request of each writer pays worker/session start-up while
holding the store lock; it is reported separately and also included in the
all-request figure.

## Findings that bound the claim

1. **The store-wide lock is real.** A hung external reducer blocks every other
   writer on that store for up to the worker timeout. Different aggregates do not
   help; different *stores* would.
2. **Simultaneous cold start is the dominant tail.** Four external writers that
   begin at the same instant queue their worker start-ups (≈0.3–0.5 s each) behind
   one lock: first requests took up to 2.5 s, above the 2 s budget, although the
   all-request p99 stays under it at the declared load (80-request samples, so p99
   is close to the worst few requests).
3. **No fairness under saturation.** At closed-loop saturation with four writers,
   a waiter can exceed `busy_timeout` and fail while the lock keeps cycling
   through other writers. This is outside the declared load.
4. **External `verify`/`replay` is linear in journal length with a worker round
   trip per event.** It is not a constant-time audit; a long-lived external-reducer
   aggregate needs minutes.
5. Failure isolation held: a hung or killed worker never partially persisted, the
   failure was reported within its documented limit, and the store verified and
   accepted new writes afterwards.

## Decision

**Retain the current architecture** (reducer evaluation inside the single
store-wide write transaction). The declared budgets are met for the declared
workload, so the ticket's conditional redesign (optimistic evaluation,
lock-shortening, a scheduler) is **not** started. Alpha release scope is narrowed
explicitly instead:

- Qualified: native and declarative machines at the measured rates, and external
  Python reducers at ≤ 2 sustained commits/s (bursts ≤ 5/s) from ≤ 4 writers on
  one store, with a ~50 ms reducer.
- Documented caveat: start external-reducer writers staggered. Do not rely on
  four simultaneous cold starts staying under 2 s. (Staggering was not measured.)
- Not qualified: saturating a store with external-reducer writers; reducers slower
  than the worker timeout sharing a store with latency-sensitive writers; more
  than 4 writers; Linux x64; verifying very long external-reducer aggregates as a
  routine operation.
- If independent workloads need isolation from a slow or hung reducer, give them
  separate stores. A shared joint invariant must stay in one store; do not shard
  it.

What would reopen the decision: a consumer that needs saturated external-reducer
writers, or measured Linux numbers that miss the budget. Revisit after protocol or
reducer-path changes.
