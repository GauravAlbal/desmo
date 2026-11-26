# Assurance & Verification Boundaries

Desmo's core architectural property is **shared transition semantics**: the same `.ik.toml` definition checked by `ik check` is executed at runtime by `ik`. This document states plainly what is mechanically tested, what is bounded-checked, what is formally proved, and what is **not** proved.

## Summary

| Layer | Status | What it establishes | Explicit limits |
|---|---|---|---|
| **Shared transition semantics** | **Implemented & tested** | Declarative `.ik.toml` guards, invariants, and state transitions share one expression AST and evaluator across runtime execution, `ik check`, `ik scenario`, and Hegel. | Does not prevent host code from bypassing `ik` or writing directly to an external DB/API. |
| **Core test suites (`scripts/public-verify`)** | **Passing on macOS arm64 & Linux x64** | 700+ Rust tests, 160 Python tests, 83 TypeScript tests, 44 Go tests covering capability currentness, illegal transitions, invariants, operation attribution, replay, regions, codegen, simulator, and trace validation. | Finite test coverage; not a universal proof. |
| **Deleted-wall mutation suite (`validation/ik06/`)** | **15/15 kernel mutants killed** | Deliberately deleting each of 15 constitutional kernel walls (`AcceptStaleCapability`, `SkipGenerationIncrement`, `SkipNonceRotation`, `AllowTerminalMutation`, `SkipGenesisValidation`, `SkipInvariantValidation`, `PersistOnRefusal`, `CommitPartialTransaction`, `SkipOperationBinding`, `SkipDuplicateGate`, `OmitCapabilityFromFingerprint`, `RerunLiveGuardsOnReplay`, `AllowSecondPendingEffect`, `AllowEffectConflict`, `MutateOnDuplicate`) fails tests with strict seam-firing enforcement. | Covers kernel constitutional laws in Rust. |
| **SDK transport mutation suite (`validation/sdk_mutants/`)** | **6/6 transport mutants killed** | Deletes critical transport walls in TypeScript (`runtime_version` handshake, `runEffect` duplicate-plan guard), Python (`_uncertain` operation tracking, `run_effect` duplicate-plan guard), and Go (`json.Decoder.UseNumber` `int64` precision, `RunEffect` duplicate-plan guard) in an isolated temp workspace and verifies language SDK suites catch each regression. | Targets the 6 highest-risk SDK transport safety invariants rather than mutating every helper. |
| **Whole-store integrity & CI guard (`ik store verify`, `ik guard`)** | **Read-only audit & CI check** | `ik store verify` runs `PRAGMA quick_check(1)` for SQLite B-tree integrity plus `meta` schema checks, zero orphan rows in `receipts`/`journal`/`operations`, full hash chains, deterministic receipts, materialized `aggregates` projection parity, and optional `--machine` semantic replay without mutating file bytes or mtime. `ik guard` scans TypeScript/Python/Go repos for direct bypass writes against protected status tables. | `ik guard` checks configured status-table write patterns in source paths rather than proving whole-program dataflow. |
| **Real process-crash matrix (`tests/crash_matrix.rs`)** | **32+ crash-point tests passing** | Kills real child processes via `SIGABRT` and `exit(86)` across SQLite persistence boundaries (`Genesis`, `Admit`, `Command`, `Record`, plus `after_operation` attribution boundaries) and verifies atomic recovery and replay. | Trusts SQLite WAL/transactions, `rusqlite`, and the local OS/filesystem. |
| **Concurrent linearizability (`tests/linearizability.rs`)** | **Passing (40 seeds × 3 workers)** | Deterministic multi-worker SQLite command/record/show/reopen histories checked against a sequential reference model via interval-respecting DFS. | Finite schedule sampling, not universal linearizability or crash-concurrent proof. |
| **Dual-solver SMT feasibility (`tests/smt_*.rs`)** | **Optional adapter (Z3 + cvc5)** | `CheckedInvariant` discharges require both Z3 and cvc5 agreement; 26 known-solution cases (17 feasible / 9 impossible) plus scalar overflow tag equivalence over all `i64`. | `text_in` and unbounded string ordering stay `unknown`; single-solver `SolverAsserted` never discharges. |
| **Model checking & property sampling (TLC / Hegel)** | **Optional adapters** | Pinned TLC 2.19 checks exported finite state graphs for dead ends; Hegel runs sampled execution over generated inputs. | TLC checks the exported graph (not guards/facts); Hegel is sampled execution, never proof. |
| **Kani bounded proofs (`kani` CI job, `ubuntu-24.04`)** | **Verified in CI (Kani 0.68 / CBMC 6.11)** | Five CI-gated Kani proofs: production payload-opaque authority gates (`prove_command_authority_end_to_end`, `prove_outcome_binding_authority_end_to_end`), checked `i64` arithmetic non-wrap (`prove_checked_i64_arith_no_silent_wrap`), ordered-guard error non-fallthrough (`prove_ordered_guard_error_never_falls_through`), and fixed-width 3-region terminal/joint atomicity (`prove_region_lowering_terminal_and_joint_invariants`), alongside bounded ASCII nonce `validate_capability` proofs. | Concrete `serde_json::Value` / `Sha256` serialization, SQLite WAL, OS/filesystem, and unrestricted 64-bit general multiplier synthesis remain outside Kani's boundary. |

