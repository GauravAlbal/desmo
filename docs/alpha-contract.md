# Desmo scoped alpha contract (proposed, IK-00)

Status: **proposed**. This file is a claim ledger, not a release certification.
Planning baseline: `f7ea1437279d7274920b01c26794c3ffd04a546f`. Published
manifest: `LATEST_RELEASE` = v0.4.1 at `5b3d0516123386e1041bf51648a6de6dba5fd0b4`.

Implemented, source-accepted, executed, and artifact-qualified are separate
states. Only the baseline report (`validation/ik00/baseline.json`) records what
was actually executed.

## Supported slice

| Axis | Alpha claim |
|---|---|
| Deployment | Single worker or agent harness with **persistent local SQLite**. Lifecycle facts and effect intents live inside one authority boundary. |
| Machine modes | Declarative schema 1/2 (`.ik.toml`), external Python reducer with declared laws, native Rust. |
| Clients | TypeScript and Python entry points; Go compatibility retained. |
| Proposed artifact targets | macOS ARM64, Linux x64. **Only targets actually qualified may be advertised.** v0.4.1 ships darwin-arm64 only. |
| Authority | Rust runtime. Hosts route lifecycle mutations through it. |

## Behaviors claimed (each needs evidence at release)

| Claim | Gate / evidence owner |
|---|---|
| Stale or copied capability is refused (`IK_CAPABILITY_STALE`) | `scripts/accept` (Rust, Python, Go, TypeScript) |
| Declared invariants refuse violating transitions; refusals do not commit | Rust tests, `tests/vfy_*.rs` |
| Atomic journal/projection/receipt commit; crash prefix replayable | `tests/crash_matrix.rs`, store tests |
| Exact command/source exploration evidence; `ik check` is bounded, corpus-relative | `ik check`, `docs/ik-check.md` |
| Law inventory is advisory; `.ik.toml` + `spec_hash` are authoritative | `ik laws`, `tests/vfy_laws.rs` |
| Old aggregates keep their machine version; new creates may use a new version | `python/tests/test_machine_evolution.py` |
| Lossless integers and envelope validation in the Node client | `typescript/src/*.test.ts` |
| Initial state violating a declared hard law is refused before any row or capability (declarative and external machines; native only with `validate_initial`) | `tests/genesis_validity.rs`, `python/tests/test_reducer.py` |
| Kernel-side (Rust/CLI) caller `operation_id` attribution is committed atomically with the mutation; an exact repeat returns history, a changed repeat conflicts, refusals/no-ops leave no row; `lookup_operation` reports committed-exact, conflict, or not-recorded | `tests/operation_attribution.rs` |
| Python, TypeScript and Go clients dispatch each mutation once under an operation id (generated if omitted, exposed on every error), distinguish not-sent from submission-unknown, return history distinctly from a new handle, never retry or refresh; Go exposes the narrow `CommandWithOperation`/`LookupOperation` path | `python/tests/test_operation_recovery.py`, `typescript/src/operation.test.ts`, `go/operation_test.go`, `docs/client-recovery.md` |
| `ik check` keeps obligation status (satisfied / falsified / incomplete / not applicable / not examined / approved debt / uncovered) apart from a strict release policy that requires a complete exploration and executed, unwitnessed-free standard-lane evidence; non-standard obligations are never counted satisfied; no artifact digest is fabricated; `ik laws` links invariants to obligations | `tests/release_evidence.rs`, `docs/ik-check.md` |
| Trace exports (`ikaros.trace.v2`) are tagged and validated: a read, refusal, no-op, unknown outcome, exact-duplicate response and bounded witness are distinguishable and never rendered as commits; redacted/replayed exports cannot claim to be original runs | `typescript/src/trace.test.ts` |
| Write contention on one store is measured and bounded: reducer evaluation holds the store-wide write transaction; the measured, macOS-arm64-only envelope and the narrowed external-reducer scope are in `docs/execution-envelope.md` | `validation/ik07/report.json`, `validation/ik07/native.json`, `docs/execution-envelope.md` |
| `ik schema` exports deterministic, Rust-derived machine metadata; `desmo codegen` generates TypeScript command/fact/successor contracts from it (conditionally absent facts stay optional, multi-outcome commands stay unions), `--check` detects a stale projection, and the generated identity is checked at runtime. Types are not ownership: aliased handles and casts are still refused by the kernel | `tests/machine_schema.rs`, `typescript/src/codegen.test.ts` |
| One exact-artifact approval app: an effect is bound to the approved digest and an executor reading mutable latest bytes is stopped before any write; duplicate outcome is a no-op, a conflicting outcome is refused; executor death is recovered **at-least-once** (not exactly-once) against an idempotent sink; a lost reply is resolved by `lookup_operation`; a projection is repaired from canonical state; a router keyed on persisted identity keeps v1/v2/v3 aggregates on their own definitions through rollback (a default-version router is refused with `IK_MACHINE_MISMATCH`). A defective transition is refused at runtime and by a bounded `ik check` witness; a flat machine violating its invariant is no longer silently accepted | `python/tests/test_publish_approval_app.py`, `tests/flat_invariant_witness.rs`, `examples/publish-approval/scenarios.py` |
| Each supported claim maps to enforcement and executed evidence, and each deleted-wall mutant (15, including genesis validation and three operation-attribution walls) is killed with its seam proven reached; a mutant that never fired, a missing/ignored test, or a removed evidence row fails. Independent review was done by a separate agent (no human review); client transport has no deleted-wall control | `validation/ik06/claims.json`, `validation/ik06/results.json`, `python/tests/test_qualification_manifest.py` |
| A public-source candidate is an allowlisted verbatim export of one commit with an origin-to-export manifest; private-workflow references and absolute paths block it unless acknowledged with a reviewable reason, and secrets can never be acknowledged. The owner confirmed disclosure and licensing authority, the package names, support terms and a snapshot-only history; the current candidate carries no acknowledged references | `python/tests/test_public_export.py`, `docs/public-release-checklist.md` |

## Not claimed

Valid genesis for native machines that do not override `validate_initial`,
Linux x64 contention numbers, saturated or >4-writer external-reducer stores,
packed npm/Linux artifacts (IK-11), exactly-once
external effects, Postgres/MySQL, distributed coordination,
browser/edge/serverless, hostile-process sandboxing, general historical-state
migration. Also not claimed: consumer acceptance. The first internal consumer's lifecycle is
qualified in scratch and on disposable stores only; its live pin is unmoved and no live host
run is claimed. Independent review of the qualification manifest was done by a separate agent,
and the cold run was done by an agent with honor-system isolation; no human review or human
cold run is claimed.

## Hosting / topology

Qualified: Node/Python/Go **workers with persistent local storage**. Not
qualified: read-only serverless filesystems, multiple independent replicas
sharing one SQLite file, browsers.

## Consumer (identification only)

A first internal consumer has a native Go lifecycle through the Go client; its
active runtime pin is `desmo v0.1.1` at `79c70a4`. **No pin moves in IK-00.** Its host-side
routing change is qualified on disposable stores only (see `docs/alpha-readiness.md`, IK-10).

## Evidence rules

A named blocker is not a reason to weaken a gate. Missing platform, skipped
required test, or uncertain result is recorded as a blocker.
