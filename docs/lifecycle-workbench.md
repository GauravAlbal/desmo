# Lifecycle workbench: reuse map, interfaces, compatibility, acceptance (IK-X00)

Status: design record for an additive tooling track. Nothing described under "Delta" is implemented by this
document. Source reconciled against the repository at the commit that adds this file.

## 1. What already exists (reuse, do not rebuild)

| Capability | Where | Notes |
|---|---|---|
| `ik check` pipeline: parse, load corpus/baseline, explore, discharge, optional `--prior` diff, report, release evidence | `src/cli.rs`, `src/declarative/{check,region,explore,analyze,diff,plan,evidence,report,release}.rs` | Flags today: `--prior --corpus --baseline --release --json`. Bounds are fixed in the CLI (`Bounds{4096,16}`). |
| Exploration with witnesses and completeness | `src/declarative/explore.rs` (`CaseCorpus`, `WitnessBook`, `complete`/`incomplete_reason`) | Source of "witness found / no witness in this search" and of truncation status. |
| Structural graph | `src/declarative/analyze.rs` (`machine_graph`, `StructuralAnalysis`) | There is no separate `graph` module. |
| Explanation | `src/declarative/explain.rs` (`explain`) | Library function only. There is no `ik explain` command; use `ik check --json` (witness and `diagnostic`) for a packed readout. |
| Verification plan, coordinates, discharges | `plan.rs` (`VerificationPlanV1`, `SemanticCoordinate`), `evidence.rs` | Property identity for goals. |
| Counterexample envelope and replay classification | `counterexample.rs` (`CounterexampleV1`, `classify_replay`) | Strings steps and an opaque reproduce payload. |
| Trace | `typescript/src/trace.ts` (`ikaros.trace.v2`) | TypeScript parser/renderer. |
| Hegel | `Cargo.toml` (`hegeltest =0.30.0`, dev-dependency); `tests/hegel_qual.rs`, `hegel_kernel.rs`, `hegel_props.rs`, `vfy_adopt.rs` | Kernel-level `Store` properties on a hardcoded machine; no generic machine driver. |
| Store/CLI/clients for a scenario driver | `src/store.rs`, `src/protocol.rs`, `src/bin/ik.rs`, `python/`, `typescript/`, `go/` | Create, command, operation ids, `lookup_operation`, reopen, effect record, receipts. |
| Existing scenario example | `examples/publish-approval/scenarios.py` | Exercises the packed binary through the Python client. |
| Fault seams | `src/law_mutant.rs`, crash hook in `src/store.rs` | `test-support` only; the packaged `ik` must not contain them (`tests/crash_surface_contract.rs`). |
| External process code | `src/external.rs` | Python reducer worker (stdio). Not an analysis adapter; do not reuse for adapters. |

Missing at the time of that inspection: check profiles, explicit useful-path goals, stuck-state classification, an external analysis-adapter runner, a Hegel executable adapter, any TLC integration, `desmo doctor`. `ik graph export` and the TLC graph adapter have since been added. The adapter starter, `ik analysis conformance`, and a prepared unqualified P route have since been added; see `docs/analysis-adapters.md`. Hegel executable testing and `desmo doctor` are still absent.

## 2. Journey to interface map

| Journey | Entry | One semantic implementation | Delta |
|---|---|---|---|
| J0 arrive | `desmo init`, `desmo doctor` (new) | existing init + a doctor that reads manifests only | scaffold gains a profile and two scenarios |
| J1 useful work | `ik check --profile P [--plan]` | native `check_with_run` + profile goals | profile loader, goal evaluation, plan preview |
| J2 adverse ordering | `ik scenario run S --out D` | real `Store` in a fresh temp store | scenario file format, deterministic driver, synthetic executor |
| J3 explain/reproduce | `ik check --reproduce BUNDLE`, `desmo trace` | trace v2, `CounterexampleV1` | bundle format, honest reproduction status |
| J4 reconcile | existing operation lookup | unchanged | none; analysis never authorizes a commit |
| J5 change | `ik check --prior`, profile diff, scenario `prior_machine` | existing diff + profile identity + runtime `present` observation | `--prior` text names version, added/removed commands and states, and that existing instances keep the version they were created under; a continuity scenario records which named commands still work and which new commands are refused; evidence invalidation by content digest |
| J6 escalate | `ik check --backend NAME` | adapter runner (X03) | request/result contract |
| J7 new adapter | manifest + executable | adapter runner | starter and conformance suite |

## 3. Compatibility rules (binding for this track)

1. Protocol 1.0, store schema 4, `CORE_LAWS`, and the persisted machine `spec_hash` are unchanged.
2. `ik check` without new flags behaves exactly as today (text, `--json` shape, exit codes 0 accept / 1 refuse / 2 usage).
   New JSON members are additive; new output is behind new flags or new top-level sections that old readers ignore.
3. New evidence is a versioned envelope (`ik.analysis.result.v1`), not new fields under strict existing schemas.
4. The runtime never calls an adapter or an analysis result during a commit.
5. The packaged `ik` gains no crash-arming interface. Scenario faults are expressed through public operations
   (duplicate request, stale handle, dropped reply modeled by discarding the response, reopen, delayed or
   conflicting synthetic effect outcome).
6. Profiles have their own digest. A goal id is never sufficient for cache reuse: evidence binds goal content,
   machine semantics, runtime artifact, scenario, mapping, backend version and limits.
7. Missing tools, timeouts, unsupported semantics, stale or incomplete evidence are explicit outcomes.

## 4. Acceptance list (executable, per package)

| Package | Acceptance |
|---|---|
| X01 | always-refuse machine fails a required useful-path goal while safety holds; a terminal state with no outgoing edge is not reported as a deadlock; missing corpus cases are reported as an input-coverage gap, not unreachability; a truncated search keeps its incomplete status and a strict required goal does not pass; `--plan` states scope without running; baseline `ik check` output is byte-identical |
| X02 | scenario run reproducible from a bundle in a second process; refuses a production store path; records which fault was applied; no real external effect |
| X03 | runner rejects: stale hashes, exit 0 without a valid result, malformed/truncated output, output path traversal and symlinks, wrong request/subject identity; timeouts and kills reap the process tree and mark incomplete |
| X04 | Hegel adapter: real pass, real shrunk failure against a deliberately broken driver, replay from the artifact |
| X05 | TLC adapter on a runtime-derived finite graph: a passing property, a violated property with mapped reproduction, one unsupported and one incomplete case |
| X06 | one report for all evidence kinds with a next action; regression promotion is explicit; `ik handoff` leads with `Desmo <kind>: <subject>` and a plain first block (change: declared delta before exploration/observation), with cited ids under `Exact evidence`; the first block also embeds a facts strip and monospace route map and ends on the action/decision line; the command table follows; mermaid is `ik diagram --format mermaid` only |
| X07 | profile/goal/corpus/fairness change invalidates evidence; old-version evidence cannot satisfy a new version |
| X08 | an adapter added from the starter without kernel edits; P marked prepared unless a real run exists |
| X09 | fresh agents use installed bytes; results are labeled usability evidence, not human review or IK-14 |

## 5. Decisions recorded

- Native `ik check` remains the entry point and default engine; no Java, .NET, solver or network is needed by default.
- File-based subprocess contract only: manifest, `describe`, `run --request R --out D`. No daemon, registry or ABI.
- Hegel is the first executable backend; one TLC route (graph export) is the model demonstration. No second compiler.
- Subject for models starts as a runtime-derived finite graph with explicit frontier and restrictions; no universal
  `.ik.toml` translator; opaque Python/native reducers get no symbolic guarantees from schema metadata.