## What Desmo does NOT guarantee

- **Not "formally verified" end-to-end.** `ik check` is bounded, corpus-relative analysis. Saying "ACCEPT" means all obligations were witnessed within the supplied corpus and bounds — not a universal proof over all inputs.
- **Not cross-aggregate atomic invariants or unbounded inductive proof.** Transactions enforce invariants on one `aggregate_id` at a time (cross-aggregate invariants such as balance conservation across two accounts are outside scope). `ik check` runs bounded corpus exploration and optional SMT obligation unreachability rather than k-induction or guarded deadlock-freedom proofs; however, because the runtime evaluates `[invariants]` on every commit, an unexplored invariant violation refuses loudly (`IK_INVARIANT_FAILED`) instead of persisting illegal state.
- **Not a host sandbox or distributed coordinator.** Qualified deployment is a single worker or agent harness backed by **persistent local SQLite**. Multi-replica shared SQLite, read-only serverless filesystems, and untrusted external reducer processes are out of scope.
- **Not cryptographic caller authentication.** Capability tokens `(generation, nonce)` are deterministic rotation digests (`derive_nonce` in `src/runtime.rs`) built for staleness and optimistic-concurrency fencing across cooperative workers, not secrets against a hostile caller forging envelopes.
- **Not exactly-once external side effects.** External effects (payments, emails, webhooks) are **at-least-once** with kernel-bound effect identity and idempotent outcome recording (`record`); the external sink must honor the idempotency key.
- **Not a guarantee that your business rule is the right business rule.** Desmo enforces the `.ik.toml` rules you declare; whether those rules match your product intent remains a human decision (surfaced explicitly in `ik handoff`).

## Durability & measured performance

- **SQLite durability defaults:** `Store` enables `PRAGMA journal_mode=WAL` and defaults to `PRAGMA synchronous=FULL` (`IK_SQLITE_SYNCHRONOUS=normal` is an explicit opt-in for workloads that accept WAL `NORMAL` semantics). Note that on macOS APFS, standard `fsync` under `synchronous=FULL` acts as a normal write barrier unless `fullfsync` is enabled by the OS/drive.
- **Indexed queries:** `aggregates` maintains composite secondary indexes (`idx_aggregates_machine_id`, `idx_aggregates_machine_terminal_id`) so `ik list` and `machine.list(...)` avoid full table scans on machine/terminal filters.
- **Measured on Apple Silicon macOS (`cargo build --release --bin ik && python3 scripts/bench.py`):**
  - Persistent `ik --session` commits (`WAL + FULL`, 500 commits): **3,495.1 commits/s** (`p50=0.232ms`, `p99=1.132ms`).
  - Persistent `ik --session` commits (`WAL + NORMAL`, 500 commits): **1,575.6 commits/s** (`p50=0.488ms`, `p99=2.292ms`; single-run local variance on small 500-commit WAL batches).
  - Indexed `ik list --machine order --terminal false --limit 50 --json` at 5,000 aggregates (20 runs): **`p50=5.27ms`**, **`p99=13.88ms`** (including fresh CLI process spawn).

## Reproducing the public gate

```sh
sh scripts/public-verify
```

Needs only Rust, Python 3, Go, and Node. No private credentials or cloud services.
