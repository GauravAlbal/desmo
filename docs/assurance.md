# Assurance & Verification Boundaries

Desmo's core architectural property is **shared transition semantics**: the same `.ik.toml` definition checked by `ik check` is executed at runtime by `ik`. This document states plainly what is mechanically tested, what is bounded-checked, what is formally proved, and what is **not** proved.

## Summary

| Layer | Status | What it establishes | Explicit limits |
|---|---|---|---|
| **Shared transition semantics** | **Implemented & tested** | Declarative `.ik.toml` guards, invariants, and state transitions share one expression AST and evaluator across runtime execution, `ik check`, `ik scenario`, and Hegel. | Does not prevent host code from bypassing `ik` or writing directly to an external DB/API. |
| **Core test suites (`scripts/public-verify`)** | **Passing on macOS arm64 & Linux x64** | 700+ Rust tests, 160 Python tests, 83 TypeScript tests, 44 Go tests covering capability currentness, illegal transitions, invariants, operation attribution, replay, regions, codegen, simulator, and trace validation. | Finite test coverage; not a universal proof. |
| **Deleted-wall mutation suite (`validation/ik06/`)** | **15/15 kernel mutants killed** | Deliberately deleting each of 15 constitutional kernel walls (`AcceptStaleCapability`, `SkipGenerationIncrement`, `SkipNonceRotation`, `AllowTerminalMutation`, `SkipGenesisValidation`, `SkipInvariantValidation`, `PersistOnRefusal`, `CommitPartialTransaction`, `SkipOperationBinding`, `SkipDuplicateGate`, `OmitCapabilityFromFingerprint`, `RerunLiveGuardsOnReplay`, `AllowSecondPendingEffect`, `AllowEffectConflict`, `MutateOnDuplicate`) fails tests with strict seam-firing enforcement. | Covers kernel constitutional laws in Rust. |
| **SDK transport mutation suite (`validation/sdk_mutants/`)** | **6/6 transport mutants killed** | Deletes critical transport walls in TypeScript (`runtime_version` handshake, `runEffect` duplicate-plan guard), Python (`_uncertain` operation tracking, `run_effect` duplicate-plan guard), and Go (`json.Decoder.UseNumber` `int64` precision, `RunEffect` duplicate-plan guard) in an isolated temp workspace and verifies language SDK suites catch each regression. | Targets the 6 highest-risk SDK transport safety invariants rather than mutating every helper. |
| **Whole-store integrity & CI guard (`ik store verify`, `ik guard`)** | **Read-only audit & CI check** | `ik store verify` validates `meta` schema, zero orphan rows in `receipts`/`journal`/`operations`, full hash chains, deterministic receipts, materialized `aggregates` projection parity, and optional `--machine` semantic replay without mutating file bytes or mtime. `ik guard` scans TypeScript/Python/Go repos for direct bypass writes against protected status tables. | `ik guard` checks configured status-table write patterns in source paths rather than proving whole-program dataflow. |
| **Real process-crash matrix (`tests/crash_matrix.rs`)** | **32+ crash-point tests passing** | Kills real child processes via `SIGABRT` and `exit(86)` across SQLite persistence boundaries (`Genesis`, `Admit`, `Command`, `Record`, plus `after_operation` attribution boundaries) and verifies atomic recovery and replay. | Trusts SQLite WAL/transactions, `rusqlite`, and the local OS/filesystem. |
| **Concurrent linearizability (`tests/linearizability.rs`)** | **Passing (40 seeds × 3 workers)** | Deterministic multi-worker SQLite command/record/show/reopen histories checked against a sequential reference model via interval-respecting DFS. | Finite schedule sampling, not universal linearizability or crash-concurrent proof. |
| **Dual-solver SMT feasibility (`tests/smt_*.rs`)** | **Optional adapter (Z3 + cvc5)** | `CheckedInvariant` discharges require both Z3 and cvc5 agreement; 26 known-solution cases (17 feasible / 9 impossible) plus scalar overflow tag equivalence over all `i64`. | `text_in` and unbounded string ordering stay `unknown`; single-solver `SolverAsserted` never discharges. |
| **Model checking & property sampling (TLC / Hegel)** | **Optional adapters** | Pinned TLC 2.19 checks exported finite state graphs for dead ends; Hegel runs sampled execution over generated inputs. | TLC checks the exported graph (not guards/facts); Hegel is sampled execution, never proof. |
| **Kani bounded proofs (`kani` CI job, `ubuntu-24.04`)** | **Verified in CI (Kani 0.68 / CBMC 6.11)** | Five CI-gated Kani proofs: production payload-opaque authority gates (`prove_command_authority_end_to_end`, `prove_outcome_binding_authority_end_to_end`), checked `i64` arithmetic non-wrap (`prove_checked_i64_arith_no_silent_wrap`), ordered-guard error non-fallthrough (`prove_ordered_guard_error_never_falls_through`), and fixed-width 3-region terminal/joint atomicity (`prove_region_lowering_terminal_and_joint_invariants`), alongside bounded ASCII nonce `validate_capability` proofs. | Concrete `serde_json::Value` / `Sha256` serialization, SQLite WAL, OS/filesystem, and unrestricted 64-bit general multiplier synthesis remain outside Kani's boundary. |

## What Desmo does NOT guarantee

- **Not "formally verified" end-to-end.** `ik check` is bounded, corpus-relative analysis. Saying "ACCEPT" means all obligations were witnessed within the supplied corpus and bounds — not a universal proof over all inputs.
- **Not a host sandbox or distributed coordinator.** Qualified deployment is a single worker or agent harness backed by **persistent local SQLite**. Multi-replica shared SQLite, read-only serverless filesystems, and untrusted external reducer processes are out of scope.
- **Not exactly-once external side effects.** External effects (payments, emails, webhooks) are **at-least-once** with kernel-bound effect identity and idempotent outcome recording (`record`); the external sink must honor the idempotency key.
- **Not a guarantee that your business rule is the right business rule.** Desmo enforces the `.ik.toml` rules you declare; whether those rules match your product intent remains a human decision (surfaced explicitly in `ik handoff`).

## Reproducing the public gate

```sh
sh scripts/public-verify
```

Needs only Rust, Python 3, Go, and Node. No private credentials or cloud services.
