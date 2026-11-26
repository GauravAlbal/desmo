# Assurance & Verification Boundaries

Desmo's core architectural property is **shared transition semantics**: the same `.ik.toml` definition checked by `ik check` is executed at runtime by `ik`. This document states plainly what is mechanically tested, what is bounded-checked, what is formally proved, and what is **not** proved.

## Summary

| Layer | Status | What it establishes | Explicit limits |
|---|---|---|---|
| **Shared transition semantics** | **Implemented & tested** | Declarative `.ik.toml` guards, invariants, and state transitions share one expression AST and evaluator across runtime execution, `ik check`, `ik scenario`, and Hegel. | Does not prevent host code from bypassing `ik` or writing directly to an external DB/API. |
| **Core test suites (`scripts/public-verify`)** | **Passing on macOS arm64 & Linux x64** | 700+ Rust tests, 160 Python tests, 83 TypeScript tests, 44 Go tests covering capability currentness, illegal transitions, invariants, operation attribution, replay, regions, codegen, simulator, and trace validation. | Finite test coverage; not a universal proof. |
| **Deleted-wall mutation suite (`validation/ik06/`)** | **15/15 kernel mutants killed** | Deliberately deleting each of 15 constitutional kernel walls (`AcceptStaleCapability`, `SkipGenerationIncrement`, `SkipNonceRotation`, `AllowTerminalMutation`, `SkipGenesisValidation`, `SkipInvariantValidation`, `PersistOnRefusal`, `CommitPartialTransaction`, `SkipOperationBinding`, `SkipDuplicateGate`, `OmitCapabilityFromFingerprint`, `RerunLiveGuardsOnReplay`, `AllowSecondPendingEffect`, `AllowEffectConflict`, `MutateOnDuplicate`) fails tests with strict seam-firing enforcement. | Client transport layers (TS/Python/Go) do not yet have deleted-wall mutation controls. |
| **Real process-crash matrix (`tests/crash_matrix.rs`)** | **32+ crash-point tests passing** | Kills real child processes via `SIGABRT` and `exit(86)` across SQLite persistence boundaries (`Genesis`, `Admit`, `Command`, `Record`, plus `after_operation` attribution boundaries) and verifies atomic recovery and replay. | Trusts SQLite WAL/transactions, `rusqlite`, and the local OS/filesystem. |
| **Concurrent linearizability (`tests/linearizability.rs`)** | **Passing (40 seeds × 3 workers)** | Deterministic multi-worker SQLite command/record/show/reopen histories checked against a sequential reference model via interval-respecting DFS. | Finite schedule sampling, not universal linearizability or crash-concurrent proof. |
| **Dual-solver SMT feasibility (`tests/smt_*.rs`)** | **Optional adapter (Z3 + cvc5)** | `CheckedInvariant` discharges require both Z3 and cvc5 agreement; 26 known-solution cases (17 feasible / 9 impossible) plus scalar overflow tag equivalence over all `i64`. | `text_in` and unbounded string ordering stay `unknown`; single-solver `SolverAsserted` never discharges. |
| **Model checking & property sampling (TLC / Hegel)** | **Optional adapters** | Pinned TLC 2.19 checks exported finite state graphs for dead ends; Hegel runs sampled execution over generated inputs. | TLC checks the exported graph (not guards/facts); Hegel is sampled execution, never proof. |
| **Kani bounded proofs (`src/runtime/authority.rs`)** | **Verified (Kani 0.68 / CBMC 6.11)** | Exhaustive Kani proofs cover the **production payload-opaque authority gates** (`authorize_command` and `authorize_outcome_binding`, invoked directly by `execute_command` and `execute_outcome_binding` with zero `#[cfg(kani)]` cutpoints) across arbitrary `u64` generations/sequences, all effect states, terminal locks, G-002 exact-duplicate precedence, single-pending-effect lock, effect conflict/foreign refusal, and `u64::MAX` counter-ceiling non-wrap (`0/125` failed, `8/8` cover properties satisfied per harness in `<1s`), alongside the bounded ASCII nonce `validate_capability` proof. | Concrete `serde_json::Value` / `Sha256` serialization, SQLite WAL, and OS/filesystem remain outside Kani's boundary; Kani runs as a dedicated proof gate rather than inside default `scripts/public-verify`. |

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
